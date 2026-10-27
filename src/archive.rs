use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{bail, Context, Result};
use base64::prelude::*;
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::load_global_config;
use crate::crypto::CryptoManager;
use crate::util::atomic_write;
use crate::vault::Store;

/// 备份文件魔数 (7 字节)
pub const ARCHIVE_MAGIC: &[u8; 7] = b"AGSEXP1";
/// 备份文件协议版本
pub const ARCHIVE_VERSION: u8 = 0x01;
/// 随机 Salt 长度 (16 字节)
pub const SALT_LEN: usize = 16;
/// 随机 Nonce 长度 (12 字节)
pub const NONCE_LEN: usize = 12;
/// PBKDF2 迭代轮数
pub const KDF_ITERATIONS: u32 = 100_000;
/// 二进制文件头总长度 (7 + 1 + 16 + 12 = 36 字节)
pub const HEADER_LEN: usize = 7 + 1 + SALT_LEN + NONCE_LEN;

/// 跨应用多账号加密备份包数据结构
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportArchive {
    pub format_version: u32, // 1
    pub exported_at: String, // RFC3339
    pub cli_version: String,
    pub apps: HashMap<String, AppArchiveData>,
}

impl ExportArchive {
    pub fn new() -> Self {
        Self {
            format_version: 1,
            exported_at: chrono::Utc::now().to_rfc3339(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            apps: HashMap::new(),
        }
    }
}

impl Default for ExportArchive {
    fn default() -> Self {
        Self::new()
    }
}

/// 单个应用的账号备份数据
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppArchiveData {
    pub registry: crate::vault::Registry,
    /// 账号 key -> Base64 编码的明文凭证字节
    pub accounts: HashMap<String, String>,
}

impl AppArchiveData {
    pub fn new(registry: crate::vault::Registry) -> Self {
        Self {
            registry,
            accounts: HashMap::new(),
        }
    }

    /// 插入原始明文凭证字节（自动进行 Base64 编码存储）
    pub fn insert_credential(&mut self, key: impl Into<String>, raw_bytes: &[u8]) {
        self.accounts
            .insert(key.into(), BASE64_STANDARD.encode(raw_bytes));
    }

    /// 获取并解码指定账号的明文凭证字节
    pub fn get_credential(&self, key: &str) -> Option<Result<Vec<u8>>> {
        self.accounts.get(key).map(|b64| {
            BASE64_STANDARD
                .decode(b64)
                .map_err(|e| anyhow::anyhow!("Base64 解码失败: {}", e))
        })
    }
}

/// 标准 PBKDF2-HMAC-SHA256 密钥派生实现（针对 32 字节输出优化）
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    assert!(iterations > 0, "iterations must be positive");

    // 计算 HMAC-SHA256 的密钥补齐
    let mut k_pad = [0u8; 64];
    if password.len() > 64 {
        let mut hasher = Sha256::new();
        hasher.update(password);
        let digest = hasher.finalize();
        k_pad[..32].copy_from_slice(&digest);
    } else {
        k_pad[..password.len()].copy_from_slice(password);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k_pad[i];
        opad[i] ^= k_pad[i];
    }

    // 预计算 inner 和 outer 的基础 hasher 状态，避免每轮迭代重复计算 64 字节 padding
    let mut inner_base = Sha256::new();
    inner_base.update(ipad);

    let mut outer_base = Sha256::new();
    outer_base.update(opad);

    let hmac = |data: &[u8]| -> [u8; 32] {
        let mut inner = inner_base.clone();
        inner.update(data);
        let inner_hash = inner.finalize();

        let mut outer = outer_base.clone();
        outer.update(inner_hash);
        let result = outer.finalize();

        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    };

    // U_1 = PRF(P, salt || INT_32_BE(1))
    let mut initial_data = Vec::with_capacity(salt.len() + 4);
    initial_data.extend_from_slice(salt);
    initial_data.extend_from_slice(&1u32.to_be_bytes());

    let mut u = hmac(&initial_data);
    let mut result = u;

    for _ in 1..iterations {
        u = hmac(&u);
        for j in 0..32 {
            result[j] ^= u[j];
        }
    }

    result
}

/// 基于密码与 16 字节随机 Salt 派生 32 字节 AES-256 密钥
pub fn derive_key(password: &str, salt: &[u8; SALT_LEN]) -> [u8; 32] {
    pbkdf2_hmac_sha256(password.as_bytes(), salt, KDF_ITERATIONS)
}

/// 将 `ExportArchive` 数据打包并使用密码进行 AES-256-GCM 加密，封装为 `AGSEXP1` 格式二进制
pub fn encrypt_archive(archive: &ExportArchive, password: &str) -> Result<Vec<u8>> {
    let json_bytes = serde_json::to_vec(archive)
        .map_err(|e| anyhow::anyhow!("归档数据序列化为 JSON 失败: {}", e))?;

    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("初始化 AES-256-GCM 加密器失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, json_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("AES-256-GCM 加密失败: {}", e))?;

    let mut output = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    output.extend_from_slice(ARCHIVE_MAGIC);
    output.push(ARCHIVE_VERSION);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// 校验 `AGSEXP1` 格式二进制文件，使用密码解密并解析出 `ExportArchive`
pub fn decrypt_archive(data: &[u8], password: &str) -> Result<ExportArchive> {
    if data.len() < HEADER_LEN {
        bail!("无效的备份文件：数据长度过短");
    }

    if &data[..7] != ARCHIVE_MAGIC {
        bail!("无效的备份文件：文件头魔数不匹配");
    }

    let version = data[7];
    if version != ARCHIVE_VERSION {
        bail!(
            "不支持的备份版本: 0x{:02x}（当前支持: 0x{:02x}）",
            version,
            ARCHIVE_VERSION
        );
    }

    // AES-GCM tag 长度为 16 字节，总密文长度至少应包含 tag
    if data.len() < HEADER_LEN + 16 {
        bail!("密码错误或备份文件已损坏");
    }

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&data[8..24]);

    let nonce_bytes = &data[24..36];
    let ciphertext = &data[36..];

    let key = derive_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| anyhow::anyhow!("初始化 AES-256-GCM 解密器失败: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let decrypted = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("密码错误或备份文件已损坏"))?;

    let archive: ExportArchive = serde_json::from_slice(&decrypted)
        .map_err(|e| anyhow::anyhow!("备份数据解析失败: {}", e))?;

    Ok(archive)
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub base_dir: PathBuf,
    pub output_path: PathBuf,
    pub password: String,
    pub target_agents: Vec<String>,
}

impl ExportOptions {
    pub fn new(
        base_dir: PathBuf,
        output_path: PathBuf,
        password: impl Into<String>,
        target_agents: Vec<String>,
    ) -> Self {
        Self {
            base_dir,
            output_path,
            password: password.into(),
            target_agents,
        }
    }

    pub fn key(&self) -> &str {
        &self.password
    }
}

#[derive(Debug, Clone)]
pub struct ExportReport {
    pub output_path: PathBuf,
    pub exported_agents: Vec<String>,
    pub agent_counts: std::collections::HashMap<String, usize>,
    pub total_accounts: usize,
    pub file_size: u64,
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub base_dir: PathBuf,
    pub input_path: PathBuf,
    pub password: String,
    pub target_agents: Option<Vec<String>>,
    pub clean_incompatible: bool,
    pub overwrite: bool,
}

impl ImportOptions {
    pub fn new(base_dir: PathBuf, input_path: PathBuf, password: impl Into<String>) -> Self {
        Self {
            base_dir,
            input_path,
            password: password.into(),
            target_agents: None,
            clean_incompatible: false,
            overwrite: true,
        }
    }

    pub fn key(&self) -> &str {
        &self.password
    }
}

#[derive(Debug, Clone)]
pub struct ImportReport {
    pub imported_agents: Vec<String>,
    pub agent_details: std::collections::HashMap<String, (usize, usize)>,
    pub total_accounts: usize,
    pub total_new: usize,
    pub total_merged: usize,
    pub backup_dirs: Vec<PathBuf>,
    pub backup_paths: Vec<PathBuf>,
    pub cleaned_agents: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CompatibilityStatus {
    Clean,
    Compatible,
    Incompatible(String),
}

/// 检查指定 Agent 本地已有数据的兼容性状态
pub fn check_agent_compatibility(
    agent_dir: &Path,
    crypto: &dyn crate::crypto::CryptoProvider,
) -> CompatibilityStatus {
    if !agent_dir.exists() {
        return CompatibilityStatus::Clean;
    }

    let store = Store::new(agent_dir.to_path_buf());
    if !store.registry_path().exists() {
        let accounts_dir = store.accounts_dir();
        if accounts_dir.exists() {
            if let Ok(entries) = fs::read_dir(&accounts_dir) {
                if entries.flatten().next().is_some() {
                    return CompatibilityStatus::Incompatible(
                        "存在凭据快照文件但缺少有效注册表 registry.json".to_string(),
                    );
                }
            }
        }
        return CompatibilityStatus::Clean;
    }

    let reg = match store.load_registry() {
        Ok(r) => r,
        Err(e) => return CompatibilityStatus::Incompatible(format!("注册表解析损坏: {}", e)),
    };

    if reg.accounts.is_empty() {
        return CompatibilityStatus::Clean;
    }

    for acc in &reg.accounts {
        let snap_path = store.snapshot_path(&acc.key);
        if snap_path.exists() {
            if let Err(e) = store.read_account(&acc.key, crypto) {
                return CompatibilityStatus::Incompatible(format!(
                    "账号 '{}' 凭据无法使用当前本地密钥或引擎解密: {}",
                    acc.key, e
                ));
            }
        } else {
            return CompatibilityStatus::Incompatible(format!(
                "账号 '{}' 登记在注册表中，但凭据快照文件缺失",
                acc.key
            ));
        }
    }

    CompatibilityStatus::Compatible
}

/// 安全备份并清理指定 Agent 目录下的不兼容旧数据
pub fn clean_and_reset_agent(agent_dir: &Path) -> Result<PathBuf> {
    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_dir = agent_dir
        .join("backups")
        .join(format!("incompatible_{}", ts));
    fs::create_dir_all(&backup_dir)?;

    let store = Store::new(agent_dir.to_path_buf());
    if store.registry_path().exists() {
        let _ = fs::copy(store.registry_path(), backup_dir.join("registry.json"));
        let _ = fs::remove_file(store.registry_path());
    }

    let accounts_dir = store.accounts_dir();
    if accounts_dir.exists() {
        let backup_accounts = backup_dir.join("accounts");
        let _ = fs::create_dir_all(&backup_accounts);
        if let Ok(entries) = fs::read_dir(&accounts_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Some(fname) = p.file_name() {
                        let _ = fs::copy(&p, backup_accounts.join(fname));
                    }
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }

    Ok(backup_dir)
}

/// 导出存储无关且高强度口令加密的便携迁移归档包
pub fn run_export(opts: &ExportOptions) -> Result<ExportReport> {
    if opts.password.trim().is_empty() {
        bail!("导出必须提供独立的加密口令，且不能为空");
    }

    let global_config = load_global_config(&opts.base_dir).unwrap_or_default();
    let crypto = CryptoManager::create_provider(global_config.crypto_provider.as_deref(), None);

    let mut archive = ExportArchive::new();
    let mut total_accounts = 0;
    let mut exported_agents = Vec::new();
    let mut agent_counts = std::collections::HashMap::new();

    for agent in &opts.target_agents {
        let (store, agent_dir) = if let Some(agent_app) = crate::agent::get_agent(agent) {
            let st = agent_app.default_store(&opts.base_dir);
            let dir = st.dir().to_path_buf();
            (st, dir)
        } else {
            let dir = opts.base_dir.join(agent);
            (Store::new(dir.clone()), dir)
        };

        if !agent_dir.exists() {
            continue;
        }

        let reg = match store.load_registry() {
            Ok(r) => r,
            Err(_) => continue,
        };

        if reg.accounts.is_empty() {
            continue;
        }

        let mut app_data = AppArchiveData::new(reg.clone());
        let mut count = 0;
        for acc in &reg.accounts {
            let raw_bytes = store.read_account(&acc.key, &*crypto).with_context(|| {
                format!("无法读取并解密账号 '{}' (agent: '{}')", acc.key, agent)
            })?;
            app_data.insert_credential(&acc.key, &raw_bytes);
            count += 1;
        }

        if !app_data.accounts.is_empty() {
            total_accounts += app_data.accounts.len();
            exported_agents.push(agent.clone());
            agent_counts.insert(agent.clone(), count);
            archive.apps.insert(agent.clone(), app_data);
        }
    }

    if total_accounts == 0 {
        bail!("未找到任何受管账号可供导出");
    }

    let encrypted_bytes = encrypt_archive(&archive, &opts.password)?;
    if let Some(parent) = opts.output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(&opts.output_path, &encrypted_bytes)?;

    let file_size = fs::metadata(&opts.output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(ExportReport {
        output_path: opts.output_path.clone(),
        exported_agents,
        agent_counts,
        total_accounts,
        file_size,
    })
}

/// 导入便携归档包，自动按当前目标机器的存储引擎重加密，并安全处理不兼容数据
pub fn run_import(opts: &ImportOptions) -> Result<ImportReport> {
    if opts.password.trim().is_empty() {
        bail!("导入必须提供独立的解密口令，且不能为空");
    }

    if !opts.input_path.exists() {
        bail!("导入文件不存在: {}", opts.input_path.display());
    }

    let encrypted_data = fs::read(&opts.input_path)
        .with_context(|| format!("无法读取备份文件: {}", opts.input_path.display()))?;

    let archive = decrypt_archive(&encrypted_data, &opts.password)?;

    let global_config = load_global_config(&opts.base_dir).unwrap_or_default();
    let local_crypto =
        CryptoManager::create_provider(global_config.crypto_provider.as_deref(), None);

    let ts = Utc::now().format("%Y%m%d_%H%M%S");
    let mut imported_agents = Vec::new();
    let mut agent_details = std::collections::HashMap::new();
    let mut total_accounts = 0;
    let mut total_new = 0;
    let mut total_merged = 0;
    let mut backup_dirs = Vec::new();
    let mut cleaned_agents = Vec::new();

    for (agent, app_data) in &archive.apps {
        if let Some(ref targets) = opts.target_agents {
            if !targets.contains(agent) {
                continue;
            }
        }

        let (store, agent_dir) = if let Some(agent_app) = crate::agent::get_agent(agent) {
            let st = agent_app.default_store(&opts.base_dir);
            let dir = st.dir().to_path_buf();
            (st, dir)
        } else {
            let dir = opts.base_dir.join(agent);
            (Store::new(dir.clone()), dir)
        };

        let compat = check_agent_compatibility(&agent_dir, &*local_crypto);
        match compat {
            CompatibilityStatus::Incompatible(reason) => {
                if opts.clean_incompatible {
                    let b = clean_and_reset_agent(&agent_dir)?;
                    backup_dirs.push(b);
                    cleaned_agents.push(agent.clone());
                } else {
                    bail!(
                        "检测到应用 [{}] 本地数据不兼容 (原因: {})。请确认是否清理重置后再导入（或使用 --clean 参数）",
                        agent,
                        reason
                    );
                }
            }
            CompatibilityStatus::Compatible => {
                let pre_backup = agent_dir.join("backups").join(format!("pre_import_{}", ts));
                let _ = fs::create_dir_all(&pre_backup);
                let store = Store::new(agent_dir.clone());
                if store.registry_path().exists() {
                    let _ = fs::copy(store.registry_path(), pre_backup.join("registry.json"));
                }
                let accounts_dir = store.accounts_dir();
                if accounts_dir.exists() {
                    let b_acc = pre_backup.join("accounts");
                    let _ = fs::create_dir_all(&b_acc);
                    if let Ok(entries) = fs::read_dir(&accounts_dir) {
                        for e in entries.flatten() {
                            let p = e.path();
                            if p.is_file() {
                                if let Some(n) = p.file_name() {
                                    let _ = fs::copy(&p, b_acc.join(n));
                                }
                            }
                        }
                    }
                }
                backup_dirs.push(pre_backup);
            }
            CompatibilityStatus::Clean => {}
        }

        fs::create_dir_all(&agent_dir)?;
        let mut registry = store.load_registry().unwrap_or_default();

        let mut app_new = 0;
        let mut app_merged = 0;

        for acc in &app_data.registry.accounts {
            let is_existing = registry
                .find_by_key_or_email(&acc.key, &acc.email)
                .is_some();
            if is_existing && !opts.overwrite {
                continue;
            }

            if let Some(cred_result) = app_data.get_credential(&acc.key) {
                let raw_bytes = cred_result?;
                store.write_account(&acc.key, &raw_bytes, &*local_crypto)?;
                let (is_update, _) = registry.add_or_update(acc.clone());
                if is_update {
                    app_merged += 1;
                } else {
                    app_new += 1;
                }
            }
        }

        if registry.active.is_none() {
            registry.active = app_data.registry.active.clone();
        }

        store.save_registry(&registry)?;

        let app_total = app_new + app_merged;
        if app_total > 0 {
            total_accounts += app_total;
            total_new += app_new;
            total_merged += app_merged;
            imported_agents.push(agent.clone());
            agent_details.insert(agent.clone(), (app_new, app_merged));
        }
    }

    let backup_paths = backup_dirs.clone();

    Ok(ImportReport {
        imported_agents,
        agent_details,
        total_accounts,
        total_new,
        total_merged,
        backup_dirs,
        backup_paths,
        cleaned_agents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_global_config;
    use crate::vault::{Account, Registry};
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn test_pbkdf2_sha256_vectors() {
        // 标准 PBKDF2-HMAC-SHA256 测试向量 (dkLen = 32)
        // Vector 1: c = 1
        let key1 = pbkdf2_hmac_sha256(b"password", b"salt", 1);
        assert_eq!(
            hex::encode(key1),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );

        // Vector 2: c = 2
        let key2 = pbkdf2_hmac_sha256(b"password", b"salt", 2);
        assert_eq!(
            hex::encode(key2),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );

        // Vector 3: c = 4096
        let key4096 = pbkdf2_hmac_sha256(b"password", b"salt", 4096);
        assert_eq!(
            hex::encode(key4096),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    fn sample_archive() -> ExportArchive {
        let mut archive = ExportArchive::new();

        let mut codex_registry = crate::vault::Registry::default();
        codex_registry.accounts.push(crate::vault::Account {
            key: "codex:user-1".to_string(),
            alias: Some("work".to_string()),
            email: "work@example.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        });
        codex_registry.active = Some("codex:user-1".to_string());

        let mut codex_data = AppArchiveData::new(codex_registry);
        codex_data.insert_credential("codex:user-1", b"{\"access_token\":\"secret-token-123\"}");

        archive.apps.insert("codex".to_string(), codex_data);
        archive
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let original = sample_archive();
        let password = "MySecurePassword123!@#";

        let encrypted = encrypt_archive(&original, password).expect("Encryption should succeed");

        // 校验文件头魔数与版本
        assert_eq!(&encrypted[..7], ARCHIVE_MAGIC);
        assert_eq!(encrypted[7], ARCHIVE_VERSION);
        assert!(encrypted.len() > HEADER_LEN);

        // 解密
        let decrypted = decrypt_archive(&encrypted, password).expect("Decryption should succeed");
        assert_eq!(decrypted.format_version, original.format_version);
        assert_eq!(decrypted.exported_at, original.exported_at);
        assert_eq!(decrypted.cli_version, original.cli_version);
        assert_eq!(decrypted.apps.len(), original.apps.len());

        let dec_app = decrypted.apps.get("codex").expect("codex app must exist");
        let orig_app = original.apps.get("codex").expect("codex app must exist");
        assert_eq!(dec_app.registry.accounts, orig_app.registry.accounts);
        assert_eq!(dec_app.registry.active, orig_app.registry.active);
        assert_eq!(dec_app.accounts, orig_app.accounts);

        // 校验凭证提取
        let codex_app = decrypted.apps.get("codex").expect("codex app must exist");
        let raw_cred = codex_app
            .get_credential("codex:user-1")
            .expect("credential should exist")
            .expect("credential should decode");
        assert_eq!(raw_cred, b"{\"access_token\":\"secret-token-123\"}");
    }

    #[test]
    fn test_decrypt_wrong_password() {
        let original = sample_archive();
        let encrypted = encrypt_archive(&original, "correct-password").unwrap();

        let err = decrypt_archive(&encrypted, "wrong-password").unwrap_err();
        assert!(
            err.to_string().contains("密码错误或备份文件已损坏"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_decrypt_tampered_data() {
        let original = sample_archive();
        let password = "test-password";
        let mut encrypted = encrypt_archive(&original, password).unwrap();

        // 篡改密文最后一个字节
        let last_idx = encrypted.len() - 1;
        encrypted[last_idx] ^= 0xff;

        let err = decrypt_archive(&encrypted, password).unwrap_err();
        assert!(
            err.to_string().contains("密码错误或备份文件已损坏"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_decrypt_invalid_magic() {
        let original = sample_archive();
        let password = "test-password";
        let mut encrypted = encrypt_archive(&original, password).unwrap();

        // 破坏魔数
        encrypted[0] = b'X';
        let err = decrypt_archive(&encrypted, password).unwrap_err();
        assert!(
            err.to_string().contains("文件头魔数不匹配"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_decrypt_unsupported_version() {
        let original = sample_archive();
        let password = "test-password";
        let mut encrypted = encrypt_archive(&original, password).unwrap();

        // 破坏版本字节
        encrypted[7] = 0x99;
        let err = decrypt_archive(&encrypted, password).unwrap_err();
        assert!(
            err.to_string().contains("不支持的备份版本"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_decrypt_truncated_data() {
        let short_data = b"AGSEXP1";
        let err = decrypt_archive(short_data, "pass").unwrap_err();
        assert!(
            err.to_string().contains("数据长度过短"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_export_import_roundtrip() {
        let dir = tempdir().unwrap();
        let src_base_dir = dir.path().join("src_machine");

        // 1. 准备待导出的源端账号：codex 与 agy 均包含虚拟账号
        let codex_dir = src_base_dir.join("codex");
        let codex_store = Store::new(codex_dir);
        let plain_crypto = CryptoManager::create_provider(Some("plain"), None);

        let codex_key = "codex:user-test";
        let codex_secret = b"{\"token\":\"my-secret-token-123\"}";
        codex_store
            .write_account(codex_key, codex_secret, &*plain_crypto)
            .unwrap();

        let mut codex_reg = codex_store.load_registry().unwrap();
        codex_reg.upsert(Account {
            key: codex_key.to_string(),
            alias: Some("myalias".to_string()),
            email: "test-codex@example.com".to_string(),
            plan: Some("pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        });
        codex_store.save_registry(&codex_reg).unwrap();

        // 准备 agy 账号
        let agy_dir = src_base_dir.join("agy");
        let agy_store = Store::new(agy_dir);
        let agy_key = "agy:user-test";
        let agy_secret = b"{\"token\":\"my-agy-secret-456\"}";
        agy_store
            .write_account(agy_key, agy_secret, &*plain_crypto)
            .unwrap();

        let mut agy_reg = agy_store.load_registry().unwrap();
        agy_reg.upsert(Account {
            key: agy_key.to_string(),
            alias: Some("agyalias".to_string()),
            email: "test-agy@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        agy_store.save_registry(&agy_reg).unwrap();

        // 2. 执行 run_export 导出加密归档
        let export_file = dir.path().join("backup.agx");
        let export_report = run_export(&ExportOptions {
            base_dir: src_base_dir.clone(),
            output_path: export_file.clone(),
            password: "strong-export-pass-123".to_string(),
            target_agents: vec!["codex".to_string(), "agy".to_string()],
        })
        .unwrap();

        assert_eq!(export_report.total_accounts, 2);
        assert_eq!(export_report.exported_agents.len(), 2);
        assert!(export_file.exists());
        assert!(export_report.file_size > 0);

        // 3. 在目标目录预先准备一个 codex 已有账号，以便验证预备份快照逻辑
        let dest_base_dir = dir.path().join("dest_machine");
        let dest_codex_dir = dest_base_dir.join("codex");
        let dest_codex_store = Store::new(dest_codex_dir.clone());
        let initial_secret = b"{\"token\":\"initial-old-token\"}";
        dest_codex_store
            .write_account("codex:old-user", initial_secret, &*plain_crypto)
            .unwrap();
        let mut initial_reg = dest_codex_store.load_registry().unwrap();
        initial_reg.upsert(Account {
            key: "codex:old-user".to_string(),
            alias: Some("old".to_string()),
            email: "old@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        dest_codex_store.save_registry(&initial_reg).unwrap();

        // 4. 验证错误密码导入失败
        let err = run_import(&ImportOptions {
            base_dir: dest_base_dir.clone(),
            input_path: export_file.clone(),
            password: "wrong-password".to_string(),
            target_agents: None,
            clean_incompatible: false,
            overwrite: true,
        });
        assert!(err.is_err());

        // 5. 正确密码导入成功
        let import_report = run_import(&ImportOptions {
            base_dir: dest_base_dir.clone(),
            input_path: export_file.clone(),
            password: "strong-export-pass-123".to_string(),
            target_agents: None,
            clean_incompatible: false,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(import_report.total_accounts, 2);
        assert!(import_report.imported_agents.contains(&"codex".to_string()));
        assert!(import_report.imported_agents.contains(&"agy".to_string()));

        // 6. 验证目标端账号解包与存储正确性
        let dest_crypto = CryptoManager::create_provider(None, None);
        let loaded_codex = dest_codex_store
            .read_account(codex_key, &*dest_crypto)
            .unwrap();
        assert_eq!(loaded_codex, codex_secret);

        let dest_agy_store = Store::new(dest_base_dir.join("agy"));
        let loaded_agy = dest_agy_store.read_account(agy_key, &*dest_crypto).unwrap();
        assert_eq!(loaded_agy, agy_secret);

        // 验证 codex 原有账号依然存在（被合并入注册表）
        let loaded_old = dest_codex_store
            .read_account("codex:old-user", &*dest_crypto)
            .unwrap();
        assert_eq!(loaded_old, initial_secret);

        // 7. 验证预导入快照 backups/pre_import_* 成功生成
        let backups_dir = dest_codex_dir.join("backups");
        assert!(backups_dir.exists());
        let pre_import_entries: Vec<_> = fs::read_dir(&backups_dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("pre_import_"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            !pre_import_entries.is_empty(),
            "Expected pre_import_* snapshot directory to exist"
        );

        let pre_import_dir = pre_import_entries[0].path();
        assert!(pre_import_dir.join("registry.json").exists());
        let backed_reg_content = fs::read_to_string(pre_import_dir.join("registry.json")).unwrap();
        assert!(backed_reg_content.contains("old@example.com"));
    }

    #[test]
    fn test_import_incompatible_data_clean() {
        let dir = tempdir().unwrap();
        let base_dir = dir.path().to_path_buf();

        // 构造一个目标目录下损坏的 registry.json
        let codex_dir = base_dir.join("codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(codex_dir.join("registry.json"), b"corrupted-invalid-json").unwrap();

        let plain_crypto = CryptoManager::create_provider(Some("plain"), None);
        let compat = check_agent_compatibility(&codex_dir, &*plain_crypto);
        assert!(matches!(compat, CompatibilityStatus::Incompatible(_)));

        // 导出一个合法的数据包
        let src_dir = dir.path().join("src_machine");
        let src_codex = src_dir.join("codex");
        let src_store = Store::new(src_codex);
        src_store
            .write_account("codex:acc", b"data", &*plain_crypto)
            .unwrap();
        let mut reg = src_store.load_registry().unwrap();
        reg.upsert(Account {
            key: "codex:acc".to_string(),
            alias: None,
            email: "clean@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        src_store.save_registry(&reg).unwrap();

        let export_file = dir.path().join("data.agx");
        run_export(&ExportOptions {
            base_dir: src_dir,
            output_path: export_file.clone(),
            password: "pwd".to_string(),
            target_agents: vec!["codex".to_string()],
        })
        .unwrap();

        // 不开启 clean_incompatible 导入报错
        let err = run_import(&ImportOptions {
            base_dir: base_dir.clone(),
            input_path: export_file.clone(),
            password: "pwd".to_string(),
            target_agents: None,
            clean_incompatible: false,
            overwrite: true,
        });
        assert!(err.is_err());

        // 开启 clean_incompatible 成功清理备份并导入
        let report = run_import(&ImportOptions {
            base_dir: base_dir.clone(),
            input_path: export_file.clone(),
            password: "pwd".to_string(),
            target_agents: None,
            clean_incompatible: true,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(report.total_accounts, 1);
        assert_eq!(report.cleaned_agents, vec!["codex"]);
        assert!(!report.backup_dirs.is_empty());

        // 验证旧文件已备份在 backup_dirs 中
        let backup_reg = report.backup_dirs[0].join("registry.json");
        assert!(backup_reg.exists());
        assert_eq!(
            fs::read_to_string(backup_reg).unwrap(),
            "corrupted-invalid-json"
        );

        // 验证新数据导入成功
        let store = Store::new(codex_dir);
        let new_reg = store.load_registry().unwrap();
        assert_eq!(new_reg.accounts[0].email, "clean@example.com");
    }

    #[test]
    fn test_import_incompatible_different_key_with_clean() {
        let dir = tempdir().unwrap();
        let base_dir = dir.path().to_path_buf();

        // 1. 在目标 codex 目录下用 "old-key-111" 加密写入一个账号
        let codex_dir = base_dir.join("codex");
        let store = Store::new(codex_dir.clone());
        let old_crypto = CryptoManager::create_provider(Some("env"), Some("old-key-111"));
        let old_key = "codex:old-acc";
        let old_secret = b"{\"token\":\"old-encrypted-secret\"}";
        store
            .write_account(old_key, old_secret, &*old_crypto)
            .unwrap();

        let mut reg = store.load_registry().unwrap();
        reg.upsert(Account {
            key: old_key.to_string(),
            alias: Some("oldalias".to_string()),
            email: "old-diff-key@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        store.save_registry(&reg).unwrap();

        // 配置目标机器当前使用 "new-key-222"
        let current_target_crypto =
            CryptoManager::create_provider(Some("env"), Some("new-key-222"));

        // 验证由于密钥不同，解密失败，被判定为 Incompatible
        let compat = check_agent_compatibility(&codex_dir, &*current_target_crypto);
        match compat {
            CompatibilityStatus::Incompatible(reason) => {
                assert!(
                    reason.contains("无法使用当前本地密钥或引擎解密"),
                    "Reason was: {}",
                    reason
                );
            }
            other => panic!(
                "Expected Incompatible status due to key mismatch, got {:?}",
                other
            ),
        }

        // 2. 准备源机器导出包
        let src_dir = dir.path().join("src_machine");
        let src_codex = src_dir.join("codex");
        let src_store = Store::new(src_codex);
        let src_plain_crypto = CryptoManager::create_provider(Some("plain"), None);
        let new_key = "codex:imported-acc";
        let new_secret = b"{\"token\":\"new-imported-secret\"}";
        src_store
            .write_account(new_key, new_secret, &*src_plain_crypto)
            .unwrap();
        let mut src_reg = src_store.load_registry().unwrap();
        src_reg.upsert(Account {
            key: new_key.to_string(),
            alias: Some("newalias".to_string()),
            email: "new-imported@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        src_store.save_registry(&src_reg).unwrap();

        let export_file = dir.path().join("package.agx");
        run_export(&ExportOptions {
            base_dir: src_dir,
            output_path: export_file.clone(),
            password: "export-password-999".to_string(),
            target_agents: vec!["codex".to_string()],
        })
        .unwrap();

        // 3. 在目标环境上更新 global config 为 env 引擎
        let mut cfg = load_global_config(&base_dir).unwrap();
        cfg.crypto_provider = Some("env".to_string());
        save_global_config(&base_dir, &cfg).unwrap();

        // 临时设置当前环境变量 AGS_KEY 为 new-key-222
        struct EnvGuard;
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                unsafe {
                    std::env::remove_var("AGS_KEY");
                }
            }
        }
        let _guard = EnvGuard;
        unsafe {
            std::env::set_var("AGS_KEY", "new-key-222");
        }

        // 不开启 clean_incompatible 导入必定失败
        let err = run_import(&ImportOptions {
            base_dir: base_dir.clone(),
            input_path: export_file.clone(),
            password: "export-password-999".to_string(),
            target_agents: None,
            clean_incompatible: false,
            overwrite: true,
        });
        assert!(
            err.is_err(),
            "Expected error when clean_incompatible is false"
        );

        // 开启 clean_incompatible 成功清理旧数据并备份，然后完成导入
        let report = run_import(&ImportOptions {
            base_dir: base_dir.clone(),
            input_path: export_file.clone(),
            password: "export-password-999".to_string(),
            target_agents: None,
            clean_incompatible: true,
            overwrite: true,
        })
        .unwrap();

        assert_eq!(report.total_accounts, 1);
        assert_eq!(report.cleaned_agents, vec!["codex"]);
        assert_eq!(report.backup_dirs.len(), 1);

        // 验证备份目录存在，且备份了旧的 accounts 快照和 registry
        let backup_dir = &report.backup_dirs[0];
        assert!(backup_dir.join("registry.json").exists());
        let backed_reg_str = fs::read_to_string(backup_dir.join("registry.json")).unwrap();
        assert!(backed_reg_str.contains("old-diff-key@example.com"));

        // 验证新账号成功导入并可用当前 new-key-222 解密
        let loaded = store
            .read_account(new_key, &*current_target_crypto)
            .unwrap();
        assert_eq!(loaded, new_secret);

        let final_reg = store.load_registry().unwrap();
        assert_eq!(final_reg.accounts.len(), 1);
        assert_eq!(final_reg.accounts[0].email, "new-imported@example.com");
    }

    #[test]
    fn test_check_agent_compatibility_scenarios() {
        let dir = tempdir().unwrap();
        let base_dir = dir.path().to_path_buf();
        let plain_crypto = CryptoManager::create_provider(Some("plain"), None);

        // 1. 不存在的目录 -> Clean
        let non_exist = base_dir.join("non_exist");
        assert_eq!(
            check_agent_compatibility(&non_exist, &*plain_crypto),
            CompatibilityStatus::Clean
        );

        // 2. 空目录 -> Clean
        let empty_dir = base_dir.join("empty");
        fs::create_dir_all(&empty_dir).unwrap();
        assert_eq!(
            check_agent_compatibility(&empty_dir, &*plain_crypto),
            CompatibilityStatus::Clean
        );

        // 3. 有 accounts 文件但无 registry.json -> Incompatible
        let orphan_dir = base_dir.join("orphan");
        fs::create_dir_all(orphan_dir.join("accounts")).unwrap();
        fs::write(
            orphan_dir.join("accounts").join("orphan.json"),
            b"some data",
        )
        .unwrap();
        let compat = check_agent_compatibility(&orphan_dir, &*plain_crypto);
        assert!(matches!(compat, CompatibilityStatus::Incompatible(_)));

        // 4. registry.json 格式损坏 -> Incompatible
        let corrupt_dir = base_dir.join("corrupt");
        fs::create_dir_all(&corrupt_dir).unwrap();
        fs::write(corrupt_dir.join("registry.json"), b"not valid json").unwrap();
        let compat = check_agent_compatibility(&corrupt_dir, &*plain_crypto);
        assert!(matches!(compat, CompatibilityStatus::Incompatible(_)));

        // 5. registry.json 登记了账号但 snapshot 文件缺失 -> Incompatible
        let missing_snap_dir = base_dir.join("missing_snap");
        let store_missing = Store::new(missing_snap_dir.clone());
        let mut reg_missing = Registry::default();
        reg_missing.upsert(Account {
            key: "test:missing".to_string(),
            alias: None,
            email: "missing@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        store_missing.save_registry(&reg_missing).unwrap();
        let compat = check_agent_compatibility(&missing_snap_dir, &*plain_crypto);
        assert!(matches!(compat, CompatibilityStatus::Incompatible(_)));

        // 6. 正常已加密数据使用正确 key -> Compatible
        let valid_dir = base_dir.join("valid");
        let store_valid = Store::new(valid_dir.clone());
        let env_crypto = CryptoManager::create_provider(Some("env"), Some("test-key-abc"));
        store_valid
            .write_account("test:valid", b"secret", &*env_crypto)
            .unwrap();
        let mut reg_valid = Registry::default();
        reg_valid.upsert(Account {
            key: "test:valid".to_string(),
            alias: None,
            email: "valid@example.com".to_string(),
            plan: None,
            mode: None,
            added_at: Utc::now(),
            updated_at: None,
        });
        store_valid.save_registry(&reg_valid).unwrap();

        assert_eq!(
            check_agent_compatibility(&valid_dir, &*env_crypto),
            CompatibilityStatus::Compatible
        );

        // 7. 使用错误 key 检查正常数据 -> Incompatible
        let wrong_crypto = CryptoManager::create_provider(Some("env"), Some("wrong-key-xyz"));
        assert!(matches!(
            check_agent_compatibility(&valid_dir, &*wrong_crypto),
            CompatibilityStatus::Incompatible(_)
        ));
    }
}
