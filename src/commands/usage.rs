use crate::agent::{all_agents, get_agent};
use crate::crypto::CryptoProvider;
use anyhow::{bail, Result};
use colored::*;
use std::path::Path;

/// Handle the `ags usage` command across all agents or a filtered agent.
pub async fn handle_usage(
    base_dir: &Path,
    crypto: &dyn CryptoProvider,
    agent_filter: Option<&str>,
) -> Result<()> {
    if let Some(target) = agent_filter {
        let agent = match get_agent(target) {
            Some(a) => a,
            None => bail!("Unknown agent '{}'. Supported: codex, agy", target),
        };
        let store = agent.default_store(base_dir);
        match agent.fetch_usage(&store, crypto).await {
            Ok(report) => {
                crate::ui::progress::render_usage_report(&report, true);
            }
            Err(e) => {
                eprintln!(
                    "{}: {}",
                    format!("{} usage error", agent.display_name()).red(),
                    e
                );
            }
        }
    } else {
        let agents = all_agents();
        for (idx, agent) in agents.iter().enumerate() {
            if idx > 0 {
                println!();
            }
            let store = agent.default_store(base_dir);
            match agent.fetch_usage(&store, crypto).await {
                Ok(report) => {
                    crate::ui::progress::render_usage_report(&report, true);
                }
                Err(e) => {
                    eprintln!(
                        "{}: {}",
                        format!("{} usage error", agent.display_name()).red(),
                        e
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_handle_usage_all_empty() {
        let dir = tempdir().unwrap();
        let crypto = PlaintextCryptoProvider;
        let res = handle_usage(dir.path(), &crypto, None).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_handle_usage_single_agent() {
        let dir = tempdir().unwrap();
        let crypto = PlaintextCryptoProvider;
        let res_codex = handle_usage(dir.path(), &crypto, Some("codex")).await;
        assert!(res_codex.is_ok());

        let res_agy = handle_usage(dir.path(), &crypto, Some("agy")).await;
        assert!(res_agy.is_ok());
    }

    #[tokio::test]
    async fn test_handle_usage_unknown_agent() {
        let dir = tempdir().unwrap();
        let crypto = PlaintextCryptoProvider;
        let res = handle_usage(dir.path(), &crypto, Some("unknown_agent")).await;
        assert!(res.is_err());
    }
}
