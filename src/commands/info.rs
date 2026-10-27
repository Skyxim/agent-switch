use crate::agent::all_agents;
use crate::crypto::{env::ENV_VAR_NAME, CryptoProvider};
use anyhow::Result;
use colored::*;
use std::env;
use std::path::Path;

/// Handle the `ags info` command to display environment, store engine, and agent account summaries.
pub fn handle_info(base_dir: &Path, crypto: &dyn CryptoProvider) -> Result<()> {
    println!("{}", "=== Agent Switch (ags) Info ===".bold().cyan());
    println!("Version:          {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Base Directory:   {}",
        base_dir.display().to_string().yellow()
    );

    let env_key_set = env::var(ENV_VAR_NAME)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);

    let engine_desc = if crypto.name() == "env" {
        if env_key_set {
            format!("env (key configured via {})", ENV_VAR_NAME).green()
        } else {
            format!("env ({} empty -> unencrypted)", ENV_VAR_NAME).yellow()
        }
    } else {
        crypto.name().cyan()
    };

    println!("Store Engine:     {}", engine_desc);
    println!("Supported:        env (default), plain, system (planned)");
    println!("Encryption State: {}", crypto.description().green());
    println!(
        "{:<18}{}",
        format!("{}:", ENV_VAR_NAME),
        if env_key_set {
            "Set (AES-256-GCM)".green().bold()
        } else {
            "Not set (Plaintext fallback)".yellow()
        }
    );

    for agent in all_agents() {
        let store = agent.default_store(base_dir);
        let reg = store.load_registry().unwrap_or_default();
        let active = reg.active.as_deref().unwrap_or("-");
        let label = format!("{} Accounts:", agent.display_name());
        println!(
            "{:<18}{} total, active: {}",
            label,
            reg.accounts.len(),
            active.cyan()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use tempfile::tempdir;

    #[test]
    fn test_handle_info_output() {
        let dir = tempdir().unwrap();
        let crypto = PlaintextCryptoProvider;
        assert!(handle_info(dir.path(), &crypto).is_ok());
    }
}
