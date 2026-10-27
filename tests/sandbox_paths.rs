use agent_switch::agent::{
    default_auth_path, default_creds_path, default_token_path, AgyAgent, CodexAgent,
};
use agent_switch::config::{get_base_dir, load_global_config, save_global_config};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::tempdir;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

struct EnvCleaner {
    saved_ags_home: Option<String>,
    saved_xdg_config_home: Option<String>,
    saved_ags_crypto_provider: Option<String>,
    saved_codex_auth_path: Option<String>,
    saved_codex_home: Option<String>,
    saved_antigravity_home: Option<String>,
    saved_agy_home: Option<String>,
    saved_agy_keyring_data: Option<String>,
}

impl EnvCleaner {
    fn new() -> Self {
        Self {
            saved_ags_home: env::var("AGS_HOME").ok(),
            saved_xdg_config_home: env::var("XDG_CONFIG_HOME").ok(),
            saved_ags_crypto_provider: env::var("AGS_CRYPTO_PROVIDER").ok(),
            saved_codex_auth_path: env::var("CODEX_AUTH_PATH").ok(),
            saved_codex_home: env::var("CODEX_HOME").ok(),
            saved_antigravity_home: env::var("ANTIGRAVITY_HOME").ok(),
            saved_agy_home: env::var("AGY_HOME").ok(),
            saved_agy_keyring_data: env::var("AGY_KEYRING_DATA").ok(),
        }
    }
}

impl Drop for EnvCleaner {
    fn drop(&mut self) {
        Self::restore("AGS_HOME", &self.saved_ags_home);
        Self::restore("XDG_CONFIG_HOME", &self.saved_xdg_config_home);
        Self::restore("AGS_CRYPTO_PROVIDER", &self.saved_ags_crypto_provider);
        Self::restore("CODEX_AUTH_PATH", &self.saved_codex_auth_path);
        Self::restore("CODEX_HOME", &self.saved_codex_home);
        Self::restore("ANTIGRAVITY_HOME", &self.saved_antigravity_home);
        Self::restore("AGY_HOME", &self.saved_agy_home);
        Self::restore("AGY_KEYRING_DATA", &self.saved_agy_keyring_data);
    }
}

impl EnvCleaner {
    fn restore(var: &str, saved: &Option<String>) {
        match saved {
            Some(v) => unsafe { env::set_var(var, v) },
            None => unsafe { env::remove_var(var) },
        }
    }
}

#[test]
fn test_sandbox_isolation_with_ags_home() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _cleaner = EnvCleaner::new();

    let temp_sandbox = tempdir().unwrap();
    let sandbox_path = temp_sandbox.path();
    let sandbox_str = sandbox_path.to_str().unwrap();

    let codex_home = sandbox_path.join(".codex");
    let agy_home = sandbox_path.join(".gemini").join("antigravity-cli");

    unsafe {
        env::set_var("AGS_HOME", sandbox_str);
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("CODEX_AUTH_PATH");
        env::set_var("CODEX_HOME", codex_home.to_str().unwrap());
        env::set_var("ANTIGRAVITY_HOME", agy_home.to_str().unwrap());
        env::remove_var("AGY_HOME");
    }

    // 1. Verify get_base_dir points directly to virtual sandbox root
    assert_eq!(get_base_dir(), sandbox_path.to_path_buf());

    // 2. Verify Codex auth path points to sandbox .codex/auth.json
    let codex = CodexAgent::new();
    assert_eq!(codex.auth_path(), codex_home.join("auth.json"));
    assert_eq!(default_auth_path(), codex_home.join("auth.json"));

    // 3. Verify AGY token and creds paths point to sandbox
    assert_eq!(
        default_token_path(),
        agy_home.join("antigravity-oauth-token")
    );
    assert_eq!(default_creds_path(), agy_home.join("oauth_creds.json"));
}

#[test]
fn test_sandbox_isolation_with_host_xdg_config_home_set() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _cleaner = EnvCleaner::new();

    let temp_sandbox = tempdir().unwrap();
    let sandbox_path = temp_sandbox.path();
    let sandbox_str = sandbox_path.to_str().unwrap();

    unsafe {
        // Set AGS_HOME to sandbox
        env::set_var("AGS_HOME", sandbox_str);
        // Simulate host XDG_CONFIG_HOME set (e.g. CI runner /home/runner/.config)
        env::set_var("XDG_CONFIG_HOME", "/home/runner/.config");
    }

    // AGS_HOME takes precedence over host XDG_CONFIG_HOME
    assert_eq!(get_base_dir(), sandbox_path.to_path_buf());
}

#[test]
fn test_local_overrides_over_sandbox_home() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _cleaner = EnvCleaner::new();

    let temp_sandbox = tempdir().unwrap();
    let sandbox_str = temp_sandbox.path().to_str().unwrap();

    unsafe {
        env::set_var("AGS_HOME", sandbox_str);
        env::remove_var("XDG_CONFIG_HOME");
        env::set_var("CODEX_HOME", "/custom/codex/location");
        env::set_var("ANTIGRAVITY_HOME", "/custom/agy/location");
    }

    // Codex paths should use CODEX_HOME
    assert_eq!(
        default_auth_path(),
        PathBuf::from("/custom/codex/location/auth.json")
    );
    assert_eq!(
        CodexAgent::new().auth_path(),
        PathBuf::from("/custom/codex/location/auth.json")
    );

    // AGY paths should use ANTIGRAVITY_HOME
    assert_eq!(
        default_token_path(),
        PathBuf::from("/custom/agy/location/antigravity-oauth-token")
    );
    assert_eq!(
        default_creds_path(),
        PathBuf::from("/custom/agy/location/oauth_creds.json")
    );

    // Direct CODEX_AUTH_PATH override takes precedence over CODEX_HOME
    unsafe {
        env::set_var("CODEX_AUTH_PATH", "/custom/file/auth.json");
    }
    assert_eq!(default_auth_path(), PathBuf::from("/custom/file/auth.json"));
    assert_eq!(
        CodexAgent::new().auth_path(),
        PathBuf::from("/custom/file/auth.json")
    );

    // AGY_HOME fallback when ANTIGRAVITY_HOME is unset
    unsafe {
        env::remove_var("ANTIGRAVITY_HOME");
        env::set_var("AGY_HOME", "/custom/agy2/location");
    }
    assert_eq!(
        default_token_path(),
        PathBuf::from("/custom/agy2/location/antigravity-oauth-token")
    );
    assert_eq!(
        default_creds_path(),
        PathBuf::from("/custom/agy2/location/oauth_creds.json")
    );
}

#[test]
fn test_sandbox_file_read_write() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _cleaner = EnvCleaner::new();

    let temp_sandbox = tempdir().unwrap();
    let sandbox_path = temp_sandbox.path();

    let codex_home = sandbox_path.join(".codex");
    let agy_home = sandbox_path.join(".gemini").join("antigravity-cli");

    unsafe {
        env::set_var("AGS_HOME", sandbox_path.to_str().unwrap());
        env::remove_var("XDG_CONFIG_HOME");
        env::remove_var("CODEX_AUTH_PATH");
        env::set_var("CODEX_HOME", codex_home.to_str().unwrap());
        env::set_var("ANTIGRAVITY_HOME", agy_home.to_str().unwrap());
        env::remove_var("AGY_HOME");
        env::remove_var("AGY_KEYRING_DATA");
        env::remove_var("AGS_CRYPTO_PROVIDER");
    }

    // Verify AGS base directory configuration read/write isolation
    let base_dir = get_base_dir();
    assert_eq!(base_dir, sandbox_path.to_path_buf());

    let mut global_config = load_global_config(&base_dir).unwrap();
    assert!(global_config.crypto_provider.is_none());
    global_config.crypto_provider = Some("plain".to_string());
    save_global_config(&base_dir, &global_config).unwrap();

    let reloaded_config = load_global_config(&base_dir).unwrap();
    assert_eq!(reloaded_config.crypto_provider, Some("plain".to_string()));
    assert!(sandbox_path.join("config.json").exists());

    // Create fake Codex auth in sandbox
    let codex_auth = codex_home.join("auth.json");
    fs::create_dir_all(codex_auth.parent().unwrap()).unwrap();
    fs::write(&codex_auth, b"{\"fake_codex\":true}").unwrap();

    let codex = CodexAgent::new();
    let read_codex = codex.read_auth_raw().unwrap();
    assert_eq!(read_codex, b"{\"fake_codex\":true}");

    // Create fake AGY token in sandbox
    let agy_token = agy_home.join("antigravity-oauth-token");
    fs::create_dir_all(agy_token.parent().unwrap()).unwrap();
    fs::write(&agy_token, b"{\"fake_agy\":true}").unwrap();

    let read_agy = AgyAgent::read_credentials_raw().unwrap();
    assert_eq!(read_agy, b"{\"fake_agy\":true}");
}
