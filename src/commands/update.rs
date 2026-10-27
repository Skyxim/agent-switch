use anyhow::{Context, Result};
use colored::*;
use std::cmp::Ordering;
use std::io::Read;
use std::path::Path;

#[derive(clap::Args, Debug, Clone)]
pub struct UpdateArgs {
    /// 仅检查是否有新版本，不执行下载与安装
    #[arg(long)]
    pub check: bool,

    /// 强制更新（即使本地版本与远端一致）
    #[arg(short = 'f', long = "force")]
    pub force: bool,

    /// 指定更新到某一特定 Release tag（可选，默认最新版本）
    #[arg(long)]
    pub tag: Option<String>,
}

/// 程序安装来源类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// 通过 Homebrew 包管理器安装（路径包含 cellar/、homebrew、/opt/homebrew 或指向此类目录的符号链接）
    Homebrew,
    /// 通过 Cargo 安装（路径包含 .cargo/bin）
    Cargo,
    /// 本地源码开发构建（路径包含 /target/debug/ 或 /target/release/）
    DevBuild,
    /// 独立二进制或通过官方脚本安装在标准路径（如 /usr/local/bin、~/.local/bin、$LOCALAPPDATA/Programs 等）
    Standalone,
}

impl std::fmt::Display for InstallSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallSource::Homebrew => write!(f, "Homebrew"),
            InstallSource::Cargo => write!(f, "Cargo"),
            InstallSource::DevBuild => write!(f, "DevBuild"),
            InstallSource::Standalone => write!(f, "Standalone"),
        }
    }
}

fn path_matches_source(s: &str) -> Option<InstallSource> {
    // 1. 本地开发构建路径特征：/target/debug/ 或 /target/release/
    if s.contains("/target/debug")
        || s.starts_with("target/debug")
        || s.contains("/target/release")
        || s.starts_with("target/release")
        || (s.contains("/target/") && (s.contains("/debug") || s.contains("/release")))
    {
        return Some(InstallSource::DevBuild);
    }

    // 2. Cargo 默认安装路径特征：.cargo/bin
    if s.contains(".cargo/bin") {
        return Some(InstallSource::Cargo);
    }

    // 3. Homebrew 路径特征：cellar/、homebrew、/opt/homebrew、linuxbrew
    if s.contains("cellar/")
        || s.contains("homebrew")
        || s.contains("/opt/homebrew")
        || s.contains("linuxbrew")
    {
        return Some(InstallSource::Homebrew);
    }

    None
}

/// 检测可执行文件的安装来源
pub fn detect_install_source(exe_path: &Path) -> InstallSource {
    // 优先尝试解析符号链接（Homebrew 常将 bin/ags 软链接到 Cellar/...）
    if let Ok(canonical) = std::fs::canonicalize(exe_path) {
        let canon_str = canonical
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        if let Some(src) = path_matches_source(&canon_str) {
            return src;
        }
    }

    // 检查原始传入路径
    let orig_str = exe_path.to_string_lossy().replace('\\', "/").to_lowercase();
    if let Some(src) = path_matches_source(&orig_str) {
        return src;
    }

    InstallSource::Standalone
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: Option<u64>,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub struct GithubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: Option<String>,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

/// 简易语义化版本结构，用于版本高低对比
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SimpleVersion {
    pub numbers: Vec<u64>,
    pub pre: Option<String>,
}

impl SimpleVersion {
    pub fn parse(s: &str) -> Result<Self> {
        let trimmed = s.trim().trim_start_matches(['v', 'V']);
        if trimmed.is_empty() {
            anyhow::bail!("版本字符串为空");
        }

        // 去除构建元数据 (+build.123)
        let core_and_pre = match trimmed.split_once('+') {
            Some((head, _)) => head,
            None => trimmed,
        };

        let (core, pre) = match core_and_pre.split_once('-') {
            Some((core_str, pre_str)) => (core_str, Some(pre_str.to_string())),
            None => (core_and_pre, None),
        };

        let numbers: Result<Vec<u64>, _> = core
            .split('.')
            .map(|part| {
                part.parse::<u64>()
                    .with_context(|| format!("解析版本号段 '{}' 失败", part))
            })
            .collect();
        let numbers = numbers?;

        if numbers.is_empty() {
            anyhow::bail!("版本核心号段不能为空");
        }

        Ok(Self { numbers, pre })
    }
}

impl PartialOrd for SimpleVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SimpleVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let max_len = self.numbers.len().max(other.numbers.len());
        for i in 0..max_len {
            let a = self.numbers.get(i).copied().unwrap_or(0);
            let b = other.numbers.get(i).copied().unwrap_or(0);
            if a != b {
                return a.cmp(&b);
            }
        }

        // 数值段完全相等时，比较先行预发标识 (pre-release)
        // 规则：正式版本（无 pre-release）高于预览版本（有 pre-release）
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

/// 对比两个版本号字符串（支持前导 v/V）
/// 返回 Ordering：v1.cmp(v2)
pub fn compare_versions(v1: &str, v2: &str) -> Result<Ordering> {
    let ver1 = SimpleVersion::parse(v1)?;
    let ver2 = SimpleVersion::parse(v2)?;
    Ok(ver1.cmp(&ver2))
}

/// 根据操作系统与架构获取标准 Target Triple
pub fn get_target_for(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => anyhow::bail!("当前平台暂不支持自动更新: {}/{}", os, arch),
    }
}

/// 获取当前运行环境的目标平台 Triple
pub fn current_target() -> Result<&'static str> {
    get_target_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// 在 Release 资产列表中查找匹配目标平台的构建资产
pub fn find_matching_asset<'a>(
    assets: &'a [ReleaseAsset],
    target: &str,
) -> Option<&'a ReleaseAsset> {
    // 过滤掉校验和及签名等非可执行资产
    let candidates: Vec<&ReleaseAsset> = assets
        .iter()
        .filter(|a| {
            let lower = a.name.to_lowercase();
            !lower.ends_with(".sha256")
                && !lower.ends_with(".sha512")
                && !lower.ends_with(".md5")
                && !lower.ends_with(".sig")
                && !lower.ends_with(".asc")
                && !lower.ends_with(".txt")
        })
        .collect();

    // 1. 完全匹配 target 标识
    if let Some(asset) = candidates.iter().find(|a| a.name.contains(target)) {
        return Some(asset);
    }

    // 2. Linux 平台 musl 与 gnu 互为回退兜底
    if target.contains("unknown-linux-musl") {
        let gnu_target = target.replace("unknown-linux-musl", "unknown-linux-gnu");
        if let Some(asset) = candidates.iter().find(|a| a.name.contains(&gnu_target)) {
            return Some(asset);
        }
    } else if target.contains("unknown-linux-gnu") {
        let musl_target = target.replace("unknown-linux-gnu", "unknown-linux-musl");
        if let Some(asset) = candidates.iter().find(|a| a.name.contains(&musl_target)) {
            return Some(asset);
        }
    }

    None
}

/// 从下载的数据中提取可执行二进制内容（支持 .tar.gz、.zip 及原始二进制）
pub fn extract_binary(asset_name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let lower_name = asset_name.to_lowercase();
    if lower_name.ends_with(".tar.gz") || lower_name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(data);
        let mut archive = tar::Archive::new(gz);
        for entry_res in archive.entries().context("读取 tar 归档条目失败")? {
            let mut entry = entry_res.context("读取 tar 条目失败")?;
            let is_file = entry.header().entry_type().is_file();
            let path = entry.path().context("获取 tar 文件路径失败")?;
            let is_match = is_file
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|name| name == "ags" || name == "ags.exe")
                    .unwrap_or(false);

            if is_match {
                let mut buf = Vec::new();
                entry
                    .read_to_end(&mut buf)
                    .context("读取归档中二进制失败")?;
                return Ok(buf);
            }
        }
        anyhow::bail!(
            "在归档文件 '{}' 中未找到可执行文件 'ags' 或 'ags.exe'",
            asset_name
        );
    } else if lower_name.ends_with(".zip") {
        let cursor = std::io::Cursor::new(data);
        let mut zip = zip::ZipArchive::new(cursor).context("解析 zip 归档失败")?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).context("读取 zip 条目失败")?;
            if file.is_file() {
                let name = file.name();
                let is_match = Path::new(name)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|base| base == "ags" || base == "ags.exe")
                    .unwrap_or(false);

                if is_match {
                    let mut buf = Vec::new();
                    file.read_to_end(&mut buf)
                        .context("读取 zip 中二进制失败")?;
                    return Ok(buf);
                }
            }
        }
        anyhow::bail!(
            "在 zip 归档 '{}' 中未找到可执行文件 'ags' 或 'ags.exe'",
            asset_name
        );
    } else {
        // 原始裸二进制
        Ok(data.to_vec())
    }
}

/// 执行文件替换（在目标文件所在同级目录创建临时文件，赋予权限后原子替换）
pub fn replace_executable(target_path: &Path, binary_bytes: &[u8]) -> Result<()> {
    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法获取可执行文件所在目录"))?;

    let temp_file_path = parent.join(format!(".ags-update-tmp.{}", rand::random::<u32>()));

    std::fs::write(&temp_file_path, binary_bytes)
        .with_context(|| format!("写入临时更新文件 {} 失败", temp_file_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(&temp_file_path, perms) {
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e).context("设置更新文件可执行权限 (0755) 失败");
        }

        if let Err(e) = std::fs::rename(&temp_file_path, target_path) {
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e).with_context(|| {
                format!(
                    "原子替换可执行文件 '{}' 失败 (若安装在受限系统目录，请尝试使用 sudo 重新运行)",
                    target_path.display()
                )
            });
        }
    }

    #[cfg(windows)]
    {
        let old_exe = target_path.with_extension("old");
        if old_exe.exists() {
            let _ = std::fs::remove_file(&old_exe);
        }

        if let Err(e) = std::fs::rename(target_path, &old_exe) {
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e)
                .with_context(|| format!("重命名旧版本文件至 {} 失败", old_exe.display()));
        }

        if let Err(e) = std::fs::rename(&temp_file_path, target_path) {
            let _ = std::fs::rename(&old_exe, target_path);
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e).with_context(|| "替换新版本文件失败，已尝试恢复原有版本");
        }

        // 尝试清理 .old 文件，若被系统锁定则静默忽略
        let _ = std::fs::remove_file(&old_exe);
    }

    #[cfg(not(any(unix, windows)))]
    {
        std::fs::rename(&temp_file_path, target_path)?;
    }

    Ok(())
}

/// 请求 GitHub API 获取 Release 信息
pub async fn fetch_github_release(
    client: &reqwest::Client,
    tag: Option<&str>,
) -> Result<GithubRelease> {
    let url = match tag {
        Some(t) => {
            let trimmed = t.trim();
            format!(
                "https://api.github.com/repos/Skyxim/agent-switch/releases/tags/{}",
                trimmed
            )
        }
        None => "https://api.github.com/repos/Skyxim/agent-switch/releases/latest".to_string(),
    };

    let mut req = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github.v3+json");

    if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        let token = token.trim();
        if !token.is_empty() {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
        }
    }

    let resp = req.send().await.context("连接 GitHub Release API 失败")?;
    let status = resp.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        // 如果指定 tag 遇到 404，尝试切换是否带 'v' 前缀重试
        if let Some(t) = tag {
            let alt_tag = if let Some(stripped) = t.strip_prefix('v') {
                stripped.to_string()
            } else {
                format!("v{}", t)
            };
            let alt_url = format!(
                "https://api.github.com/repos/Skyxim/agent-switch/releases/tags/{}",
                alt_tag
            );
            let mut alt_req = client
                .get(&alt_url)
                .header(reqwest::header::ACCEPT, "application/vnd.github.v3+json");
            if let Ok(token) = std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN"))
            {
                let token = token.trim();
                if !token.is_empty() {
                    alt_req =
                        alt_req.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
                }
            }
            if let Ok(alt_resp) = alt_req.send().await {
                if alt_resp.status().is_success() {
                    return alt_resp
                        .json::<GithubRelease>()
                        .await
                        .context("解析 GitHub Release 响应失败");
                }
            }
        }
        anyhow::bail!("未找到指定的 Release 版本 (HTTP 404)。请确认该版本是否已发布。");
    }

    if status == reqwest::StatusCode::FORBIDDEN {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "GitHub API 请求受限 (HTTP 403): {}。如触发频控，可设置 GITHUB_TOKEN 环境变量。",
            body
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("请求 GitHub Release API 失败 (HTTP {}): {}", status, body);
    }

    resp.json::<GithubRelease>()
        .await
        .context("解析 GitHub Release 响应失败")
}

/// 执行 update 主流程
pub async fn handle_update(args: UpdateArgs) -> Result<()> {
    let current_ver = env!("CARGO_PKG_VERSION");
    let target = current_target()?;
    let current_exe = std::env::current_exe().context("无法获取当前运行程序路径")?;
    let source = detect_install_source(&current_exe);

    println!("{} 正在检查 GitHub Releases 最新版本...", "→".cyan().bold());

    let client = reqwest::Client::builder()
        .user_agent(format!("agent-switch/{}", current_ver))
        .build()
        .context("构建 HTTP 客户端失败")?;

    let release = fetch_github_release(&client, args.tag.as_deref()).await?;
    let remote_tag = release.tag_name.trim();
    let cmp = compare_versions(current_ver, remote_tag)?;

    let display_current = format!("v{}", current_ver);
    let display_remote = if remote_tag.starts_with('v') || remote_tag.starts_with('V') {
        remote_tag.to_string()
    } else {
        format!("v{}", remote_tag)
    };

    if args.check {
        println!();
        println!(
            "{}",
            "=== agent-switch (ags) 版本检查报告 ===".bold().cyan()
        );
        println!("当前本地版本: {}", display_current.yellow());
        println!("远端目标版本: {}", display_remote.green().bold());
        match cmp {
            Ordering::Less => {
                println!("版本状态:     {}", "发现新版本，建议更新！".green().bold());
                println!(
                    "更新命令:     运行 {} 执行自动更新",
                    "ags update".cyan().bold()
                );
            }
            Ordering::Equal => {
                println!("版本状态:     {}", "当前已是最新版本".green());
            }
            Ordering::Greater => {
                println!(
                    "版本状态:     {}",
                    "本地版本高于远端 Release (开发构建)".yellow()
                );
            }
        }

        if !args.force {
            match source {
                InstallSource::Homebrew => {
                    println!();
                    println!(
                        "{}",
                        "提示: 当前通过 Homebrew 安装，可使用 brew update && brew upgrade agent-switch 进行更新"
                            .yellow()
                    );
                }
                InstallSource::Cargo => {
                    println!();
                    println!(
                        "{}",
                        "提示: 当前通过 Cargo 安装，可使用 cargo install --force agent-switch 进行更新"
                            .yellow()
                    );
                }
                InstallSource::DevBuild => {
                    println!();
                    println!(
                        "{}",
                        "提示: 当前检测到本地开发版本，建议通过源码编译或加 -f/--force 覆盖"
                            .yellow()
                    );
                }
                InstallSource::Standalone => {}
            }
        }

        return Ok(());
    }

    if !args.force {
        match source {
            InstallSource::Homebrew => {
                println!(
                    "{} 目标版本: {} (当前版本: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} 当前通过 Homebrew 安装，请使用以下命令更新：\n  {}\n若要强制通过 GitHub Releases 覆盖自替换二进制，请使用 {}。",
                    "i".cyan().bold(),
                    "brew update && brew upgrade agent-switch".green().bold(),
                    "ags update --force".cyan().bold()
                );
                return Ok(());
            }
            InstallSource::Cargo => {
                println!(
                    "{} 目标版本: {} (当前版本: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} 当前通过 Cargo 安装，请使用以下命令更新：\n  {}\n若要强制通过 GitHub Releases 覆盖自替换二进制，请使用 {}。",
                    "i".cyan().bold(),
                    "cargo install --force agent-switch".green().bold(),
                    "ags update --force".cyan().bold()
                );
                return Ok(());
            }
            InstallSource::DevBuild => {
                println!(
                    "{} 目标版本: {} (当前版本: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} 检测到本地开发版本，建议通过源码编译或加 {} 覆盖。",
                    "!".yellow().bold(),
                    "ags update --force".cyan().bold()
                );
                return Ok(());
            }
            InstallSource::Standalone => {}
        }
    }

    let can_update = match cmp {
        Ordering::Less => true,
        Ordering::Equal => args.force,
        Ordering::Greater => args.force,
    };

    if !can_update {
        if cmp == Ordering::Equal {
            println!(
                "{} 当前已是最新版本 ({})。若要强制重新安装，请使用 {}。",
                "✓".green().bold(),
                display_remote.green(),
                "ags update --force".cyan().bold()
            );
        } else {
            println!(
                "{} 当前本地版本 ({}) 高于远端目标版本 ({})。若要强制覆盖，请使用 {}。",
                "!".yellow().bold(),
                display_current.yellow(),
                display_remote.cyan(),
                "ags update --force".cyan().bold()
            );
        }
        return Ok(());
    }

    println!(
        "{} 目标版本: {} (当前版本: {})",
        "→".cyan().bold(),
        display_remote.green().bold(),
        display_current.yellow()
    );
    println!("{} 目标平台: {}", "→".cyan().bold(), target.yellow());

    let asset = find_matching_asset(&release.assets, target).ok_or_else(|| {
        let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
        anyhow::anyhow!(
            "在 Release {} 中未找到适用于当前平台 ({}) 的构建资产。\n可用文件列表: {:?}",
            display_remote,
            target,
            available
        )
    })?;

    println!(
        "{} 正在下载构建资产: {} ...",
        "→".cyan().bold(),
        asset.name.cyan().bold()
    );

    let resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("下载构建资产失败: {}", asset.browser_download_url))?;

    if !resp.status().is_success() {
        anyhow::bail!("下载资产失败 (HTTP {})", resp.status());
    }

    let bytes = resp.bytes().await.context("读取下载数据失败")?;
    let size_kb = bytes.len() as f64 / 1024.0;
    println!(
        "{} 下载完成 ({:.1} KB)，正在校验并解压可执行文件...",
        "✓".green().bold(),
        size_kb
    );

    let binary_bytes = extract_binary(&asset.name, &bytes)?;

    println!(
        "{} 正在自我替换当前程序: {} ...",
        "→".cyan().bold(),
        current_exe.display().to_string().yellow()
    );

    replace_executable(&current_exe, &binary_bytes)?;

    println!();
    println!(
        "{} 成功更新 ags 至版本 {}！",
        "✓".green().bold(),
        display_remote.green().bold()
    );
    println!(
        "可执行文件位置: {}",
        current_exe.display().to_string().cyan()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_get_target_for() {
        assert_eq!(
            get_target_for("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-musl"
        );
        assert_eq!(
            get_target_for("linux", "aarch64").unwrap(),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(
            get_target_for("macos", "aarch64").unwrap(),
            "aarch64-apple-darwin"
        );
        assert_eq!(
            get_target_for("macos", "x86_64").unwrap(),
            "x86_64-apple-darwin"
        );
        assert_eq!(
            get_target_for("windows", "x86_64").unwrap(),
            "x86_64-pc-windows-msvc"
        );
        assert!(get_target_for("freebsd", "x86_64").is_err());
        assert!(get_target_for("linux", "mips").is_err());
    }

    #[test]
    fn test_current_target_succeeds_on_supported_env() {
        let target = current_target();
        assert!(target.is_ok());
    }

    #[test]
    fn test_compare_versions() {
        // v0.1.1 > v0.1.0
        assert_eq!(compare_versions("0.1.0", "v0.1.1").unwrap(), Ordering::Less);
        assert_eq!(
            compare_versions("v0.1.1", "0.1.0").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("v0.1.0", "0.1.0").unwrap(),
            Ordering::Equal
        );

        // 次版本与主版本对比
        assert_eq!(compare_versions("0.1.9", "0.2.0").unwrap(), Ordering::Less);
        assert_eq!(
            compare_versions("1.0.0", "0.9.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("0.10.0", "0.9.9").unwrap(),
            Ordering::Greater
        );

        // 预发布版本对比
        assert_eq!(
            compare_versions("1.0.0-beta.1", "1.0.0").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("1.0.0", "1.0.0-rc1").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.0.0-alpha", "1.0.0-beta").unwrap(),
            Ordering::Less
        );

        // 构建元数据忽略
        assert_eq!(
            compare_versions("1.0.0+build1", "1.0.0+build2").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_find_matching_asset() {
        let assets = vec![
            ReleaseAsset {
                name: "ags-0.2.0-x86_64-unknown-linux-musl.tar.gz".to_string(),
                browser_download_url: "https://example.com/musl.tar.gz".to_string(),
                size: Some(1024),
            },
            ReleaseAsset {
                name: "ags-0.2.0-x86_64-unknown-linux-musl.tar.gz.sha256".to_string(),
                browser_download_url: "https://example.com/musl.sha256".to_string(),
                size: Some(64),
            },
            ReleaseAsset {
                name: "ags-0.2.0-x86_64-pc-windows-msvc.zip".to_string(),
                browser_download_url: "https://example.com/win.zip".to_string(),
                size: Some(2048),
            },
            ReleaseAsset {
                name: "ags-aarch64-apple-darwin".to_string(),
                browser_download_url: "https://example.com/mac".to_string(),
                size: Some(1500),
            },
        ];

        // 匹配 linux musl
        let matched = find_matching_asset(&assets, "x86_64-unknown-linux-musl");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name,
            "ags-0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );

        // 匹配 windows
        let matched = find_matching_asset(&assets, "x86_64-pc-windows-msvc");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name,
            "ags-0.2.0-x86_64-pc-windows-msvc.zip"
        );

        // 匹配 macos
        let matched = find_matching_asset(&assets, "aarch64-apple-darwin");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "ags-aarch64-apple-darwin");

        // 未匹配
        let matched = find_matching_asset(&assets, "armv7-unknown-linux-gnueabihf");
        assert!(matched.is_none());
    }

    #[test]
    fn test_find_matching_asset_linux_fallback() {
        let assets = vec![ReleaseAsset {
            name: "ags-0.2.0-x86_64-unknown-linux-gnu.tar.gz".to_string(),
            browser_download_url: "https://example.com/gnu.tar.gz".to_string(),
            size: Some(1024),
        }];

        // 当只有 gnu 时，musl 目标回退到 gnu
        let matched = find_matching_asset(&assets, "x86_64-unknown-linux-musl");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name,
            "ags-0.2.0-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn test_extract_binary_tar_gz() {
        let dummy_binary = b"#!/bin/sh\necho 'new ags binary'";

        // 构建内存 tar.gz
        let mut tar_builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path("ags-0.2.0/ags").unwrap();
        header.set_size(dummy_binary.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        tar_builder.append(&header, &dummy_binary[..]).unwrap();
        let tar_bytes = tar_builder.into_inner().unwrap();

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        let gz_bytes = encoder.finish().unwrap();

        let extracted =
            extract_binary("ags-0.2.0-x86_64-unknown-linux-musl.tar.gz", &gz_bytes).unwrap();
        assert_eq!(extracted, dummy_binary);
    }

    #[test]
    fn test_extract_binary_zip() {
        let dummy_binary = b"MZ-test-windows-executable";

        // 构建内存 zip
        let mut zip_buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut zip_buf);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("ags-0.2.0/ags.exe", options).unwrap();
            zip.write_all(dummy_binary).unwrap();
            zip.finish().unwrap();
        }

        let zip_bytes = zip_buf.into_inner();
        let extracted = extract_binary("ags-0.2.0-x86_64-pc-windows-msvc.zip", &zip_bytes).unwrap();
        assert_eq!(extracted, dummy_binary);
    }

    #[test]
    fn test_extract_binary_raw() {
        let raw_data = b"ELF-raw-binary-data";
        let extracted = extract_binary("ags-x86_64-unknown-linux-musl", raw_data).unwrap();
        assert_eq!(extracted, raw_data);
    }

    #[test]
    fn test_replace_executable() {
        let dir = tempdir().unwrap();
        let exe_path = dir.path().join("ags");
        std::fs::write(&exe_path, b"old-version-binary").unwrap();

        let new_binary = b"new-updated-binary-content";
        replace_executable(&exe_path, new_binary).unwrap();

        let content = std::fs::read(&exe_path).unwrap();
        assert_eq!(content, new_binary);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&exe_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o755);
        }
    }

    #[test]
    fn test_install_source_display() {
        assert_eq!(InstallSource::Homebrew.to_string(), "Homebrew");
        assert_eq!(InstallSource::Cargo.to_string(), "Cargo");
        assert_eq!(InstallSource::DevBuild.to_string(), "DevBuild");
        assert_eq!(InstallSource::Standalone.to_string(), "Standalone");
    }

    #[test]
    fn test_detect_install_source_patterns() {
        // Homebrew paths
        assert_eq!(
            detect_install_source(Path::new("/usr/local/Cellar/agent-switch/0.2.0/bin/ags")),
            InstallSource::Homebrew
        );
        assert_eq!(
            detect_install_source(Path::new("/opt/homebrew/Cellar/agent-switch/0.2.0/bin/ags")),
            InstallSource::Homebrew
        );
        assert_eq!(
            detect_install_source(Path::new("/opt/homebrew/bin/ags")),
            InstallSource::Homebrew
        );
        assert_eq!(
            detect_install_source(Path::new("/home/linuxbrew/.linuxbrew/bin/ags")),
            InstallSource::Homebrew
        );
        assert_eq!(
            detect_install_source(Path::new(
                "/home/linuxbrew/.linuxbrew/Cellar/agent-switch/0.2.0/bin/ags"
            )),
            InstallSource::Homebrew
        );
        assert_eq!(
            detect_install_source(Path::new(r"C:\opt\homebrew\bin\ags.exe")),
            InstallSource::Homebrew
        );

        // Cargo paths
        assert_eq!(
            detect_install_source(Path::new("/home/frost/.cargo/bin/ags")),
            InstallSource::Cargo
        );
        assert_eq!(
            detect_install_source(Path::new(r"C:\Users\frost\.cargo\bin\ags.exe")),
            InstallSource::Cargo
        );

        // DevBuild paths
        assert_eq!(
            detect_install_source(Path::new(
                "/home/frost/codes/rust/agent-switch/target/debug/ags"
            )),
            InstallSource::DevBuild
        );
        assert_eq!(
            detect_install_source(Path::new(
                "/home/frost/codes/rust/agent-switch/target/release/ags"
            )),
            InstallSource::DevBuild
        );
        assert_eq!(
            detect_install_source(Path::new("target/debug/ags")),
            InstallSource::DevBuild
        );
        assert_eq!(
            detect_install_source(Path::new("target/release/ags")),
            InstallSource::DevBuild
        );
        assert_eq!(
            detect_install_source(Path::new(
                "/home/frost/target/x86_64-unknown-linux-musl/debug/ags"
            )),
            InstallSource::DevBuild
        );
        assert_eq!(
            detect_install_source(Path::new(r"C:\codes\target\debug\ags.exe")),
            InstallSource::DevBuild
        );

        // Standalone paths
        assert_eq!(
            detect_install_source(Path::new("/usr/local/bin/ags")),
            InstallSource::Standalone
        );
        assert_eq!(
            detect_install_source(Path::new("/home/frost/.local/bin/ags")),
            InstallSource::Standalone
        );
        assert_eq!(
            detect_install_source(Path::new("/usr/bin/ags")),
            InstallSource::Standalone
        );
        assert_eq!(
            detect_install_source(Path::new(
                r"C:\Users\frost\AppData\Local\Programs\ags\ags.exe"
            )),
            InstallSource::Standalone
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_detect_install_source_symlink_homebrew() {
        let dir = tempdir().unwrap();
        let cellar_dir = dir
            .path()
            .join("Cellar")
            .join("agent-switch")
            .join("0.2.0")
            .join("bin");
        std::fs::create_dir_all(&cellar_dir).unwrap();
        let real_binary = cellar_dir.join("ags");
        std::fs::write(&real_binary, b"#!/bin/sh\necho ags").unwrap();

        let bin_dir = dir.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let symlink_path = bin_dir.join("ags");
        std::os::unix::fs::symlink(&real_binary, &symlink_path).unwrap();

        assert_eq!(
            detect_install_source(&symlink_path),
            InstallSource::Homebrew
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_detect_install_source_symlink_cargo() {
        let dir = tempdir().unwrap();
        let cargo_dir = dir.path().join(".cargo").join("bin");
        std::fs::create_dir_all(&cargo_dir).unwrap();
        let real_binary = cargo_dir.join("ags");
        std::fs::write(&real_binary, b"binary").unwrap();

        let symlink_path = dir.path().join("standalone_ags");
        std::os::unix::fs::symlink(&real_binary, &symlink_path).unwrap();

        assert_eq!(detect_install_source(&symlink_path), InstallSource::Cargo);
    }
}
