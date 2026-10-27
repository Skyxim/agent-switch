use crate::agent::dto::{
    AgyLoadCodeAssistResponse, AgyQuotaBucketDto, CodexJwtClaims, GeminiOAuthCredentialsFile,
    GoogleOAuthTokenResponse,
};
use crate::agent::traits::{
    AccountDisplayItem, AccountIdentity, AccountListModel, AgentApp, BoxFuture, CurrentAccountInfo,
    UsageReport,
};
use crate::crypto::CryptoProvider;
use crate::util::{atomic_write, decode_jwt_payload};
use crate::vault::{Account, Store};
use anyhow::{bail, Context, Result};
use base64::prelude::*;
use chrono::Utc;
use colored::*;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub const AGY_SERVICE: &str = "gemini";
pub const AGY_USER: &str = "antigravity";

const MASKED_CLIENT_ID: &[u8] = &[
    0x6b, 0x6a, 0x6d, 0x6b, 0x6a, 0x6a, 0x6c, 0x6a, 0x6c, 0x6a, 0x6f, 0x63, 0x6b, 0x77, 0x2e, 0x37,
    0x32, 0x29, 0x29, 0x33, 0x34, 0x68, 0x32, 0x68, 0x6b, 0x36, 0x39, 0x28, 0x3f, 0x68, 0x69, 0x6f,
    0x2c, 0x2e, 0x35, 0x36, 0x35, 0x30, 0x32, 0x6e, 0x3d, 0x6e, 0x6a, 0x69, 0x3f, 0x2a, 0x74, 0x3b,
    0x2a, 0x2a, 0x29, 0x74, 0x3d, 0x35, 0x35, 0x3d, 0x36, 0x3f, 0x2f, 0x29, 0x3f, 0x28, 0x39, 0x35,
    0x34, 0x2e, 0x3f, 0x34, 0x2e, 0x74, 0x39, 0x35, 0x37,
];

const MASKED_CLIENT_SECRET: &[u8] = &[
    0x1d, 0x15, 0x19, 0x09, 0x0a, 0x02, 0x77, 0x11, 0x6f, 0x62, 0x1c, 0x0d, 0x08, 0x6e, 0x62, 0x6c,
    0x16, 0x3e, 0x16, 0x10, 0x6b, 0x37, 0x16, 0x18, 0x62, 0x29, 0x02, 0x19, 0x6e, 0x20, 0x6c, 0x2b,
    0x1e, 0x1b, 0x3c,
];

pub fn unmask_oauth_credential(b: &[u8]) -> String {
    b.iter().map(|&c| (c ^ 0x5a) as char).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgyCreds {
    #[serde(default)]
    pub token: AgyToken,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token_email: Option<String>,
}

pub type AgyCredentials = AgyCreds;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgyToken {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_date: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
}

/// Resolves the Antigravity base directory:
/// 1. `ANTIGRAVITY_HOME` (if non-empty)
/// 2. `AGY_HOME` (if non-empty)
/// 3. `$HOME/.gemini/antigravity-cli` (via `dirs::home_dir()` or `HOME`)
///
/// Fallback: `.gemini/antigravity-cli`
pub fn default_dir() -> PathBuf {
    if let Ok(val) = env::var("ANTIGRAVITY_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(val) = env::var("AGY_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let home = dirs::home_dir().or_else(|| {
        env::var("HOME").ok().and_then(|h| {
            let trimmed = h.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        })
    });
    if let Some(user_home) = home {
        return user_home.join(".gemini").join("antigravity-cli");
    }
    PathBuf::from(".gemini").join("antigravity-cli")
}

/// Resolves the default path to `antigravity-oauth-token`:
/// 1. `ANTIGRAVITY_HOME/antigravity-oauth-token` or `AGY_HOME/antigravity-oauth-token`
/// 2. `$HOME/.gemini/antigravity-cli/antigravity-oauth-token`
pub fn default_token_path() -> PathBuf {
    default_dir().join("antigravity-oauth-token")
}

/// Resolves the default path to legacy/fallback credentials (`oauth_creds.json`):
/// 1. `ANTIGRAVITY_HOME/oauth_creds.json` or `AGY_HOME/oauth_creds.json`
/// 2. `$HOME/.gemini/oauth_creds.json`
pub fn default_creds_path() -> PathBuf {
    if let Ok(val) = env::var("ANTIGRAVITY_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("oauth_creds.json");
        }
    }
    if let Ok(val) = env::var("AGY_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("oauth_creds.json");
        }
    }
    let home = dirs::home_dir().or_else(|| {
        env::var("HOME").ok().and_then(|h| {
            let trimmed = h.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        })
    });
    if let Some(user_home) = home {
        return user_home.join(".gemini").join("oauth_creds.json");
    }
    PathBuf::from(".gemini").join("oauth_creds.json")
}

#[derive(Default)]
pub struct AgyAgent;

impl AgyAgent {
    pub fn new() -> Self {
        Self
    }

    pub fn default_dir() -> PathBuf {
        default_dir()
    }

    pub fn default_token_path() -> PathBuf {
        default_token_path()
    }

    pub fn default_creds_path() -> PathBuf {
        default_creds_path()
    }

    /// Read raw credentials from Keyring, environment variable, or fallback files
    pub fn read_credentials_raw() -> Result<Vec<u8>> {
        // 1. Check environment variable AGY_KEYRING_DATA
        if let Ok(val) = env::var("AGY_KEYRING_DATA") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                if let Ok(decoded) = BASE64_STANDARD.decode(trimmed) {
                    return Ok(decoded);
                } else {
                    return Ok(trimmed.as_bytes().to_vec());
                }
            }
        }

        // 2. Try OS Keyring
        if let Ok(data) = Self::read_keyring() {
            if !data.is_empty() {
                return Ok(data);
            }
        }

        // 3. Prefer antigravity-oauth-token
        let agy_token_path = Self::default_token_path();
        if agy_token_path.exists() {
            if let Ok(content) = fs::read(&agy_token_path) {
                let trimmed = content.trim_ascii();
                if !trimmed.is_empty() {
                    return Ok(content);
                }
            }
        }

        // 4. Fallback to oauth_creds.json
        let oauth_path = Self::default_creds_path();
        if oauth_path.exists() {
            if let Ok(content) = fs::read(&oauth_path) {
                let trimmed = content.trim_ascii();
                if !trimmed.is_empty() {
                    if let Ok(converted) = Self::convert_gemini_oauth_json(&content) {
                        return Ok(converted);
                    }
                    return Ok(content);
                }
            }
        }

        bail!(
            "Could not find Antigravity credentials in Keyring, AGY_KEYRING_DATA, {}, or {}",
            agy_token_path.display(),
            oauth_path.display()
        )
    }

    /// Read all candidate raw credentials from Keyring, environment, and known credential files.
    pub fn read_all_candidate_credentials_raw() -> Vec<Vec<u8>> {
        let mut candidates = Vec::new();

        if let Ok(val) = env::var("AGY_KEYRING_DATA") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                if let Ok(decoded) = BASE64_STANDARD.decode(trimmed) {
                    candidates.push(decoded);
                } else {
                    candidates.push(trimmed.as_bytes().to_vec());
                }
            }
        }

        if let Ok(data) = Self::read_keyring() {
            if !data.is_empty() {
                candidates.push(data);
            }
        }

        let agy_token_path = Self::default_token_path();
        if agy_token_path.exists() {
            if let Ok(content) = fs::read(&agy_token_path) {
                let trimmed = content.trim_ascii();
                if !trimmed.is_empty() {
                    candidates.push(content);
                }
            }
        }

        let oauth_path = Self::default_creds_path();
        if oauth_path.exists() {
            if let Ok(content) = fs::read(&oauth_path) {
                let trimmed = content.trim_ascii();
                if !trimmed.is_empty() {
                    if let Ok(converted) = Self::convert_gemini_oauth_json(&content) {
                        candidates.push(converted);
                    } else {
                        candidates.push(content);
                    }
                }
            }
        }

        candidates
    }

    /// Attempt to read from system keyring (Linux secret-tool, macOS security)
    fn read_keyring() -> Result<Vec<u8>> {
        #[cfg(target_os = "linux")]
        {
            // Try secret-tool lookup service gemini username antigravity
            let output = Command::new("secret-tool")
                .args(["lookup", "service", AGY_SERVICE, "username", AGY_USER])
                .output();

            if let Ok(out) = output {
                if out.status.success() && !out.stdout.is_empty() {
                    return Self::decode_keyring_value(&out.stdout);
                }
            }

            // Also try lookup service gemini:antigravity
            let output2 = Command::new("secret-tool")
                .args([
                    "lookup",
                    "service",
                    &format!("{}:{}", AGY_SERVICE, AGY_USER),
                ])
                .output();

            if let Ok(out) = output2 {
                if out.status.success() && !out.stdout.is_empty() {
                    return Self::decode_keyring_value(&out.stdout);
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let output = Command::new("/usr/bin/security")
                .args([
                    "find-generic-password",
                    "-s",
                    AGY_SERVICE,
                    "-a",
                    AGY_USER,
                    "-w",
                ])
                .output();

            if let Ok(out) = output {
                if out.status.success() && !out.stdout.is_empty() {
                    return Self::decode_keyring_value(&out.stdout);
                }
            }
        }

        bail!("Keyring lookup returned no result")
    }

    /// Write credentials to system Keyring and fallback files
    pub fn write_credentials(data: &[u8]) -> Result<()> {
        Self::write_credentials_raw(data)
    }

    /// Write credentials to system Keyring, antigravity-oauth-token, and oauth_creds.json
    pub fn write_credentials_raw(data: &[u8]) -> Result<()> {
        let mut keyring_written = false;

        #[cfg(target_os = "linux")]
        {
            let label = format!("Password for '{}' on '{}'", AGY_USER, AGY_SERVICE);
            let child = Command::new("secret-tool")
                .args([
                    "store",
                    &format!("--label={}", label),
                    "service",
                    AGY_SERVICE,
                    "username",
                    AGY_USER,
                ])
                .stdin(std::process::Stdio::piped())
                .spawn();

            if let Ok(mut c) = child {
                use std::io::Write;
                if let Some(ref mut stdin) = c.stdin {
                    let _ = stdin.write_all(data);
                }
                if let Ok(status) = c.wait() {
                    if status.success() {
                        keyring_written = true;
                    }
                }
            }
        }

        #[cfg(target_os = "macos")]
        {
            let b64 = BASE64_STANDARD.encode(data);
            let line = format!("go-keyring-base64:{}", b64);
            let status = Command::new("/usr/bin/security")
                .args([
                    "add-generic-password",
                    "-U",
                    "-s",
                    AGY_SERVICE,
                    "-a",
                    AGY_USER,
                    "-w",
                    &line,
                ])
                .status();
            if let Ok(s) = status {
                if s.success() {
                    keyring_written = true;
                }
            }
        }

        let _ = keyring_written;

        let parsed_creds = serde_json::from_slice::<AgyCreds>(data).ok();

        // 1. Write antigravity-oauth-token
        let token_path = Self::default_token_path();
        if let Some(mut creds) = parsed_creds.clone() {
            if creds.auth_method.is_none() {
                creds.auth_method = Some("consumer".to_string());
            }
            if creds.id_token_email.is_none() {
                if let Some(ref it) = creds.id_token {
                    if let Some(payload) = decode_jwt_payload(it) {
                        if let Some(e) = payload.get("email").and_then(|v| v.as_str()) {
                            creds.id_token_email = Some(e.to_string());
                        }
                    }
                }
            }
            if creds.token.token_type.is_none() {
                creds.token.token_type = Some("Bearer".to_string());
            }
            if creds.token.expiry.is_none() && creds.token.expiry_date.is_some() {
                if let Some(exp_date) = creds.token.expiry_date {
                    let dt_opt = if exp_date > 100_000_000_000 {
                        chrono::DateTime::from_timestamp_millis(exp_date)
                    } else {
                        chrono::DateTime::from_timestamp(exp_date, 0)
                    };
                    if let Some(dt) = dt_opt {
                        creds.token.expiry = Some(dt.to_rfc3339());
                    }
                }
            } else if creds.token.expiry_date.is_none() && creds.token.expiry.is_some() {
                if let Some(ref exp_str) = creds.token.expiry {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(exp_str) {
                        creds.token.expiry_date = Some(dt.timestamp_millis());
                    }
                }
            }
            if creds.token.expires_in.is_none() {
                creds.token.expires_in = Some(3599);
            }

            if let Ok(agy_token_json) = serde_json::to_string_pretty(&creds) {
                let _ = atomic_write(&token_path, agy_token_json.as_bytes());
            }
        } else {
            let _ = atomic_write(&token_path, data);
        }

        // 2. Write oauth_creds.json
        let oauth_path = Self::default_creds_path();
        if let Some(parent) = oauth_path.parent() {
            if parent.exists() {
                if let Some(ref creds) = parsed_creds {
                    let mut obj = serde_json::Map::new();
                    if let Some(acc) = &creds.token.access_token {
                        obj.insert(
                            "access_token".to_string(),
                            serde_json::Value::String(acc.clone()),
                        );
                    }
                    if let Some(ref_tok) = &creds.token.refresh_token {
                        obj.insert(
                            "refresh_token".to_string(),
                            serde_json::Value::String(ref_tok.clone()),
                        );
                    }
                    if let Some(id_tok) = &creds.id_token {
                        obj.insert(
                            "id_token".to_string(),
                            serde_json::Value::String(id_tok.clone()),
                        );
                    }
                    let expiry_millis = creds.token.expiry_date.or_else(|| {
                        creds.token.expiry.as_deref().and_then(|exp_str| {
                            chrono::DateTime::parse_from_rfc3339(exp_str)
                                .ok()
                                .map(|dt| dt.timestamp_millis())
                        })
                    });
                    if let Some(exp) = expiry_millis {
                        obj.insert(
                            "expiry_date".to_string(),
                            serde_json::Value::Number(exp.into()),
                        );
                    }
                    let token_type = creds.token.token_type.as_deref().unwrap_or("Bearer");
                    obj.insert(
                        "token_type".to_string(),
                        serde_json::Value::String(token_type.to_string()),
                    );
                    if let Ok(json_str) = serde_json::to_string_pretty(&obj) {
                        let _ = atomic_write(&oauth_path, json_str.as_bytes());
                    }
                }
            }
        }

        Ok(())
    }

    fn decode_keyring_value(raw: &[u8]) -> Result<Vec<u8>> {
        let trimmed = raw.strip_suffix(b"\n").unwrap_or(raw);
        if trimmed.starts_with(b"go-keyring-base64:") {
            let b64 = &trimmed[b"go-keyring-base64:".len()..];
            return Ok(BASE64_STANDARD.decode(b64)?);
        }
        if trimmed.starts_with(b"go-keyring-encoded:") {
            let hex_bytes = &trimmed[b"go-keyring-encoded:".len()..];
            return Ok(hex::decode(hex_bytes)?);
        }
        Ok(trimmed.to_vec())
    }

    fn convert_gemini_oauth_json(content: &[u8]) -> Result<Vec<u8>> {
        let file: GeminiOAuthCredentialsFile = serde_json::from_slice(content)?;
        let mut email = file.id_token_email;
        let id_token = file.id_token;
        if email.is_none() {
            if let Some(ref it) = id_token {
                if let Some(claims) = CodexJwtClaims::from_jwt(it) {
                    email = claims.email;
                }
            }
        }
        let creds = AgyCreds {
            token: AgyToken {
                access_token: file.access_token,
                refresh_token: file.refresh_token,
                expiry: None,
                expiry_date: file.expiry_date,
                token_type: file.token_type,
                expires_in: None,
            },
            id_token,
            auth_method: Some("consumer".to_string()),
            id_token_email: email,
        };
        Ok(serde_json::to_vec_pretty(&creds)?)
    }

    /// Parse identity (email, sub) from agy credentials
    pub fn parse_identity(data: &[u8]) -> Result<(String, String)> {
        let creds: AgyCreds = serde_json::from_slice(data)
            .context("Failed to parse Antigravity credentials schema")?;

        let mut email = String::new();
        let mut sub = String::new();

        if let Some(ref id_token) = creds.id_token {
            if let Some(claims) = CodexJwtClaims::from_jwt(id_token) {
                if let Some(e) = claims.email {
                    email = e;
                }
                if let Some(s) = claims.sub {
                    sub = s;
                }
            }
        }

        if email.is_empty() {
            if let Some(ref e) = creds.id_token_email {
                let trimmed = e.trim();
                if !trimmed.is_empty() {
                    email = trimmed.to_string();
                }
            }
        }

        if email.is_empty() {
            email = "unknown_google_user".to_string();
        }

        let key = if !sub.is_empty() {
            format!("google:{}", sub)
        } else {
            format!("google:{}", email)
        };

        Ok((key, email))
    }

    pub fn add(
        &self,
        alias: Option<&str>,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<(Account, bool)> {
        let raw_data = Self::read_credentials_raw()?;
        let (key, email) = Self::parse_identity(&raw_data)?;

        let mut reg = store.load_registry()?;
        let account = Account {
            key: key.clone(),
            alias: alias.map(|s| s.to_string()),
            email: email.clone(),
            plan: Some("Google Cloud Code".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };

        store.write_account(&key, &raw_data, crypto)?;
        let (is_update, old_key) = reg.add_or_update(account);
        if let Some(old_k) = old_key {
            if old_k != key {
                let _ = store.delete_account(&old_k);
            }
        }
        reg.active = Some(key.clone());
        store.save_registry(&reg)?;

        let saved_idx = reg
            .find_by_key_or_email(&key, &email)
            .context("Account not found in registry after saving")?;
        Ok((reg.accounts[saved_idx].clone(), is_update))
    }

    pub fn list(&self, store: &Store) -> Result<()> {
        self.list_with_header(store, false)
    }

    pub fn list_with_header(&self, store: &Store, show_app_header: bool) -> Result<()> {
        let reg = store.load_registry()?;
        if reg.accounts.is_empty() {
            if show_app_header {
                println!("{}", "Antigravity".bold().cyan());
                println!(
                    "  {}",
                    "No saved Antigravity accounts. Use `ags agy add <name>` to add one.".dimmed()
                );
            } else {
                println!(
                    "{}",
                    "No saved Antigravity accounts. Use `ags agy add <name>` to add one.".dimmed()
                );
            }
            return Ok(());
        }

        if show_app_header {
            println!("{}", "Antigravity".bold().cyan());
        }
        println!(
            "{:<3}   {:<15} {:<30} {:<18} {:<10}",
            "#", "Alias", "Email", "Plan", "Added"
        );
        println!("{}", "-".repeat(78).dimmed());

        for (idx, acc) in reg.accounts.iter().enumerate() {
            let is_active = reg.active.as_deref() == Some(&acc.key);
            let active_marker = if is_active {
                "*".green().bold().to_string()
            } else {
                " ".to_string()
            };
            let alias_str = acc.alias.as_deref().unwrap_or("-");
            let plan_str = acc.plan.as_deref().unwrap_or("Standard");
            let added_str = acc.added_at.format("%Y-%m-%d").to_string();

            let padded_alias = format!("{:<15}", alias_str);
            let padded_email = format!("{:<30}", acc.email);

            if is_active {
                println!(
                    "{:<3} {} {} {} {:<18} {:<10}",
                    idx + 1,
                    active_marker,
                    padded_alias.green().bold(),
                    padded_email.green(),
                    plan_str,
                    added_str.dimmed()
                );
            } else {
                println!(
                    "{:<3} {} {} {} {:<18} {:<10}",
                    idx + 1,
                    active_marker,
                    padded_alias,
                    padded_email,
                    plan_str,
                    added_str.dimmed()
                );
            }
        }
        Ok(())
    }

    pub fn switch(
        &self,
        query: &str,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<String> {
        let mut reg = store.load_registry()?;
        let idx = reg.find(query)?;
        let target_account = reg.accounts[idx].clone();

        // 1. Back up current credentials if available
        if let Ok(current_data) = Self::read_credentials_raw() {
            let backup_label = reg.active.as_deref().unwrap_or("current_agy");
            let _ = store.backup_file(&current_data, backup_label);
        }

        // 2. Read and decrypt target snapshot
        let target_data = store.read_account(&target_account.key, crypto)?;

        // 3. Write target credentials to Keyring and storage
        Self::write_credentials(&target_data)?;

        // 4. Update registry
        reg.previous = reg.active.take();
        reg.active = Some(target_account.key.clone());
        store.save_registry(&reg)?;

        let display_name = target_account
            .alias
            .as_deref()
            .unwrap_or(&target_account.email);
        Ok(format!(
            "Switched Antigravity to '{}' ({})",
            display_name, target_account.email
        ))
    }

    pub fn current(&self, store: &Store) -> Result<()> {
        let raw = Self::read_credentials_raw()?;
        let (key, email) = Self::parse_identity(&raw)?;
        let reg = store.load_registry()?;

        let matched_account = reg.accounts.iter().find(|a| a.key == key);
        let alias = matched_account
            .and_then(|a| a.alias.as_deref())
            .unwrap_or("-");
        let active_match = reg.active.as_deref() == Some(&key);

        println!("{}", "Current Antigravity (agy) Account:".bold());
        println!("  Email:       {}", email.cyan());
        println!("  Alias:       {}", alias.yellow());
        println!("  Key:         {}", key.dimmed());
        println!(
            "  In Registry: {}",
            if matched_account.is_some() {
                "Yes".green()
            } else {
                "No (untracked)".red()
            }
        );
        println!(
            "  Is Active:   {}",
            if active_match {
                "Yes".green()
            } else {
                "No".yellow()
            }
        );
        Ok(())
    }

    pub fn remove(&self, query: &str, store: &Store) -> Result<String> {
        let mut reg = store.load_registry()?;
        let idx = reg.find(query)?;
        let key = reg.accounts[idx].key.clone();
        let alias = reg.accounts[idx].alias.clone();
        let email = reg.accounts[idx].email.clone();

        store.delete_account(&key)?;
        reg.remove(&key);
        store.save_registry(&reg)?;

        let name = alias.as_deref().unwrap_or(&email);
        Ok(format!(
            "Removed Antigravity account '{}' ({})",
            name, email
        ))
    }

    pub async fn usage(&self, store: &Store, crypto: &dyn CryptoProvider) -> Result<()> {
        let report = self.collect_usage_report(store, crypto).await?;
        crate::render::render_usage_report(&report, false);
        Ok(())
    }

    pub async fn collect_usage_report(
        &self,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<crate::model::AppUsageReport> {
        let reg = store.load_registry()?;
        let mut report = crate::model::AppUsageReport::new("Antigravity");
        if reg.accounts.is_empty() {
            return Ok(report);
        }

        let reg_active = reg.active.clone();
        let client = reqwest::Client::new();
        let tasks = reg.accounts.into_iter().enumerate().map(|(idx, acc)| {
            let client = client.clone();
            let is_active = reg_active.as_deref() == Some(&acc.key);

            async move {
                let res =
                    Self::fetch_single_account_quota(&client, store, crypto, &acc, is_active).await;
                (idx, acc, is_active, res)
            }
        });

        use futures_util::StreamExt;
        let mut results: Vec<_> = futures_util::stream::iter(tasks)
            .buffer_unordered(4)
            .collect()
            .await;

        results.sort_by_key(|(idx, _, _, _)| *idx);

        for (idx, acc, is_active, res) in results {
            let mut acc_report = crate::model::AccountUsageReport::new(idx + 1, &acc.email);
            acc_report.alias = acc.alias;
            acc_report.is_active = is_active;

            match res {
                Ok((tier, summary, direct)) => {
                    let tier_name = if tier.is_empty() {
                        acc.plan.unwrap_or_else(|| "Standard".to_string())
                    } else {
                        tier
                    };
                    acc_report.tier = Some(tier_name);
                    acc_report.groups =
                        Self::parse_agy_quota_groups(summary.as_ref(), direct.as_ref());
                }
                Err(e) => {
                    acc_report.tier = acc.plan;
                    acc_report.error = Some(e.to_string());
                }
            }
            report.accounts.push(acc_report);
        }

        Ok(report)
    }

    async fn fetch_single_account_quota(
        client: &reqwest::Client,
        store: &Store,
        crypto: &dyn CryptoProvider,
        acc: &Account,
        is_active: bool,
    ) -> Result<(String, Option<serde_json::Value>, Option<serde_json::Value>)> {
        let mut creds = if is_active {
            let mut matched = None;
            for raw in Self::read_all_candidate_credentials_raw() {
                if let Ok((sys_key, sys_email)) = Self::parse_identity(&raw) {
                    if sys_key == acc.key || sys_email.eq_ignore_ascii_case(&acc.email) {
                        if let Ok(c) = serde_json::from_slice::<AgyCreds>(&raw) {
                            matched = Some(c);
                            break;
                        }
                    }
                }
            }
            if matched.is_none() {
                if let Ok(sys_raw) = Self::read_credentials_raw() {
                    if let Ok((sys_key, sys_email)) = Self::parse_identity(&sys_raw) {
                        if sys_key == acc.key || sys_email.eq_ignore_ascii_case(&acc.email) {
                            matched = serde_json::from_slice::<AgyCreds>(&sys_raw).ok();
                        }
                    }
                }
            }
            matched
        } else {
            None
        };

        if creds.is_none() {
            let data = store.read_account(&acc.key, crypto)?;
            creds = Some(serde_json::from_slice::<AgyCreds>(&data)?);
        }

        let mut creds = creds.context("Failed to load Antigravity credentials")?;

        // Proactive token refresh if expiring (< 5 mins) or expired
        let mut refreshed = false;
        if creds.token.refresh_token.is_some() && Self::is_token_expiring(&creds.token) {
            match Self::refresh_token_static(&mut creds, Some(store), Some(crypto)).await {
                Ok(_) => {
                    refreshed = true;
                }
                Err(e) => {
                    if e.to_string().contains("invalid_grant")
                        || e.to_string().contains("expired or revoked")
                    {
                        bail!(
                            "Antigravity token expired and refresh failed. Please run `agy auth` (or sign in again in Antigravity) and update credentials with `ags agy add`: {}",
                            e
                        );
                    }
                    if creds.token.access_token.is_none() {
                        bail!(
                            "Failed to refresh Antigravity token and no access token available: {}",
                            e
                        );
                    }
                }
            }
        }

        let mut access_token = creds
            .token
            .access_token
            .clone()
            .context("No access token present in Antigravity credentials")?;

        let ua = Self::official_user_agent();

        // 1. Prioritize loadCodeAssist + retrieveUserQuotaSummary (matches official IDE/UI)
        let mut load_res = client
            .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("User-Agent", &ua)
            .json(&serde_json::json!({
                "metadata": {
                    "ideType": "ANTIGRAVITY"
                }
            }))
            .send()
            .await
            .context("Failed to send loadCodeAssist request")?;

        if load_res.status() == reqwest::StatusCode::UNAUTHORIZED
            && !refreshed
            && creds.token.refresh_token.is_some()
        {
            Self::refresh_token_static(&mut creds, Some(store), Some(crypto))
                .await
                .context("loadCodeAssist returned 401 and token refresh failed. Please run `agy auth` and update with `ags agy add`")?;
            refreshed = true;
            access_token = creds
                .token
                .access_token
                .clone()
                .context("No access token after refresh")?;

            load_res = client
                .post("https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist")
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .header("User-Agent", &ua)
                .json(&serde_json::json!({
                    "metadata": {
                        "ideType": "ANTIGRAVITY"
                    }
                }))
                .send()
                .await
                .context("Failed to send loadCodeAssist retry request")?;
        }

        if !load_res.status().is_success() {
            let status = load_res.status();
            let body = load_res.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                bail!("loadCodeAssist returned 401 Unauthorized: token expired or invalid. Please re-authenticate via `agy auth` and update credentials with `ags agy add`.");
            }
            bail!("loadCodeAssist returned {}: {}", status, body);
        }

        let load_resp: AgyLoadCodeAssistResponse = load_res.json().await?;
        let project_id = load_resp.project_id();
        let tier_name = load_resp.tier_name();

        // 2. Call retrieveUserQuotaSummary
        let mut quota_body = serde_json::Map::new();
        if let Some(ref pid) = project_id {
            quota_body.insert(
                "project".to_string(),
                serde_json::Value::String(pid.clone()),
            );
        }

        let mut quota_res = client
            .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("User-Agent", &ua)
            .json(&serde_json::Value::Object(quota_body.clone()))
            .send()
            .await
            .context("Failed to send retrieveUserQuotaSummary request")?;

        if quota_res.status() == reqwest::StatusCode::UNAUTHORIZED
            && !refreshed
            && creds.token.refresh_token.is_some()
        {
            Self::refresh_token_static(&mut creds, Some(store), Some(crypto))
                .await
                .context("retrieveUserQuotaSummary returned 401 and token refresh failed. Please run `agy auth` and update with `ags agy add`")?;
            let _ = refreshed;
            access_token = creds
                .token
                .access_token
                .clone()
                .context("No access token after refresh")?;

            quota_res = client
                .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary")
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .header("User-Agent", &ua)
                .json(&serde_json::Value::Object(quota_body))
                .send()
                .await
                .context("Failed to send retrieveUserQuotaSummary retry request")?;
        }

        let summary_json = if quota_res.status().is_success() {
            quota_res.json::<serde_json::Value>().await.ok()
        } else {
            None
        };

        // 3. Fetch direct retrieveUserQuota as well (for Gemini quota or fallback)
        let quota_url = "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
        let direct_res = client
            .post(quota_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .header("User-Agent", &ua)
            .json(&serde_json::json!({}))
            .send()
            .await;

        let direct_json = if let Ok(res) = direct_res {
            if res.status().is_success() {
                res.json::<serde_json::Value>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        Ok((tier_name, summary_json, direct_json))
    }

    /// Check if token is expired or within 5 minutes of expiration
    pub fn is_token_expiring(token: &AgyToken) -> bool {
        let now = Utc::now();
        let threshold = chrono::Duration::minutes(5);

        if token.access_token.is_none() {
            return true;
        }

        if let Some(ref exp_str) = token.expiry {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(exp_str) {
                return dt.with_timezone(&Utc) - now < threshold;
            }
        }

        if let Some(exp_date) = token.expiry_date {
            let dt_opt = if exp_date > 100_000_000_000 {
                chrono::DateTime::from_timestamp_millis(exp_date)
            } else {
                chrono::DateTime::from_timestamp(exp_date, 0)
            };
            if let Some(dt) = dt_opt {
                return dt - now < threshold;
            }
        }

        false
    }

    /// Refresh Google OAuth access token using refresh token, then persist changes
    pub async fn refresh_token(&mut self, creds: &mut AgyCredentials) -> Result<()> {
        Self::refresh_token_static(creds, None, None).await
    }

    /// Perform token refresh via Google OAuth2 endpoint and persist updated credentials
    pub async fn refresh_token_static(
        creds: &mut AgyCredentials,
        custom_store: Option<&Store>,
        custom_crypto: Option<&dyn CryptoProvider>,
    ) -> Result<()> {
        let refresh_tok = creds
            .token
            .refresh_token
            .as_deref()
            .context("No refresh token available in Antigravity credentials")?;

        let client_id = unmask_oauth_credential(MASKED_CLIENT_ID);
        let client_secret = unmask_oauth_credential(MASKED_CLIENT_SECRET);

        let client = reqwest::Client::new();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_tok),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
        ];

        let resp = client
            .post("https://oauth2.googleapis.com/token")
            .header("User-Agent", "antigravity/cli")
            .form(&params)
            .send()
            .await
            .context(
                "Failed to send token refresh request to https://oauth2.googleapis.com/token",
            )?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            if body.contains("invalid_grant") || body.contains("unauthorized_client") {
                bail!(
                    "Antigravity login expired or revoked. Please run `agy auth` (or re-authenticate) and update credentials with `ags agy add`."
                );
            }
            bail!(
                "Google OAuth token refresh failed with {}: {}",
                status,
                body
            );
        }

        let token_resp: GoogleOAuthTokenResponse =
            serde_json::from_str(&body).context("Failed to parse token refresh response JSON")?;

        creds.token.access_token = Some(token_resp.access_token);

        if let Some(new_refresh) = token_resp.refresh_token {
            creds.token.refresh_token = Some(new_refresh);
        }
        if let Some(new_id_tok) = token_resp.id_token {
            if let Some(claims) = CodexJwtClaims::from_jwt(&new_id_tok) {
                if let Some(e) = claims.email {
                    creds.id_token_email = Some(e);
                }
            }
            creds.id_token = Some(new_id_tok);
        }

        let expires_in = token_resp.expires_in.unwrap_or(3600);
        creds.token.expires_in = Some(expires_in);
        if let Some(token_type) = token_resp.token_type {
            creds.token.token_type = Some(token_type);
        }
        let expiry_dt = Utc::now() + chrono::Duration::seconds(expires_in);
        creds.token.expiry = Some(expiry_dt.to_rfc3339());
        creds.token.expiry_date = Some(expiry_dt.timestamp_millis());

        // Persist updated credentials back to Vault and Keyring
        Self::persist_refreshed_creds(creds, custom_store, custom_crypto)?;

        Ok(())
    }

    fn persist_refreshed_creds(
        creds: &AgyCredentials,
        custom_store: Option<&Store>,
        custom_crypto: Option<&dyn CryptoProvider>,
    ) -> Result<()> {
        let raw = serde_json::to_vec_pretty(creds)?;
        let (key, email) = Self::parse_identity(&raw).unwrap_or_default();

        // 1. Vault persistence
        if let (Some(store), Some(crypto)) = (custom_store, custom_crypto) {
            if let Ok(reg) = store.load_registry() {
                let matched_key = reg
                    .accounts
                    .iter()
                    .find(|a| a.key == key)
                    .or_else(|| {
                        reg.accounts
                            .iter()
                            .find(|a| !email.is_empty() && a.email == email)
                    })
                    .map(|a| a.key.clone())
                    .or_else(|| reg.active.clone());

                if let Some(target_key) = matched_key {
                    let _ = store.write_account(&target_key, &raw, crypto);
                }
            }
        } else {
            let base_dir = crate::config::get_base_dir();
            let store = Store::new(base_dir.join("agy"));
            let global_cfg = crate::config::load_global_config(&base_dir).unwrap_or_default();
            let crypto =
                crate::crypto::CryptoManager::get_provider(global_cfg.crypto_provider.as_deref());
            if let Ok(reg) = store.load_registry() {
                let matched_key = reg
                    .accounts
                    .iter()
                    .find(|a| a.key == key)
                    .or_else(|| {
                        reg.accounts
                            .iter()
                            .find(|a| !email.is_empty() && a.email == email)
                    })
                    .map(|a| a.key.clone())
                    .or_else(|| reg.active.clone());

                if let Some(target_key) = matched_key {
                    let _ = store.write_account(&target_key, &raw, &*crypto);
                }
            }
        }

        // 2. Write to system keyring/fallback files if active or matching current system credentials.
        // If custom_store is provided and does not point to the real system store, skip touching host keyring.
        let base_dir = crate::config::get_base_dir();
        let is_system_store = match custom_store {
            Some(s) => s.dir() == base_dir.join("agy"),
            None => true,
        };

        if is_system_store {
            let store = Store::new(base_dir.join("agy"));
            let is_active = store
                .load_registry()
                .map(|r| {
                    r.active.as_deref() == Some(&key)
                        || r.accounts.iter().any(|a| {
                            (a.key == key || (!email.is_empty() && a.email == email))
                                && r.active.as_deref() == Some(&a.key)
                        })
                })
                .unwrap_or(false);

            let matches_system = if let Ok(sys_raw) = Self::read_credentials_raw() {
                if let Ok((sys_key, sys_email)) = Self::parse_identity(&sys_raw) {
                    sys_key == key || (!email.is_empty() && sys_email == email)
                } else {
                    false
                }
            } else {
                false
            };

            if is_active || matches_system {
                let _ = Self::write_credentials_raw(&raw);
            }
        }

        Ok(())
    }

    pub fn extract_access_token(data: &[u8]) -> Result<String> {
        let creds: AgyCreds = serde_json::from_slice(data)?;
        creds
            .token
            .access_token
            .context("No access token present in Antigravity credentials")
    }

    /// Official user agent string recognized by Cloud Code backend to return full quota groups.
    pub fn official_user_agent() -> String {
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            other => other,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => other,
        };
        format!(
            "antigravity/cli/1.11.5 (aidev_client; os_type={}; arch={}; cl=994719654; auth_method=consumer)",
            os, arch
        )
    }

    pub fn derive_agy_window_label(bucket: &serde_json::Value) -> String {
        if let Ok(dto) = serde_json::from_value::<AgyQuotaBucketDto>(bucket.clone()) {
            dto.derive_window_label()
        } else {
            let text = format!(
                "{} {} {} {}",
                bucket.get("window").and_then(|v| v.as_str()).unwrap_or(""),
                bucket
                    .get("bucketId")
                    .or_else(|| bucket.get("bucket_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                bucket
                    .get("displayName")
                    .or_else(|| bucket.get("display_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                bucket
                    .get("tokenType")
                    .or_else(|| bucket.get("token_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            )
            .to_lowercase();
            if text.contains("week") || text.contains("604800") || text.contains("wtus") {
                "Weekly Window".to_string()
            } else if text.contains("5h") || text.contains("18000") || text.contains("five") {
                "5-Hour Window".to_string()
            } else {
                "Weekly Window".to_string()
            }
        }
    }

    pub fn format_resets_in(reset_time: &str) -> String {
        if reset_time.is_empty() {
            return String::new();
        }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_time) {
            let now = Utc::now();
            let diff = dt.with_timezone(&Utc) - now;
            if diff.num_seconds() > 0 {
                let days = diff.num_days();
                let hours = diff.num_hours() % 24;
                let mins = diff.num_minutes() % 60;
                let in_str = if days > 0 {
                    if hours > 0 {
                        format!("{}d {}h", days, hours)
                    } else {
                        format!("{}d", days)
                    }
                } else if hours > 0 {
                    format!("{}h {}m", hours, mins)
                } else {
                    format!("{}m", mins.max(1))
                };
                format!("(resets in {})", in_str)
            } else {
                let local_dt = dt.with_timezone(&chrono::Local);
                format!("(resets at {})", local_dt.format("%Y-%m-%d %H:%M"))
            }
        } else {
            format!("(resets at {})", reset_time)
        }
    }

    pub fn format_group_title(raw: &str) -> String {
        let lower = raw.to_lowercase();
        if lower.contains("gemini") {
            "Gemini".to_string()
        } else if lower.contains("claude") || lower.contains("gpt") {
            "Claude & GPT".to_string()
        } else {
            let trimmed = raw.trim();
            if let Some(stripped) = trimmed.strip_suffix(" models") {
                stripped.to_string()
            } else if let Some(stripped) = trimmed.strip_suffix(" Models") {
                stripped.to_string()
            } else {
                trimmed.to_string()
            }
        }
    }

    pub fn group_order(name: &str) -> usize {
        let lower = name.to_lowercase();
        if lower.contains("gemini") {
            0
        } else if lower.contains("claude") || lower.contains("gpt") {
            1
        } else {
            2
        }
    }

    fn window_order(label: &str) -> usize {
        let lower = label.to_lowercase();
        if lower.contains("5-hour") || lower.contains("5h") {
            0
        } else if lower.contains("daily") || lower.contains("day") {
            1
        } else if lower.contains("weekly") || lower.contains("week") {
            2
        } else {
            3
        }
    }

    pub fn display_official_quota(
        tier: &str,
        summary_val: Option<&serde_json::Value>,
        direct_val: Option<&serde_json::Value>,
    ) {
        println!("{}", "Antigravity Quotas & Usage:".bold().cyan());
        Self::render_agy_account_quota(tier, summary_val, direct_val);
    }

    pub fn render_agy_account_quota(
        tier: &str,
        summary_val: Option<&serde_json::Value>,
        direct_val: Option<&serde_json::Value>,
    ) {
        if !tier.is_empty() {
            println!("  Tier: {}", tier.yellow().bold());
        }
        let groups = Self::parse_agy_quota_groups(summary_val, direct_val);
        for g in groups {
            if let Some(ref name) = g.name {
                println!("  {}", name.bold());
            }
            for q in g.quotas {
                crate::render::render_quota_line("    ", &q);
            }
        }
    }

    pub fn parse_agy_quota_groups(
        summary_val: Option<&serde_json::Value>,
        direct_val: Option<&serde_json::Value>,
    ) -> Vec<crate::model::QuotaGroup> {
        struct BucketInfo {
            label: String,
            rem_frac: f64,
            reset_time: String,
        }
        struct GroupInfo {
            name: String,
            buckets: Vec<BucketInfo>,
        }

        let mut groups: Vec<GroupInfo> = Vec::new();

        if let Some(summary) = summary_val {
            if let Some(raw_groups) = summary.get("groups").and_then(|g| g.as_array()) {
                for g in raw_groups {
                    let disp_name = g
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Models");
                    let mut buckets = Vec::new();

                    if let Some(raw_buckets) = g.get("buckets").and_then(|b| b.as_array()) {
                        for b in raw_buckets {
                            let rem_frac = match b.get("remainingFraction").and_then(|v| v.as_f64())
                            {
                                Some(f) => f,
                                None => continue,
                            };
                            let window_label = Self::derive_agy_window_label(b);
                            let reset_time = b
                                .get("resetTime")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            buckets.push(BucketInfo {
                                label: window_label,
                                rem_frac,
                                reset_time,
                            });
                        }
                    }

                    if !buckets.is_empty() {
                        let group_name = Self::format_group_title(disp_name);
                        if let Some(existing) = groups.iter_mut().find(|g| g.name == group_name) {
                            existing.buckets.extend(buckets);
                        } else {
                            groups.push(GroupInfo {
                                name: group_name,
                                buckets,
                            });
                        }
                    }
                }
            }
        }

        // 补充来自 direct_val 的配额（如 Gemini 或未由 summary 返回的模型配额）
        if let Some(direct) = direct_val {
            if let Some(direct_buckets) = direct.get("buckets").and_then(|b| b.as_array()) {
                for b in direct_buckets {
                    let mid = b
                        .get("modelId")
                        .or_else(|| b.get("model_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let lower = mid.to_lowercase();
                    if lower.starts_with("chat_")
                        || lower.starts_with("tab_")
                        || mid.trim().is_empty()
                    {
                        continue;
                    }
                    let rem_frac = match b
                        .get("remainingFraction")
                        .or_else(|| b.get("remaining_fraction"))
                        .and_then(|v| v.as_f64())
                    {
                        Some(f) => f,
                        None => continue,
                    };
                    let window_label = Self::derive_agy_window_label(b);
                    let reset_time = b
                        .get("resetTime")
                        .or_else(|| b.get("reset_time"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let group_name = Self::format_group_title(mid);

                    if let Some(existing) = groups.iter_mut().find(|g| g.name == group_name) {
                        if let Some(existing_bucket) = existing
                            .buckets
                            .iter_mut()
                            .find(|eb| eb.label == window_label)
                        {
                            if existing_bucket.reset_time.is_empty() && !reset_time.is_empty() {
                                existing_bucket.reset_time = reset_time;
                                existing_bucket.rem_frac = rem_frac;
                            }
                        } else {
                            existing.buckets.push(BucketInfo {
                                label: window_label,
                                rem_frac,
                                reset_time,
                            });
                        }
                    } else {
                        groups.push(GroupInfo {
                            name: group_name,
                            buckets: vec![BucketInfo {
                                label: window_label,
                                rem_frac,
                                reset_time,
                            }],
                        });
                    }
                }
            }
        }

        // 统一排序：Gemini 在前，Claude & GPT 次之，其他模型组依次排在后面
        groups.sort_by_key(|g| Self::group_order(&g.name));

        let mut result = Vec::new();
        for g in groups {
            let mut buckets = g.buckets;
            buckets.sort_by_key(|b| Self::window_order(&b.label));

            let mut quotas = Vec::new();
            for b in buckets {
                let rem_pct = (b.rem_frac * 100.0).clamp(0.0, 100.0);
                let used_pct = ((1.0 - b.rem_frac) * 100.0).clamp(0.0, 100.0);

                let lower_label = b.label.to_lowercase();
                let (window_type, display_label) =
                    if lower_label.contains("5-hour") || lower_label.contains("5h") {
                        ("5h".to_string(), "5h".to_string())
                    } else if lower_label.contains("daily") || lower_label.contains("day") {
                        ("daily".to_string(), "Daily".to_string())
                    } else if lower_label.contains("weekly") || lower_label.contains("week") {
                        ("weekly".to_string(), "Weekly".to_string())
                    } else if lower_label.contains("monthly") || lower_label.contains("month") {
                        ("monthly".to_string(), "Monthly".to_string())
                    } else {
                        ("window".to_string(), b.label)
                    };

                let resets_in_text = if !b.reset_time.is_empty() {
                    crate::model::format_resets_in_rfc3339(&b.reset_time)
                } else {
                    None
                };

                let reset_time = if !b.reset_time.is_empty() {
                    Some(b.reset_time)
                } else {
                    None
                };

                quotas.push(crate::model::QuotaWindow::new(
                    window_type,
                    display_label,
                    rem_pct,
                    used_pct,
                    reset_time,
                    resets_in_text,
                ));
            }

            if !quotas.is_empty() {
                result.push(crate::model::QuotaGroup {
                    name: Some(g.name),
                    quotas,
                });
            }
        }

        result
    }

    pub fn display_direct_quota(quota_val: &serde_json::Value) {
        Self::display_official_quota("", None, Some(quota_val));
    }

    pub fn display_quota_json(tier: &str, quota_val: &serde_json::Value) {
        Self::display_official_quota(tier, Some(quota_val), None);
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub enum AgyModelGroup {
    Gemini,
    ClaudeAndGpt,
    Other,
}

impl AgyModelGroup {
    pub fn label(&self) -> &'static str {
        match self {
            AgyModelGroup::Gemini => "Gemini",
            AgyModelGroup::ClaudeAndGpt => "Claude & GPT",
            AgyModelGroup::Other => "Other",
        }
    }

    pub fn order(&self) -> usize {
        match self {
            AgyModelGroup::Gemini => 0,
            AgyModelGroup::ClaudeAndGpt => 1,
            AgyModelGroup::Other => 2,
        }
    }
}

pub fn classify_agy_model(name: &str) -> Option<AgyModelGroup> {
    let lower = name.to_lowercase();
    // 忽略内部探针与补全辅助项 (如 chat_xxx, tab_xxx)
    if lower.starts_with("chat_") || lower.starts_with("tab_") {
        return None;
    }
    if lower.contains("gemini") {
        Some(AgyModelGroup::Gemini)
    } else if lower.contains("claude") || lower.contains("gpt") {
        Some(AgyModelGroup::ClaudeAndGpt)
    } else {
        Some(AgyModelGroup::Other)
    }
}

pub fn format_agy_reset_time(reset_time: &str) -> String {
    if reset_time.is_empty() {
        return String::new();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_time) {
        let now = Utc::now();
        let diff = dt.with_timezone(&Utc) - now;
        if diff.num_seconds() > 0 {
            let days = diff.num_days();
            let hours = diff.num_hours() % 24;
            let mins = diff.num_minutes() % 60;
            let in_str = if days > 0 {
                format!("{}d {}h", days, hours)
            } else if hours > 0 {
                format!("{}h {}m", hours, mins)
            } else {
                format!("{}m", mins.max(1))
            };
            format!("(resets in {})", in_str)
        } else {
            let local_dt = dt.with_timezone(&chrono::Local);
            format!("(resets at {})", local_dt.format("%Y-%m-%d %H:%M"))
        }
    } else {
        format!("(resets at {})", reset_time)
    }
}

impl AgentApp for AgyAgent {
    fn id(&self) -> &str {
        "agy"
    }

    fn display_name(&self) -> &str {
        "Antigravity"
    }

    fn read_active_raw(&self) -> Result<Vec<u8>> {
        Self::read_credentials_raw()
    }

    fn write_active_raw(&self, data: &[u8]) -> Result<()> {
        Self::write_credentials_raw(data)
    }

    fn parse_identity(&self, raw: &[u8]) -> Result<AccountIdentity> {
        let (key, email) = Self::parse_identity(raw)?;
        Ok(AccountIdentity {
            key,
            email,
            alias: None,
            plan: None,
            mode: Some("oauth".to_string()),
        })
    }

    fn add_account(
        &self,
        alias: Option<&str>,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<(Account, bool)> {
        self.add(alias, store, crypto)
    }

    fn switch_account(
        &self,
        query: &str,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<String> {
        self.switch(query, store, crypto)
    }

    fn remove_account(&self, query: &str, store: &Store) -> Result<String> {
        self.remove(query, store)
    }

    fn list_accounts(&self, store: &Store) -> Result<AccountListModel> {
        let reg = store.load_registry()?;
        let mut model = AccountListModel::new(self.id(), self.display_name());
        model.active_key = reg.active.clone();

        for (idx, acc) in reg.accounts.iter().enumerate() {
            let is_active = reg.active.as_deref() == Some(&acc.key);
            model.accounts.push(AccountDisplayItem {
                index: idx + 1,
                key: acc.key.clone(),
                alias: acc.alias.clone(),
                email: acc.email.clone(),
                plan: acc.plan.clone(),
                mode: acc.mode.clone(),
                is_active,
                added_at: Some(acc.added_at),
                store_status: Some("Stored".to_string()),
            });
        }
        Ok(model)
    }

    fn current_account(&self, store: &Store) -> Result<CurrentAccountInfo> {
        let raw = self.read_active_raw()?;
        let identity = self.parse_identity(&raw)?;
        let reg = store.load_registry()?;

        let matched = reg
            .accounts
            .iter()
            .find(|a| a.key == identity.key || a.email.eq_ignore_ascii_case(&identity.email));
        let in_registry = matched.is_some();
        let is_active = reg.active.as_deref() == Some(&identity.key);
        let store_alias = matched.and_then(|a| a.alias.clone());

        Ok(CurrentAccountInfo {
            identity,
            in_registry,
            is_active,
            store_alias,
        })
    }

    fn fetch_usage<'a>(
        &'a self,
        store: &'a Store,
        crypto: &'a dyn CryptoProvider,
    ) -> BoxFuture<'a, Result<UsageReport>> {
        Box::pin(async move { self.collect_usage_report(store, crypto).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn test_agy_identity_and_store() {
        let fake_jwt_payload = r#"{"sub":"987654321","email":"tester@gmail.com"}"#;
        let b64 = BASE64_URL_SAFE_NO_PAD.encode(fake_jwt_payload.as_bytes());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{}.fake_sig", b64);

        let creds = AgyCreds {
            token: AgyToken {
                access_token: Some("ya29.fake-token".to_string()),
                refresh_token: Some("1//fake-refresh".to_string()),
                expiry: Some("2026-10-26T15:00:00Z".to_string()),
                expiry_date: Some(1730000000),
                ..Default::default()
            },
            id_token: Some(fake_jwt),
            auth_method: Some("consumer".to_string()),
            ..Default::default()
        };

        let raw = serde_json::to_vec(&creds).unwrap();
        let (key, email) = AgyAgent::parse_identity(&raw).unwrap();
        assert_eq!(key, "google:987654321");
        assert_eq!(email, "tester@gmail.com");

        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;

        store.write_account(&key, &raw, &crypto).unwrap();
        let mut reg = store.load_registry().unwrap();
        reg.upsert(Account {
            key: key.clone(),
            alias: Some("work-google".to_string()),
            email,
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        });
        reg.active = Some(key.clone());
        store.save_registry(&reg).unwrap();

        let loaded = store.load_registry().unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.active.as_deref(), Some("google:987654321"));
    }

    #[test]
    fn test_agy_oauth_token_schema() {
        let json_data = r#"{
            "token": {
                "access_token": "ya29.sample-token",
                "token_type": "Bearer",
                "refresh_token": "1//sample-refresh",
                "expiry": "2026-10-26T15:00:00Z",
                "expires_in": 3599
            },
            "auth_method": "consumer",
            "id_token_email": "direct_user@example.com"
        }"#;

        let creds: AgyCreds = serde_json::from_str(json_data).unwrap();
        assert_eq!(
            creds.id_token_email.as_deref(),
            Some("direct_user@example.com")
        );
        assert_eq!(creds.token.token_type.as_deref(), Some("Bearer"));
        assert_eq!(creds.token.expires_in, Some(3599));

        let (key, email) = AgyAgent::parse_identity(json_data.as_bytes()).unwrap();
        assert_eq!(key, "google:direct_user@example.com");
        assert_eq!(email, "direct_user@example.com");
    }

    #[test]
    fn test_unmask_oauth_credentials() {
        let client_id = unmask_oauth_credential(MASKED_CLIENT_ID);
        assert_eq!(
            client_id,
            "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com"
        );

        let client_secret = unmask_oauth_credential(MASKED_CLIENT_SECRET);
        assert_eq!(client_secret.len(), 35);
        assert!(client_secret.starts_with("GOCSPX-"));
    }

    #[test]
    fn test_is_token_expiring() {
        // 1. Missing access_token is expiring
        let empty_tok = AgyToken::default();
        assert!(AgyAgent::is_token_expiring(&empty_tok));

        // 2. Far future expiry is not expiring
        let future_tok = AgyToken {
            access_token: Some("ya29.good".to_string()),
            refresh_token: Some("1//rt".to_string()),
            expiry: Some((Utc::now() + chrono::Duration::hours(2)).to_rfc3339()),
            expiry_date: None,
            ..Default::default()
        };
        assert!(!AgyAgent::is_token_expiring(&future_tok));

        // 3. Expired token is expiring
        let expired_tok = AgyToken {
            access_token: Some("ya29.expired".to_string()),
            refresh_token: Some("1//rt".to_string()),
            expiry: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
            expiry_date: None,
            ..Default::default()
        };
        assert!(AgyAgent::is_token_expiring(&expired_tok));

        // 4. Token expiring in 2 minutes (< 5 minutes threshold) is expiring
        let soon_expiring = AgyToken {
            access_token: Some("ya29.soon".to_string()),
            refresh_token: Some("1//rt".to_string()),
            expiry: Some((Utc::now() + chrono::Duration::minutes(2)).to_rfc3339()),
            expiry_date: None,
            ..Default::default()
        };
        assert!(AgyAgent::is_token_expiring(&soon_expiring));

        // 5. Expiry date in milliseconds
        let ms_future = AgyToken {
            access_token: Some("ya29.ms_good".to_string()),
            refresh_token: Some("1//rt".to_string()),
            expiry: None,
            expiry_date: Some((Utc::now() + chrono::Duration::hours(1)).timestamp_millis()),
            ..Default::default()
        };
        assert!(!AgyAgent::is_token_expiring(&ms_future));

        let ms_expired = AgyToken {
            access_token: Some("ya29.ms_bad".to_string()),
            refresh_token: Some("1//rt".to_string()),
            expiry: None,
            expiry_date: Some((Utc::now() - chrono::Duration::minutes(1)).timestamp_millis()),
            ..Default::default()
        };
        assert!(AgyAgent::is_token_expiring(&ms_expired));
    }

    #[test]
    fn test_persist_refreshed_creds() {
        let fake_jwt_payload = r#"{"sub":"refresh_user_123","email":"refresh@gmail.com"}"#;
        let b64 = BASE64_URL_SAFE_NO_PAD.encode(fake_jwt_payload.as_bytes());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{}.fake_sig", b64);

        let mut creds = AgyCreds {
            token: AgyToken {
                access_token: Some("ya29.old-token".to_string()),
                refresh_token: Some("1//old-refresh".to_string()),
                expiry: Some("2026-01-01T00:00:00Z".to_string()),
                expiry_date: Some(1730000000),
                ..Default::default()
            },
            id_token: Some(fake_jwt),
            auth_method: Some("consumer".to_string()),
            ..Default::default()
        };

        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;
        let key = "google:refresh_user_123";

        // Setup store with initial account
        let raw = serde_json::to_vec(&creds).unwrap();
        store.write_account(key, &raw, &crypto).unwrap();
        let mut reg = store.load_registry().unwrap();
        reg.upsert(Account {
            key: key.to_string(),
            alias: Some("refresh-account".to_string()),
            email: "refresh@gmail.com".to_string(),
            plan: Some("Standard".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        });
        reg.active = Some(key.to_string());
        store.save_registry(&reg).unwrap();

        // Mutate creds with refreshed token and persist
        creds.token.access_token = Some("ya29.new-refreshed-token".to_string());
        AgyAgent::persist_refreshed_creds(&creds, Some(&store), Some(&crypto)).unwrap();

        // Read back from store and verify updated access_token
        let updated_raw = store.read_account(key, &crypto).unwrap();
        let updated_creds: AgyCreds = serde_json::from_slice(&updated_raw).unwrap();
        assert_eq!(
            updated_creds.token.access_token.as_deref(),
            Some("ya29.new-refreshed-token")
        );
    }

    #[tokio::test]
    async fn test_refresh_token_missing_refresh_token_fails() {
        let mut agent = AgyAgent::new();
        let mut creds = AgyCreds {
            token: AgyToken {
                access_token: Some("ya29.old".to_string()),
                refresh_token: None,
                expiry: None,
                expiry_date: None,
                ..Default::default()
            },
            id_token: None,
            auth_method: None,
            ..Default::default()
        };
        let res = agent.refresh_token(&mut creds).await;
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("No refresh token available"));
    }

    #[test]
    fn test_agy_model_classification_and_filtering() {
        // Internal telemetry / completion models should be ignored
        assert_eq!(classify_agy_model("chat_20706"), None);
        assert_eq!(classify_agy_model("chat_23310"), None);
        assert_eq!(classify_agy_model("tab_flash_lite_preview"), None);
        assert_eq!(classify_agy_model("tab_jump_flash_lite_preview"), None);

        // Core model groups
        assert_eq!(
            classify_agy_model("claude-sonnet-4-6"),
            Some(AgyModelGroup::ClaudeAndGpt)
        );
        assert_eq!(
            classify_agy_model("claude-opus-4-6-thinking"),
            Some(AgyModelGroup::ClaudeAndGpt)
        );
        assert_eq!(
            classify_agy_model("gemini-2.5-pro"),
            Some(AgyModelGroup::Gemini)
        );
        assert_eq!(
            classify_agy_model("gemini-3.5-flash-lite"),
            Some(AgyModelGroup::Gemini)
        );
        assert_eq!(
            classify_agy_model("gpt-oss-120b-medium"),
            Some(AgyModelGroup::ClaudeAndGpt)
        );
        assert_eq!(
            classify_agy_model("custom-finetune"),
            Some(AgyModelGroup::Other)
        );
    }

    #[test]
    fn test_format_group_title() {
        assert_eq!(AgyAgent::format_group_title("Gemini models"), "Gemini");
        assert_eq!(
            AgyAgent::format_group_title("Claude and GPT models"),
            "Claude & GPT"
        );
        assert_eq!(
            AgyAgent::format_group_title("Claude models"),
            "Claude & GPT"
        );
        assert_eq!(AgyAgent::format_group_title("GPT models"), "Claude & GPT");
        assert_eq!(
            AgyAgent::format_group_title("Custom Finetune models"),
            "Custom Finetune"
        );
    }

    #[test]
    fn test_derive_agy_window_label() {
        let b_5h = serde_json::json!({ "window": "5h" });
        assert_eq!(AgyAgent::derive_agy_window_label(&b_5h), "5-Hour Window");

        let b_18k = serde_json::json!({ "window": "18000s" });
        assert_eq!(AgyAgent::derive_agy_window_label(&b_18k), "5-Hour Window");

        let b_weekly = serde_json::json!({ "window": "weekly" });
        assert_eq!(
            AgyAgent::derive_agy_window_label(&b_weekly),
            "Weekly Window"
        );

        let b_id = serde_json::json!({ "bucketId": "3p-weekly" });
        assert_eq!(AgyAgent::derive_agy_window_label(&b_id), "Weekly Window");
    }

    #[test]
    fn test_agy_display_official_quota_render() {
        let summary_sample = serde_json::json!({
            "groups": [
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-weekly",
                            "window": "weekly",
                            "remainingFraction": 0.0,
                            "resetTime": "2026-11-01T14:32:00Z"
                        },
                        {
                            "bucketId": "3p-5h",
                            "window": "5h",
                            "remainingFraction": 0.015,
                            "resetTime": "2026-10-26T15:32:00Z"
                        }
                    ]
                }
            ]
        });

        let direct_sample = serde_json::json!({
            "buckets": [
                {
                    "modelId": "gemini-2.5-pro",
                    "remainingFraction": 0.926,
                    "resetTime": "2026-10-26T18:45:00Z"
                }
            ]
        });

        // Test display_official_quota without panic
        AgyAgent::display_official_quota(
            "Google AI Pro",
            Some(&summary_sample),
            Some(&direct_sample),
        );
    }

    #[test]
    fn test_parse_agy_quota_groups_gemini_5h_and_weekly_correct_order_and_values() {
        let now = Utc::now();
        let rt_weekly = (now + chrono::Duration::hours(118)).to_rfc3339(); // 4d 22h
        let rt_5h = (now + chrono::Duration::hours(2) + chrono::Duration::minutes(43)).to_rfc3339(); // 2h 43m

        // Direct buckets with 19.2% Weekly and 65.0% 5h
        let direct_json = serde_json::json!({
            "buckets": [
                {
                    "modelId": "gemini-2.5-pro",
                    "remainingFraction": 0.192,
                    "resetTime": rt_weekly,
                    "window": "weekly"
                },
                {
                    "modelId": "gemini-2.5-flash",
                    "remainingFraction": 0.650,
                    "resetTime": rt_5h,
                    "window": "5h"
                }
            ]
        });

        let groups = AgyAgent::parse_agy_quota_groups(None, Some(&direct_json));
        assert_eq!(groups.len(), 1);
        let gemini_group = &groups[0];
        assert_eq!(gemini_group.name.as_deref(), Some("Gemini"));
        assert_eq!(gemini_group.quotas.len(), 2);

        // 5h must be the first line (index 0)
        let q_5h = &gemini_group.quotas[0];
        assert_eq!(q_5h.window_type, "5h");
        assert_eq!(q_5h.display_label, "5h");
        assert!((q_5h.remaining_percent - 65.0).abs() < 0.1);
        assert!(q_5h.resets_in_text.as_ref().unwrap().contains("2h"));

        // Weekly must be the second line (index 1)
        let q_weekly = &gemini_group.quotas[1];
        assert_eq!(q_weekly.window_type, "weekly");
        assert_eq!(q_weekly.display_label, "Weekly");
        assert!((q_weekly.remaining_percent - 19.2).abs() < 0.1);
        assert!(q_weekly.resets_in_text.as_ref().unwrap().contains("4d"));
    }

    #[test]
    fn test_parse_agy_quota_groups_wtus_classified_as_weekly_even_with_short_reset_time() {
        let now = Utc::now();
        // Reset in 8h 30m (less than 24h, would previously be falsely labeled as Daily)
        let rt_wtus =
            (now + chrono::Duration::hours(8) + chrono::Duration::minutes(30)).to_rfc3339();

        let direct_json = serde_json::json!({
            "buckets": [
                {
                    "modelId": "gemini-2.5-flash",
                    "remainingFraction": 0.1883,
                    "resetTime": rt_wtus,
                    "tokenType": "WTUS"
                }
            ]
        });

        let groups = AgyAgent::parse_agy_quota_groups(None, Some(&direct_json));
        assert_eq!(groups.len(), 1);
        let gemini_group = &groups[0];
        assert_eq!(gemini_group.name.as_deref(), Some("Gemini"));
        assert_eq!(gemini_group.quotas.len(), 1);

        let q = &gemini_group.quotas[0];
        assert_eq!(q.window_type, "weekly");
        assert_eq!(q.display_label, "Weekly");
        assert!((q.remaining_percent - 18.8).abs() < 0.1);
        assert!(q.resets_in_text.as_ref().unwrap().contains("8h"));
    }

    #[test]
    fn test_parse_agy_quota_groups_full_official_summary() {
        let summary = serde_json::json!({
            "groups": [
                {
                    "displayName": "Gemini Models",
                    "buckets": [
                        {
                            "bucketId": "gemini-weekly",
                            "displayName": "Weekly Limit Remaining",
                            "window": "weekly",
                            "remainingFraction": 0.1883,
                            "resetTime": "2026-10-27T12:59:29Z"
                        },
                        {
                            "bucketId": "gemini-5h",
                            "displayName": "Five Hour Limit Remaining",
                            "window": "5h",
                            "remainingFraction": 1.0,
                            "resetTime": "2026-10-27T09:40:25Z"
                        }
                    ]
                },
                {
                    "displayName": "Claude and GPT models",
                    "buckets": [
                        {
                            "bucketId": "3p-5h",
                            "displayName": "Five Hour Limit Remaining",
                            "window": "5h",
                            "remainingFraction": 1.0,
                            "resetTime": "2026-10-27T09:40:25Z"
                        },
                        {
                            "bucketId": "3p-weekly",
                            "displayName": "Weekly Limit Remaining",
                            "window": "weekly",
                            "remainingFraction": 1.0,
                            "resetTime": "2026-11-03T04:40:25Z"
                        }
                    ]
                }
            ]
        });

        let groups = AgyAgent::parse_agy_quota_groups(Some(&summary), None);
        assert_eq!(groups.len(), 2);

        // Group 0: Gemini
        assert_eq!(groups[0].name.as_deref(), Some("Gemini"));
        assert_eq!(groups[0].quotas.len(), 2);
        assert_eq!(groups[0].quotas[0].display_label, "5h");
        assert_eq!(groups[0].quotas[0].remaining_percent, 100.0);
        assert_eq!(groups[0].quotas[1].display_label, "Weekly");
        assert!((groups[0].quotas[1].remaining_percent - 18.8).abs() < 0.1);

        // Group 1: Claude & GPT
        assert_eq!(groups[1].name.as_deref(), Some("Claude & GPT"));
        assert_eq!(groups[1].quotas.len(), 2);
        assert_eq!(groups[1].quotas[0].display_label, "5h");
        assert_eq!(groups[1].quotas[1].display_label, "Weekly");
    }

    #[test]
    fn test_agy_add_deduplication() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;
        let agent = AgyAgent::new();

        let fake_jwt_payload = r#"{"sub":"dedup_user_1","email":"dedup@example.com"}"#;
        let b64 = BASE64_URL_SAFE_NO_PAD.encode(fake_jwt_payload.as_bytes());
        let fake_jwt = format!("eyJhbGciOiJub25lIn0.{}.fake_sig", b64);

        let creds = AgyCreds {
            token: AgyToken {
                access_token: Some("ya29.initial_token".to_string()),
                refresh_token: Some("1//rt".to_string()),
                expiry: Some((Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
                ..Default::default()
            },
            id_token: Some(fake_jwt),
            auth_method: Some("consumer".to_string()),
            id_token_email: Some("dedup@example.com".to_string()),
        };

        let raw_json = serde_json::to_string(&creds).unwrap();
        let encoded = BASE64_STANDARD.encode(raw_json.as_bytes());
        std::env::set_var("AGY_KEYRING_DATA", &encoded);

        // 1. Initial add
        let (acc1, is_update1) = agent.add(Some("first-alias"), &store, &crypto).unwrap();
        assert!(!is_update1);
        assert_eq!(acc1.email, "dedup@example.com");
        assert_eq!(acc1.alias.as_deref(), Some("first-alias"));

        let reg1 = store.load_registry().unwrap();
        assert_eq!(reg1.accounts.len(), 1);
        assert_eq!(reg1.active.as_deref(), Some("google:dedup_user_1"));

        // 2. Duplicate add with new alias -> should overwrite existing
        let (acc2, is_update2) = agent.add(Some("second-alias"), &store, &crypto).unwrap();
        assert!(is_update2);
        assert_eq!(acc2.email, "dedup@example.com");
        assert_eq!(acc2.alias.as_deref(), Some("second-alias"));

        let reg2 = store.load_registry().unwrap();
        assert_eq!(reg2.accounts.len(), 1);
        assert_eq!(reg2.accounts[0].alias.as_deref(), Some("second-alias"));
        assert!(reg2.accounts[0].updated_at.is_some());

        // Clean up env
        std::env::remove_var("AGY_KEYRING_DATA");
    }

    #[tokio::test]
    async fn test_agy_usage_empty_registry() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;
        let agent = AgyAgent::new();

        // Must succeed without errors on empty registry
        let res = agent.usage(&store, &crypto).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_agy_usage_multiple_accounts_handles_errors_gracefully() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;
        let agent = AgyAgent::new();

        let mut reg = store.load_registry().unwrap();
        let acc1 = Account {
            key: "google:agy-user-1".to_string(),
            alias: Some("account-1".to_string()),
            email: "user1@example.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };
        let acc2 = Account {
            key: "google:agy-user-2".to_string(),
            alias: None,
            email: "user2@example.com".to_string(),
            plan: Some("Standard".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };
        reg.accounts.push(acc1);
        reg.accounts.push(acc2);
        reg.active = Some("google:agy-user-1".to_string());
        store.save_registry(&reg).unwrap();

        // Write invalid data for user-2
        store
            .write_account("google:agy-user-2", b"invalid-data", &crypto)
            .unwrap();

        // Must succeed without terminating early
        let res = agent.usage(&store, &crypto).await;
        assert!(res.is_ok());
    }

    use crate::util::TEST_ENV_LOCK as ENV_LOCK;

    struct EnvGuard {
        saved_antigravity_home: Option<String>,
        saved_agy_home: Option<String>,
        saved_home: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                saved_antigravity_home: env::var("ANTIGRAVITY_HOME").ok(),
                saved_agy_home: env::var("AGY_HOME").ok(),
                saved_home: env::var("HOME").ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.saved_antigravity_home {
                Some(val) => unsafe { env::set_var("ANTIGRAVITY_HOME", val) },
                None => unsafe { env::remove_var("ANTIGRAVITY_HOME") },
            }
            match &self.saved_agy_home {
                Some(val) => unsafe { env::set_var("AGY_HOME", val) },
                None => unsafe { env::remove_var("AGY_HOME") },
            }
            match &self.saved_home {
                Some(val) => unsafe { env::set_var("HOME", val) },
                None => unsafe { env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn test_agy_paths_priorities() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        unsafe {
            env::remove_var("ANTIGRAVITY_HOME");
            env::remove_var("AGY_HOME");
        }

        // Priority 3: standard home
        let d = default_dir();
        assert!(d.ends_with(Path::new(".gemini").join("antigravity-cli")));
        assert_eq!(default_token_path(), d.join("antigravity-oauth-token"));
        assert!(default_creds_path().ends_with(Path::new(".gemini").join("oauth_creds.json")));
        assert_eq!(AgyAgent::default_token_path(), default_token_path());
        assert_eq!(AgyAgent::default_creds_path(), default_creds_path());

        // Priority 2: AGY_HOME overrides standard home
        unsafe {
            env::set_var("AGY_HOME", "   /tmp/custom-agy   ");
        }
        assert_eq!(default_dir(), PathBuf::from("/tmp/custom-agy"));
        assert_eq!(
            default_token_path(),
            PathBuf::from("/tmp/custom-agy/antigravity-oauth-token")
        );
        assert_eq!(
            default_creds_path(),
            PathBuf::from("/tmp/custom-agy/oauth_creds.json")
        );

        // Priority 1: ANTIGRAVITY_HOME takes highest precedence
        unsafe {
            env::set_var("ANTIGRAVITY_HOME", "   /tmp/custom-antigravity   ");
        }
        assert_eq!(default_dir(), PathBuf::from("/tmp/custom-antigravity"));
        assert_eq!(
            default_token_path(),
            PathBuf::from("/tmp/custom-antigravity/antigravity-oauth-token")
        );
        assert_eq!(
            default_creds_path(),
            PathBuf::from("/tmp/custom-antigravity/oauth_creds.json")
        );
    }

    #[test]
    fn test_agy_agent_app_trait_implementation() {
        let agent = AgyAgent::new();
        assert_eq!(agent.id(), "agy");
        assert_eq!(agent.display_name(), "Antigravity");

        let fake_jwt_payload = r#"{"sub":"sub-agy-1","email":"agy-user@gmail.com"}"#;
        let header = BASE64_URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = BASE64_URL_SAFE_NO_PAD.encode(fake_jwt_payload.as_bytes());
        let fake_id_token = format!("{}.{}.sig", header, payload);

        let creds_json = serde_json::json!({
            "token": {
                "access_token": "ya29.sample",
                "refresh_token": "1//refresh"
            },
            "id_token": fake_id_token
        });

        let raw_bytes = serde_json::to_vec(&creds_json).unwrap();
        let identity = agent.parse_identity(&raw_bytes).unwrap();
        assert_eq!(identity.key, "google:sub-agy-1");
        assert_eq!(identity.email, "agy-user@gmail.com");
        assert_eq!(identity.mode.as_deref(), Some("oauth"));

        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("store"));
        let list_model = agent.list_accounts(&store).unwrap();
        assert_eq!(list_model.app_id, "agy");
        assert!(list_model.is_empty());
    }
}
