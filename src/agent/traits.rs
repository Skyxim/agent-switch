use crate::crypto::CryptoProvider;
use crate::model::{AccountUsageReport, AppUsageReport, QuotaGroup, QuotaWindow};
use crate::ui::{AccountTableItem, AccountTableOptions};
pub use crate::vault::CredentialStore;
use crate::vault::{Account, Store};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

/// Boxed pinned future for object-safe asynchronous operations.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// Re-export / alias usage types for standardized domain modeling
pub type UsageReport = AppUsageReport;
pub type UsageWindow = QuotaWindow;
pub type UsageGroup = QuotaGroup;
pub type AccountUsage = AccountUsageReport;

/// Strongly-typed identity parsed from agent credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub key: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

impl AccountIdentity {
    pub fn new(key: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            email: email.into(),
            alias: None,
            plan: None,
            mode: None,
        }
    }

    pub fn with_alias(mut self, alias: Option<impl Into<String>>) -> Self {
        self.alias = alias.map(Into::into);
        self
    }

    pub fn with_plan(mut self, plan: Option<impl Into<String>>) -> Self {
        self.plan = plan.map(Into::into);
        self
    }

    pub fn with_mode(mut self, mode: Option<impl Into<String>>) -> Self {
        self.mode = mode.map(Into::into);
        self
    }
}

/// An account entry formatted for listing and UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDisplayItem {
    pub index: usize,
    pub key: String,
    pub alias: Option<String>,
    pub email: String,
    pub plan: Option<String>,
    pub mode: Option<String>,
    pub is_active: bool,
    pub added_at: Option<DateTime<Utc>>,
    pub store_status: Option<String>,
}

impl AccountDisplayItem {
    pub fn to_table_item(&self) -> AccountTableItem {
        AccountTableItem {
            index: self.index,
            is_active: self.is_active,
            alias: self.alias.clone(),
            email: self.email.clone(),
            plan: self.plan.clone(),
            added_at: self.added_at.map(|dt| dt.format("%Y-%m-%d").to_string()),
            store_status: self.store_status.clone(),
        }
    }
}

/// Strong-typed model holding account listing data for an agent app (pure logic, no terminal printing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountListModel {
    pub app_id: String,
    pub app_display_name: String,
    pub active_key: Option<String>,
    pub accounts: Vec<AccountDisplayItem>,
}

impl AccountListModel {
    pub fn new(app_id: impl Into<String>, app_display_name: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            app_display_name: app_display_name.into(),
            active_key: None,
            accounts: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    /// Convert items to table display items.
    pub fn to_table_items(&self) -> Vec<AccountTableItem> {
        self.accounts.iter().map(|a| a.to_table_item()).collect()
    }

    /// Render table view of accounts using the UI module.
    pub fn render_table(&self, show_app_header: bool) -> String {
        let default_empty = format!(
            "No saved {} accounts. Use `ags {} add <name>` to add one.",
            self.app_display_name, self.app_id
        );
        let options = AccountTableOptions {
            app_display_name: Some(self.app_display_name.clone()),
            show_app_header,
            show_store_status: false,
            empty_message: Some(default_empty),
        };
        crate::ui::render_account_table(&self.to_table_items(), &options)
    }
}

/// Strong-typed information about the currently active or configured account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentAccountInfo {
    pub identity: AccountIdentity,
    pub in_registry: bool,
    pub is_active: bool,
    pub store_alias: Option<String>,
}

/// Core domain trait defining an agent application's pure logic operations.
pub trait AgentApp: Send + Sync {
    /// Agent identifier, e.g. "codex", "agy".
    fn id(&self) -> &str;

    /// User-facing display name, e.g. "Codex", "Antigravity".
    fn display_name(&self) -> &str;

    /// Read raw active credential data from the environment or active file.
    fn read_active_raw(&self) -> Result<Vec<u8>>;

    /// Write raw active credential data to the environment or active file.
    fn write_active_raw(&self, data: &[u8]) -> Result<()>;

    /// Parse account identity from raw credential bytes.
    fn parse_identity(&self, raw: &[u8]) -> Result<AccountIdentity>;

    /// Add or update an account snapshot in the vault store.
    fn add_account(
        &self,
        alias: Option<&str>,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<(Account, bool)>;

    /// Switch the active account by alias, email, or index query.
    fn switch_account(
        &self,
        query: &str,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<String>;

    /// Remove an account from the vault store by alias, email, or index query.
    fn remove_account(&self, query: &str, store: &Store) -> Result<String>;

    /// List all saved accounts as a pure data model.
    fn list_accounts(&self, store: &Store) -> Result<AccountListModel>;

    /// Query the currently active account as a pure data model.
    fn current_account(&self, store: &Store) -> Result<CurrentAccountInfo>;

    /// Fetch usage and quota reports asynchronously.
    fn fetch_usage<'a>(
        &'a self,
        store: &'a Store,
        crypto: &'a dyn CryptoProvider,
    ) -> BoxFuture<'a, Result<UsageReport>>;

    /// Return the default storage for this agent under a given base directory.
    fn default_store(&self, base_dir: &Path) -> Store {
        Store::new(base_dir.join(self.id()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_identity_builder() {
        let id = AccountIdentity::new("codex:123", "user@example.com")
            .with_alias(Some("work"))
            .with_plan(Some("team"))
            .with_mode(Some("oauth"));

        assert_eq!(id.key, "codex:123");
        assert_eq!(id.email, "user@example.com");
        assert_eq!(id.alias.as_deref(), Some("work"));
        assert_eq!(id.plan.as_deref(), Some("team"));
        assert_eq!(id.mode.as_deref(), Some("oauth"));
    }

    #[test]
    fn test_account_list_model_rendering() {
        let mut model = AccountListModel::new("codex", "Codex");
        assert!(model.is_empty());
        assert_eq!(model.len(), 0);

        model.accounts.push(AccountDisplayItem {
            index: 1,
            key: "codex:123".to_string(),
            alias: Some("primary".to_string()),
            email: "primary@example.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            is_active: true,
            added_at: Some(Utc::now()),
            store_status: Some("Stored".to_string()),
        });

        assert_eq!(model.len(), 1);
        let rendered = model.render_table(true);
        assert!(rendered.contains("Codex"));
        assert!(rendered.contains("primary@example.com"));
    }

    #[test]
    fn test_agent_default_store() {
        use std::path::PathBuf;
        struct DummyAgent;
        impl AgentApp for DummyAgent {
            fn id(&self) -> &str {
                "dummy"
            }
            fn display_name(&self) -> &str {
                "Dummy"
            }
            fn read_active_raw(&self) -> Result<Vec<u8>> {
                Ok(vec![])
            }
            fn write_active_raw(&self, _data: &[u8]) -> Result<()> {
                Ok(())
            }
            fn parse_identity(&self, _raw: &[u8]) -> Result<AccountIdentity> {
                Ok(AccountIdentity::new("dummy:1", "dummy@example.com"))
            }
            fn add_account(
                &self,
                _alias: Option<&str>,
                _store: &Store,
                _crypto: &dyn CryptoProvider,
            ) -> Result<(Account, bool)> {
                unimplemented!()
            }
            fn switch_account(
                &self,
                _query: &str,
                _store: &Store,
                _crypto: &dyn CryptoProvider,
            ) -> Result<String> {
                unimplemented!()
            }
            fn remove_account(&self, _query: &str, _store: &Store) -> Result<String> {
                unimplemented!()
            }
            fn list_accounts(&self, _store: &Store) -> Result<AccountListModel> {
                unimplemented!()
            }
            fn current_account(&self, _store: &Store) -> Result<CurrentAccountInfo> {
                unimplemented!()
            }
            fn fetch_usage<'a>(
                &'a self,
                _store: &'a Store,
                _crypto: &'a dyn CryptoProvider,
            ) -> BoxFuture<'a, Result<UsageReport>> {
                unimplemented!()
            }
        }

        let agent = DummyAgent;
        let base = PathBuf::from("/tmp/ags-test");
        let store = agent.default_store(&base);
        assert_eq!(store.dir(), base.join("dummy"));
    }
}
