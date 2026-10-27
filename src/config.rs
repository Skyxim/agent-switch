use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crypto_provider: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub active_agents: HashMap<String, String>,
}

/// Resolves the base configuration directory according to the priority:
/// 1. `AGS_HOME` (if non-empty, directly return its path)
/// 2. `XDG_CONFIG_HOME/agent-switch` (if non-empty)
/// 3. `dirs::home_dir()/.config/agent-switch`
/// 4. `HOME/.config/agent-switch` (if non-empty)
/// 5. Fallback `.config/agent-switch`
pub fn resolve_base_dir(
    ags_home: Option<&str>,
    xdg_config_home: Option<&str>,
    home_dir: Option<PathBuf>,
    env_home: Option<&str>,
) -> PathBuf {
    if let Some(val) = ags_home {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Some(val) = xdg_config_home {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("agent-switch");
        }
    }

    if let Some(home) = home_dir {
        return home.join(".config").join("agent-switch");
    }

    if let Some(val) = env_home {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join(".config").join("agent-switch");
        }
    }

    PathBuf::from(".config").join("agent-switch")
}

pub fn get_base_dir() -> PathBuf {
    let ags_home = env::var("AGS_HOME").ok();
    let xdg_home = env::var("XDG_CONFIG_HOME").ok();
    let home_dir = dirs::home_dir();
    let env_home = env::var("HOME").ok();

    resolve_base_dir(
        ags_home.as_deref(),
        xdg_home.as_deref(),
        home_dir,
        env_home.as_deref(),
    )
}

pub fn default_base_dir() -> PathBuf {
    get_base_dir()
}

pub fn load_global_config(base_dir: &Path) -> Result<GlobalConfig> {
    let path = base_dir.join("config.json");
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }
    let data = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read global config: {}", path.display()))?;
    let config: GlobalConfig = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse global config: {}", path.display()))?;
    Ok(config)
}

pub fn save_global_config(base_dir: &Path, config: &GlobalConfig) -> Result<()> {
    fs::create_dir_all(base_dir)?;
    let path = base_dir.join("config.json");
    let content = serde_json::to_string_pretty(config)?;
    let temp_path = base_dir.join("config.json.tmp");
    fs::write(&temp_path, format!("{}\n", content))?;
    fs::rename(&temp_path, path)?;
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub use crate::util::TEST_ENV_LOCK as ENV_LOCK;

    struct EnvGuard {
        saved_ags_home: Option<String>,
        saved_xdg_config_home: Option<String>,
        saved_home: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                saved_ags_home: env::var("AGS_HOME").ok(),
                saved_xdg_config_home: env::var("XDG_CONFIG_HOME").ok(),
                saved_home: env::var("HOME").ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.saved_ags_home {
                Some(val) => unsafe { env::set_var("AGS_HOME", val) },
                None => unsafe { env::remove_var("AGS_HOME") },
            }
            match &self.saved_xdg_config_home {
                Some(val) => unsafe { env::set_var("XDG_CONFIG_HOME", val) },
                None => unsafe { env::remove_var("XDG_CONFIG_HOME") },
            }
            match &self.saved_home {
                Some(val) => unsafe { env::set_var("HOME", val) },
                None => unsafe { env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn test_get_base_dir_default_ends_with_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        unsafe {
            env::remove_var("AGS_HOME");
            env::remove_var("XDG_CONFIG_HOME");
        }

        let base_dir = get_base_dir();
        assert!(
            base_dir.ends_with(Path::new(".config").join("agent-switch")),
            "Expected base_dir to end with .config/agent-switch, got: {:?}",
            base_dir
        );
    }

    #[test]
    fn test_get_base_dir_ags_home_override() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        let custom_dir = "/tmp/test-ags-home";
        unsafe {
            env::set_var("AGS_HOME", custom_dir);
            env::remove_var("XDG_CONFIG_HOME");
        }

        let base_dir = get_base_dir();
        assert_eq!(base_dir, PathBuf::from(custom_dir));

        // Test with whitespace trimmed
        unsafe {
            env::set_var("AGS_HOME", "   /tmp/test-ags-trimmed   ");
        }
        assert_eq!(get_base_dir(), PathBuf::from("/tmp/test-ags-trimmed"));

        // Test that empty or whitespace-only AGS_HOME falls back
        unsafe {
            env::set_var("AGS_HOME", "   ");
        }
        let fallback_dir = get_base_dir();
        assert!(fallback_dir.ends_with(Path::new(".config").join("agent-switch")));
    }

    #[test]
    fn test_default_base_dir() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();
        unsafe {
            env::remove_var("AGS_HOME");
            env::remove_var("XDG_CONFIG_HOME");
        }
        assert_eq!(default_base_dir(), get_base_dir());
    }

    #[test]
    fn test_get_base_dir_xdg_config_home_override() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        unsafe {
            env::remove_var("AGS_HOME");
            env::set_var("XDG_CONFIG_HOME", "/tmp/test-xdg");
        }

        let base_dir = get_base_dir();
        assert_eq!(
            base_dir,
            PathBuf::from("/tmp/test-xdg").join("agent-switch")
        );
    }

    #[test]
    fn test_ags_home_overrides_xdg_config_home() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        unsafe {
            env::set_var("AGS_HOME", "/tmp/custom-ags-home");
            env::set_var("XDG_CONFIG_HOME", "/home/runner/.config");
        }

        assert_eq!(get_base_dir(), PathBuf::from("/tmp/custom-ags-home"));
    }

    #[test]
    fn test_resolve_base_dir_priority_levels() {
        // Priority 1: AGS_HOME
        let p1 = resolve_base_dir(
            Some("  /custom/ags-home  "),
            Some("/custom/xdg"),
            Some(PathBuf::from("/home/user")),
            Some("/home/user2"),
        );
        assert_eq!(p1, PathBuf::from("/custom/ags-home"));

        // Priority 2: XDG_CONFIG_HOME
        let p2 = resolve_base_dir(
            Some("   "),
            Some("  /custom/xdg  "),
            Some(PathBuf::from("/home/user")),
            Some("/home/user2"),
        );
        assert_eq!(p2, PathBuf::from("/custom/xdg").join("agent-switch"));

        // Priority 3: dirs::home_dir
        let p3 = resolve_base_dir(
            None,
            Some(""),
            Some(PathBuf::from("/home/user")),
            Some("/home/user2"),
        );
        assert_eq!(p3, PathBuf::from("/home/user/.config").join("agent-switch"));

        // Priority 4: HOME
        let p4 = resolve_base_dir(None, None, None, Some("  /home/user2  "));
        assert_eq!(
            p4,
            PathBuf::from("/home/user2/.config").join("agent-switch")
        );

        // Priority 5: Fallback
        let p5 = resolve_base_dir(None, None, None, Some("   "));
        assert_eq!(p5, PathBuf::from(".config").join("agent-switch"));

        let p6 = resolve_base_dir(None, None, None, None);
        assert_eq!(p6, PathBuf::from(".config").join("agent-switch"));
    }
}
