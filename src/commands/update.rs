use anyhow::{Context, Result};
use colored::*;
use std::cmp::Ordering;
use std::io::Read;
use std::path::Path;

#[derive(clap::Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Check for available updates without downloading or installing
    #[arg(long)]
    pub check: bool,

    /// Force update even if already up to date
    #[arg(short = 'f', long = "force")]
    pub force: bool,

    /// Target a specific Release tag (optional, defaults to latest)
    #[arg(long)]
    pub tag: Option<String>,
}

/// Binary installation source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// Installed via Homebrew package manager
    Homebrew,
    /// Installed via Cargo (cargo install)
    Cargo,
    /// Local development build (target/debug or target/release)
    DevBuild,
    /// Standalone binary or official script install
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
    // 1. Local dev build path patterns: /target/debug/ or /target/release/
    if s.contains("/target/debug")
        || s.starts_with("target/debug")
        || s.contains("/target/release")
        || s.starts_with("target/release")
        || (s.contains("/target/") && (s.contains("/debug") || s.contains("/release")))
    {
        return Some(InstallSource::DevBuild);
    }

    // 2. Cargo default install path pattern: .cargo/bin
    if s.contains(".cargo/bin") {
        return Some(InstallSource::Cargo);
    }

    // 3. Homebrew path patterns: cellar/, homebrew, /opt/homebrew, linuxbrew
    if s.contains("cellar/")
        || s.contains("homebrew")
        || s.contains("/opt/homebrew")
        || s.contains("linuxbrew")
    {
        return Some(InstallSource::Homebrew);
    }

    None
}

/// Detect installation source of the executable
pub fn detect_install_source(exe_path: &Path) -> InstallSource {
    // Prefer resolving symbolic links (Homebrew often symlinks bin/ags to Cellar/...)
    if let Ok(canonical) = std::fs::canonicalize(exe_path) {
        let canon_str = canonical
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        if let Some(src) = path_matches_source(&canon_str) {
            return src;
        }
    }

    // Check original provided path
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

/// Simple semantic version structure for version comparison
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SimpleVersion {
    pub numbers: Vec<u64>,
    pub pre: Option<String>,
}

impl SimpleVersion {
    pub fn parse(s: &str) -> Result<Self> {
        let trimmed = s.trim().trim_start_matches(['v', 'V']);
        if trimmed.is_empty() {
            anyhow::bail!("Version string is empty");
        }

        // Strip build metadata (+build.123)
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
                    .with_context(|| format!("Failed to parse version segment '{}'", part))
            })
            .collect();
        let numbers = numbers?;

        if numbers.is_empty() {
            anyhow::bail!("Version core numbers cannot be empty");
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

        // When numeric parts are identical, compare pre-release identifiers
        // Rule: Release versions (without pre-release) are greater than pre-release versions
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

/// Compare two version strings (supports leading v/V)
/// Returns Ordering: v1.cmp(v2)
pub fn compare_versions(v1: &str, v2: &str) -> Result<Ordering> {
    let ver1 = SimpleVersion::parse(v1)?;
    let ver2 = SimpleVersion::parse(v2)?;
    Ok(ver1.cmp(&ver2))
}

/// Get standard Target Triple based on OS and architecture
pub fn get_target_for(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => anyhow::bail!(
            "Target platform is not supported for auto-update: {}/{}",
            os,
            arch
        ),
    }
}

/// Get current runtime target platform triple
pub fn current_target() -> Result<&'static str> {
    get_target_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Find release asset matching target platform from list of release assets
pub fn find_matching_asset<'a>(
    assets: &'a [ReleaseAsset],
    target: &str,
) -> Option<&'a ReleaseAsset> {
    // Filter out checksums, signatures, and non-executable assets
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

    // 1. Exact match with target identifier
    if let Some(asset) = candidates.iter().find(|a| a.name.contains(target)) {
        return Some(asset);
    }

    // 2. Linux platform musl and gnu fallback
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

/// Extract executable binary bytes from downloaded data (supports .tar.gz, .zip, and raw binaries)
pub fn extract_binary(asset_name: &str, data: &[u8]) -> Result<Vec<u8>> {
    let lower_name = asset_name.to_lowercase();
    if lower_name.ends_with(".tar.gz") || lower_name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(data);
        let mut archive = tar::Archive::new(gz);
        for entry_res in archive
            .entries()
            .context("Failed to read tar archive entries")?
        {
            let mut entry = entry_res.context("Failed to read tar entry")?;
            let is_file = entry.header().entry_type().is_file();
            let path = entry.path().context("Failed to get tar file path")?;
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
                    .context("Failed to read binary in archive")?;
                return Ok(buf);
            }
        }
        anyhow::bail!(
            "Executable 'ags' or 'ags.exe' not found in archive '{}'",
            asset_name
        );
    } else if lower_name.ends_with(".zip") {
        let cursor = std::io::Cursor::new(data);
        let mut zip = zip::ZipArchive::new(cursor).context("Failed to parse zip archive")?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).context("Failed to read zip entry")?;
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
                        .context("Failed to read binary in zip")?;
                    return Ok(buf);
                }
            }
        }
        anyhow::bail!(
            "Executable 'ags' or 'ags.exe' not found in zip archive '{}'",
            asset_name
        );
    } else {
        // Raw binary
        Ok(data.to_vec())
    }
}

/// Perform executable replacement (creates temporary file in target directory and atomically replaces)
pub fn replace_executable(target_path: &Path, binary_bytes: &[u8]) -> Result<()> {
    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Unable to get executable directory"))?;

    let temp_file_path = parent.join(format!(".ags-update-tmp.{}", rand::random::<u32>()));

    std::fs::write(&temp_file_path, binary_bytes).with_context(|| {
        format!(
            "Failed to write temporary update file {}",
            temp_file_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        if let Err(e) = std::fs::set_permissions(&temp_file_path, perms) {
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e).context("Failed to set executable permissions (0755) on update file");
        }

        if let Err(e) = std::fs::rename(&temp_file_path, target_path) {
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e).with_context(|| {
                format!(
                    "Failed to atomically replace executable '{}' (try running with sudo if installed in a protected directory)",
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
            return Err(e).with_context(|| {
                format!("Failed to rename old version file to {}", old_exe.display())
            });
        }

        if let Err(e) = std::fs::rename(&temp_file_path, target_path) {
            let _ = std::fs::rename(&old_exe, target_path);
            let _ = std::fs::remove_file(&temp_file_path);
            return Err(e)
                .with_context(|| "Failed to replace with new binary; restored previous version");
        }

        // Clean up .old file; ignore silently if locked by system
        let _ = std::fs::remove_file(&old_exe);
    }

    #[cfg(not(any(unix, windows)))]
    {
        std::fs::rename(&temp_file_path, target_path)?;
    }

    Ok(())
}

/// Request GitHub API for Release information
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

    let resp = req
        .send()
        .await
        .context("Failed to connect to GitHub Release API")?;
    let status = resp.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        // If specified tag returned 404, retry toggling 'v' prefix
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
                        .context("Failed to parse GitHub Release response");
                }
            }
        }
        anyhow::bail!(
            "Specified release not found (HTTP 404). Please ensure this release is published."
        );
    }

    if status == reqwest::StatusCode::FORBIDDEN {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "GitHub API rate limit exceeded (HTTP 403): {}. Set GITHUB_TOKEN environment variable to avoid rate limits.",
            body
        );
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Failed to fetch GitHub Release API (HTTP {}): {}",
            status,
            body
        );
    }

    resp.json::<GithubRelease>()
        .await
        .context("Failed to parse GitHub Release response")
}

/// Execute main update process
pub async fn handle_update(args: UpdateArgs) -> Result<()> {
    let current_ver = env!("CARGO_PKG_VERSION");
    let target = current_target()?;
    let current_exe = std::env::current_exe().context("Unable to get current executable path")?;
    let source = detect_install_source(&current_exe);

    println!(
        "{} Checking for updates from GitHub Releases...",
        "→".cyan().bold()
    );

    let client = reqwest::Client::builder()
        .user_agent(format!("agent-switch/{}", current_ver))
        .build()
        .context("Failed to build HTTP client")?;

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
            "=== agent-switch (ags) Version Check Report ==="
                .bold()
                .cyan()
        );
        println!("Current version: {}", display_current.yellow());
        println!("Target version:  {}", display_remote.green().bold());
        match cmp {
            Ordering::Less => {
                println!(
                    "Version status:  {}",
                    "New version available, update recommended!".green().bold()
                );
                println!(
                    "Update command:  Run {} to perform automatic update",
                    "ags update".cyan().bold()
                );
            }
            Ordering::Equal => {
                println!("Version status:  {}", "Already up to date".green());
            }
            Ordering::Greater => {
                println!(
                    "Version status:  {}",
                    "Local version is ahead of remote Release (dev build)".yellow()
                );
            }
        }

        if !args.force {
            match source {
                InstallSource::Homebrew => {
                    println!();
                    println!(
                        "{}",
                        "Hint: Currently installed via Homebrew. Run `brew update && brew upgrade agent-switch` to update."
                            .yellow()
                    );
                }
                InstallSource::Cargo => {
                    println!();
                    println!(
                        "{}",
                        "Hint: Currently installed via Cargo. Run `cargo install --force agent-switch` to update."
                            .yellow()
                    );
                }
                InstallSource::DevBuild => {
                    println!();
                    println!(
                        "{}",
                        "Hint: Local development build detected. Build from source or run with -f/--force to overwrite."
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
                    "{} Target version: {} (Current version: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} Currently installed via Homebrew. Please update using:\n  {}\nTo force self-update and replace binary from GitHub Releases, run {}.",
                    "i".cyan().bold(),
                    "brew update && brew upgrade agent-switch".green().bold(),
                    "ags update --force".cyan().bold()
                );
                return Ok(());
            }
            InstallSource::Cargo => {
                println!(
                    "{} Target version: {} (Current version: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} Currently installed via Cargo. Please update using:\n  {}\nTo force self-update and replace binary from GitHub Releases, run {}.",
                    "i".cyan().bold(),
                    "cargo install --force agent-switch".green().bold(),
                    "ags update --force".cyan().bold()
                );
                return Ok(());
            }
            InstallSource::DevBuild => {
                println!(
                    "{} Target version: {} (Current version: {})",
                    "→".cyan().bold(),
                    display_remote.green().bold(),
                    display_current.yellow()
                );
                println!(
                    "{} Local development build detected. Build from source or run with {} to overwrite.",
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
                "{} Already up to date ({}). Run {} to reinstall.",
                "✓".green().bold(),
                display_remote.green(),
                "ags update --force".cyan().bold()
            );
        } else {
            println!(
                "{} Current version ({}) is ahead of target version ({}). Run {} to force overwrite.",
                "!".yellow().bold(),
                display_current.yellow(),
                display_remote.cyan(),
                "ags update --force".cyan().bold()
            );
        }
        return Ok(());
    }

    println!(
        "{} Target version: {} (Current version: {})",
        "→".cyan().bold(),
        display_remote.green().bold(),
        display_current.yellow()
    );
    println!("{} Target platform: {}", "→".cyan().bold(), target.yellow());

    let asset = find_matching_asset(&release.assets, target).ok_or_else(|| {
        let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
        anyhow::anyhow!(
            "No suitable release asset found for platform ({}) in Release {}.\nAvailable assets: {:?}",
            target,
            display_remote,
            available
        )
    })?;

    println!(
        "{} Downloading release asset: {} ...",
        "→".cyan().bold(),
        asset.name.cyan().bold()
    );

    let resp = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| {
            format!(
                "Failed to download release asset: {}",
                asset.browser_download_url
            )
        })?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to download asset (HTTP {})", resp.status());
    }

    let bytes = resp
        .bytes()
        .await
        .context("Failed to read download response body")?;
    let size_kb = bytes.len() as f64 / 1024.0;
    println!(
        "{} Download complete ({:.1} KB), verifying and unpacking executable...",
        "✓".green().bold(),
        size_kb
    );

    let binary_bytes = extract_binary(&asset.name, &bytes)?;

    println!(
        "{} Replacing executable: {} ...",
        "→".cyan().bold(),
        current_exe.display().to_string().yellow()
    );

    replace_executable(&current_exe, &binary_bytes)?;

    println!();
    println!(
        "{} Successfully updated ags to version {}!",
        "✓".green().bold(),
        display_remote.green().bold()
    );
    println!(
        "Executable path: {}",
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

        // Minor and major version comparison
        assert_eq!(compare_versions("0.1.9", "0.2.0").unwrap(), Ordering::Less);
        assert_eq!(
            compare_versions("1.0.0", "0.9.9").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("0.10.0", "0.9.9").unwrap(),
            Ordering::Greater
        );

        // Pre-release version comparison
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

        // Build metadata ignored
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

        // Match linux musl
        let matched = find_matching_asset(&assets, "x86_64-unknown-linux-musl");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name,
            "ags-0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );

        // Match windows
        let matched = find_matching_asset(&assets, "x86_64-pc-windows-msvc");
        assert!(matched.is_some());
        assert_eq!(
            matched.unwrap().name,
            "ags-0.2.0-x86_64-pc-windows-msvc.zip"
        );

        // Match macos
        let matched = find_matching_asset(&assets, "aarch64-apple-darwin");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "ags-aarch64-apple-darwin");

        // Unmatched
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

        // Fallback to gnu when only gnu is present
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

        // Build in-memory tar.gz
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

        // Build in-memory zip
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
