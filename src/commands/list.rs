use crate::agent::{all_agents, get_agent};
use crate::ui::AccountTableOptions;
use anyhow::{bail, Result};
use std::path::Path;

/// Handle the `ags list` (or `ags ls`) command across all agents or a filtered agent.
pub fn handle_list(base_dir: &Path, agent_filter: Option<&str>) -> Result<()> {
    if let Some(target) = agent_filter {
        let agent = match get_agent(target) {
            Some(a) => a,
            None => bail!("Unknown agent '{}'. Supported: codex, agy", target),
        };
        let store = agent.default_store(base_dir);
        let list_model = agent.list_accounts(&store)?;
        let items = list_model.to_table_items();
        let default_empty = format!(
            "No saved {} accounts. Use `ags {} add <name>` to add one.",
            agent.display_name(),
            agent.id()
        );
        let options = AccountTableOptions {
            app_display_name: Some(agent.display_name().to_string()),
            show_app_header: true,
            show_store_status: false,
            empty_message: Some(default_empty),
        };
        let rendered = crate::ui::render_account_table(&items, &options);
        if !rendered.is_empty() {
            println!("{}", rendered);
        }
    } else {
        let agents = all_agents();
        for (idx, agent) in agents.iter().enumerate() {
            if idx > 0 {
                println!();
            }
            let store = agent.default_store(base_dir);
            let list_model = agent.list_accounts(&store)?;
            let items = list_model.to_table_items();
            let default_empty = format!(
                "No saved {} accounts. Use `ags {} add <name>` to add one.",
                agent.display_name(),
                agent.id()
            );
            let options = AccountTableOptions {
                app_display_name: Some(agent.display_name().to_string()),
                show_app_header: true,
                show_store_status: false,
                empty_message: Some(default_empty),
            };
            let rendered = crate::ui::render_account_table(&items, &options);
            if !rendered.is_empty() {
                println!("{}", rendered);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_handle_list_all_agents() {
        let dir = tempdir().unwrap();
        let res = handle_list(dir.path(), None);
        assert!(res.is_ok());
    }

    #[test]
    fn test_handle_list_single_agent() {
        let dir = tempdir().unwrap();
        let res_codex = handle_list(dir.path(), Some("codex"));
        assert!(res_codex.is_ok());

        let res_agy = handle_list(dir.path(), Some("agy"));
        assert!(res_agy.is_ok());
    }

    #[test]
    fn test_handle_list_unknown_agent() {
        let dir = tempdir().unwrap();
        let res = handle_list(dir.path(), Some("nonexistent_agent"));
        assert!(res.is_err());
    }
}
