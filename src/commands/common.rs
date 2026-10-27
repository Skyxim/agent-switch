use crate::agent::AgentApp;
use crate::crypto::CryptoProvider;
use crate::ui::AccountTableOptions;
use crate::vault::Store;
use anyhow::Result;
use colored::*;

/// Execute adding an account snapshot via AgentApp.
pub fn execute_add(
    agent: &dyn AgentApp,
    store: &Store,
    crypto: &dyn CryptoProvider,
    alias: Option<&str>,
) -> Result<()> {
    let (acc, is_updated) = agent.add_account(alias, store, crypto)?;
    let display_name = acc.alias.as_deref().unwrap_or("-");
    if is_updated {
        println!(
            "{} Updated existing account '{}' ({})",
            "✓".green().bold(),
            display_name.bold(),
            acc.email.cyan()
        );
    } else {
        println!(
            "{} Saved new account '{}' ({})",
            "✓".green().bold(),
            display_name.bold(),
            acc.email.cyan()
        );
    }
    Ok(())
}

/// Execute listing accounts via AgentApp and rendering table.
pub fn execute_list(agent: &dyn AgentApp, store: &Store, show_header: bool) -> Result<()> {
    let list_model = agent.list_accounts(store)?;
    let items = list_model.to_table_items();
    let default_empty = format!(
        "No saved {} accounts. Use `ags {} add <name>` to add one.",
        agent.display_name(),
        agent.id()
    );
    let options = AccountTableOptions {
        app_display_name: Some(agent.display_name().to_string()),
        show_app_header: show_header,
        show_store_status: false,
        empty_message: Some(default_empty),
    };
    let rendered = crate::ui::render_account_table(&items, &options);
    if !rendered.is_empty() {
        println!("{}", rendered);
    }
    Ok(())
}

/// Execute switching active account via AgentApp.
pub fn execute_switch(
    agent: &dyn AgentApp,
    store: &Store,
    crypto: &dyn CryptoProvider,
    query: &str,
) -> Result<()> {
    let msg = agent.switch_account(query, store, crypto)?;
    println!("{} {}", "✓".green().bold(), msg.green());
    Ok(())
}

/// Execute displaying current active account via AgentApp.
pub fn execute_current(agent: &dyn AgentApp, store: &Store) -> Result<()> {
    let info = agent.current_account(store)?;
    let header_title = if agent.id() == "agy" {
        "Current Antigravity (agy) Account:".bold()
    } else {
        format!("Current {} Account:", agent.display_name()).bold()
    };
    println!("{}", header_title);
    println!("  Email:       {}", info.identity.email.cyan());
    let alias = info
        .store_alias
        .as_deref()
        .or(info.identity.alias.as_deref())
        .unwrap_or("-");
    println!("  Alias:       {}", alias.yellow());
    if let Some(ref mode) = info.identity.mode {
        println!("  Mode:        {}", mode);
    }
    if let Some(ref plan) = info.identity.plan {
        println!("  Plan:        {}", plan);
    }
    println!("  Key:         {}", info.identity.key.dimmed());
    println!(
        "  In Registry: {}",
        if info.in_registry {
            "Yes".green()
        } else {
            "No (untracked)".red()
        }
    );
    println!(
        "  Is Active:   {}",
        if info.is_active {
            "Yes".green()
        } else {
            "No (key mismatch)".yellow()
        }
    );
    Ok(())
}

/// Execute removing an account via AgentApp.
pub fn execute_rm(agent: &dyn AgentApp, store: &Store, query: &str) -> Result<()> {
    let msg = agent.remove_account(query, store)?;
    println!("{} {}", "✓".green().bold(), msg.yellow());
    Ok(())
}

/// Execute querying and rendering quota usage via AgentApp.
pub async fn execute_usage(
    agent: &dyn AgentApp,
    store: &Store,
    crypto: &dyn CryptoProvider,
    show_header: bool,
) -> Result<()> {
    match agent.fetch_usage(store, crypto).await {
        Ok(report) => {
            crate::ui::progress::render_usage_report(&report, show_header);
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "{}: {}",
                format!("{} usage error", agent.display_name()).red(),
                e
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::traits::{
        AccountDisplayItem, AccountIdentity, AccountListModel, BoxFuture, CurrentAccountInfo,
        UsageReport,
    };
    use crate::crypto::plain::PlaintextCryptoProvider;
    use crate::vault::Account;
    use chrono::Utc;
    use tempfile::tempdir;

    struct MockAgent {
        id: String,
        name: String,
    }

    impl AgentApp for MockAgent {
        fn id(&self) -> &str {
            &self.id
        }

        fn display_name(&self) -> &str {
            &self.name
        }

        fn read_active_raw(&self) -> Result<Vec<u8>> {
            Ok(b"{}".to_vec())
        }

        fn write_active_raw(&self, _data: &[u8]) -> Result<()> {
            Ok(())
        }

        fn parse_identity(&self, _raw: &[u8]) -> Result<AccountIdentity> {
            Ok(AccountIdentity::new("mock:key", "mock@example.com"))
        }

        fn add_account(
            &self,
            alias: Option<&str>,
            _store: &Store,
            _crypto: &dyn CryptoProvider,
        ) -> Result<(Account, bool)> {
            Ok((
                Account {
                    key: "mock:key".to_string(),
                    alias: alias.map(String::from),
                    email: "mock@example.com".to_string(),
                    plan: Some("Pro".to_string()),
                    mode: None,
                    added_at: Utc::now(),
                    updated_at: None,
                },
                false,
            ))
        }

        fn switch_account(
            &self,
            query: &str,
            _store: &Store,
            _crypto: &dyn CryptoProvider,
        ) -> Result<String> {
            Ok(format!("Switched to {}", query))
        }

        fn remove_account(&self, query: &str, _store: &Store) -> Result<String> {
            Ok(format!("Removed account {}", query))
        }

        fn list_accounts(&self, _store: &Store) -> Result<AccountListModel> {
            let mut model = AccountListModel::new(&self.id, &self.name);
            model.accounts.push(AccountDisplayItem {
                index: 1,
                key: "mock:key".to_string(),
                alias: Some("primary".to_string()),
                email: "mock@example.com".to_string(),
                plan: Some("Pro".to_string()),
                mode: None,
                is_active: true,
                added_at: Some(Utc::now()),
                store_status: None,
            });
            Ok(model)
        }

        fn current_account(&self, _store: &Store) -> Result<CurrentAccountInfo> {
            Ok(CurrentAccountInfo {
                identity: AccountIdentity::new("mock:key", "mock@example.com")
                    .with_alias(Some("primary"))
                    .with_plan(Some("Pro")),
                in_registry: true,
                is_active: true,
                store_alias: Some("primary".to_string()),
            })
        }

        fn fetch_usage<'a>(
            &'a self,
            _store: &'a Store,
            _crypto: &'a dyn CryptoProvider,
        ) -> BoxFuture<'a, Result<UsageReport>> {
            Box::pin(async move { Ok(UsageReport::new("Mock")) })
        }
    }

    #[tokio::test]
    async fn test_common_agent_subcommand_dispatch() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;
        let agent = MockAgent {
            id: "mock".to_string(),
            name: "MockAgent".to_string(),
        };

        // Test add
        assert!(execute_add(&agent, &store, &crypto, Some("test_alias")).is_ok());

        // Test list
        assert!(execute_list(&agent, &store, true).is_ok());
        assert!(execute_list(&agent, &store, false).is_ok());

        // Test switch
        assert!(execute_switch(&agent, &store, &crypto, "test_alias").is_ok());

        // Test current
        assert!(execute_current(&agent, &store).is_ok());

        // Test remove
        assert!(execute_rm(&agent, &store, "test_alias").is_ok());

        // Test usage
        assert!(execute_usage(&agent, &store, &crypto, false).await.is_ok());
    }
}
