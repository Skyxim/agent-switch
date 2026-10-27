use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const REGISTRY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub added_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(default)]
    pub accounts: Vec<Account>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            active: None,
            previous: None,
            accounts: Vec::new(),
        }
    }
}

impl Registry {
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.accounts.iter().position(|a| a.key == key)
    }

    /// Find an account by exact key or case-insensitive email match
    pub fn find_by_key_or_email(&self, key: &str, email: &str) -> Option<usize> {
        self.accounts.iter().position(|a| {
            a.key == key || (!email.is_empty() && a.email.eq_ignore_ascii_case(email))
        })
    }

    /// Add a new account or update an existing one matching key or email.
    /// Returns `(is_update, Option<old_key_if_changed>)`.
    pub fn add_or_update(&mut self, mut account: Account) -> (bool, Option<String>) {
        if let Some(idx) = self.find_by_key_or_email(&account.key, &account.email) {
            let old = &self.accounts[idx];
            let old_key = old.key.clone();

            // Retain existing alias if new alias is None
            if account.alias.is_none() {
                account.alias = old.alias.clone();
            }
            // Retain existing plan if new plan is None
            if account.plan.is_none() {
                account.plan = old.plan.clone();
            }
            // Retain existing mode if new mode is None
            if account.mode.is_none() {
                account.mode = old.mode.clone();
            }

            // Retain original added_at timestamp and record updated_at
            account.added_at = old.added_at;
            account.updated_at = Some(Utc::now());

            let key_changed = old_key != account.key;
            if key_changed {
                if self.active.as_deref() == Some(&old_key) {
                    self.active = Some(account.key.clone());
                }
                if self.previous.as_deref() == Some(&old_key) {
                    self.previous = Some(account.key.clone());
                }
            }

            self.accounts[idx] = account;
            (true, if key_changed { Some(old_key) } else { None })
        } else {
            self.accounts.push(account);
            (false, None)
        }
    }

    pub fn upsert(&mut self, account: Account) {
        self.add_or_update(account);
    }

    pub fn remove(&mut self, key: &str) -> Option<Account> {
        let removed = if let Some(idx) = self.index_of(key) {
            Some(self.accounts.remove(idx))
        } else {
            None
        };

        if self.active.as_deref() == Some(key) {
            self.active = None;
        }
        if self.previous.as_deref() == Some(key) {
            self.previous = None;
        }
        removed
    }

    pub fn find(&self, query: &str) -> Result<usize> {
        let q = query.trim();
        if q.is_empty() {
            bail!("Search query cannot be empty");
        }

        // Special "-" switch to previous
        if q == "-" {
            if let Some(ref prev) = self.previous {
                if let Some(idx) = self.index_of(prev) {
                    return Ok(idx);
                }
            }
            bail!("No previous account recorded");
        }

        // 1-based index
        if let Ok(num) = q.parse::<usize>() {
            if num >= 1 && num <= self.accounts.len() {
                return Ok(num - 1);
            }
            bail!("Index {} is out of range (1..{})", num, self.accounts.len());
        }

        // Exact match by alias or email
        let mut exact_matches = Vec::new();
        for (idx, acc) in self.accounts.iter().enumerate() {
            if acc
                .alias
                .as_deref()
                .map(|a| a.eq_ignore_ascii_case(q))
                .unwrap_or(false)
                || acc.email.eq_ignore_ascii_case(q)
            {
                exact_matches.push(idx);
            }
        }
        if exact_matches.len() == 1 {
            return Ok(exact_matches[0]);
        }
        if exact_matches.len() > 1 {
            bail!("Ambiguous query '{}' matched multiple accounts exactly", q);
        }

        // Partial match (case-insensitive substring)
        let lq = q.to_lowercase();
        let mut partial_matches = Vec::new();
        for (idx, acc) in self.accounts.iter().enumerate() {
            let alias_match = acc
                .alias
                .as_deref()
                .map(|a| a.to_lowercase().contains(&lq))
                .unwrap_or(false);
            let email_match = acc.email.to_lowercase().contains(&lq);
            if alias_match || email_match {
                partial_matches.push(idx);
            }
        }

        if partial_matches.len() == 1 {
            return Ok(partial_matches[0]);
        }
        if partial_matches.len() > 1 {
            bail!(
                "Ambiguous query '{}' matched multiple accounts: {}",
                q,
                partial_matches.len()
            );
        }

        bail!("No account matching '{}' found", q);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_operations() {
        let mut reg = Registry::default();
        let acc1 = Account {
            key: "google:111".to_string(),
            alias: Some("personal".to_string()),
            email: "user1@gmail.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };
        let acc2 = Account {
            key: "google:222".to_string(),
            alias: Some("work".to_string()),
            email: "user2@company.com".to_string(),
            plan: Some("Ultra".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };

        reg.upsert(acc1.clone());
        reg.upsert(acc2.clone());
        assert_eq!(reg.accounts.len(), 2);

        // Find by 1-based index
        assert_eq!(reg.find("1").unwrap(), 0);
        assert_eq!(reg.find("2").unwrap(), 1);

        // Find by alias
        assert_eq!(reg.find("personal").unwrap(), 0);
        assert_eq!(reg.find("WORK").unwrap(), 1);

        // Find by email substring
        assert_eq!(reg.find("company").unwrap(), 1);

        // Ambiguous search
        let amb = reg.find("user");
        assert!(amb.is_err());

        // Previous
        reg.active = Some("google:222".to_string());
        reg.previous = Some("google:111".to_string());
        assert_eq!(reg.find("-").unwrap(), 0);

        // Remove
        reg.remove("google:111");
        assert_eq!(reg.accounts.len(), 1);
        assert_eq!(reg.previous, None);
    }

    #[test]
    fn test_add_or_update_by_email_case_insensitive() {
        let mut reg = Registry::default();
        let original_time = Utc::now();
        let acc1 = Account {
            key: "codex:sub-100".to_string(),
            alias: Some("primary".to_string()),
            email: "TestUser@Example.Com".to_string(),
            plan: Some("Plus".to_string()),
            mode: Some("oauth".to_string()),
            added_at: original_time,
            updated_at: None,
        };

        // First add: new account
        let (is_update, old_key) = reg.add_or_update(acc1);
        assert!(!is_update);
        assert_eq!(old_key, None);
        assert_eq!(reg.accounts.len(), 1);

        // Second add with same email in lowercase, new alias and new plan
        let acc2 = Account {
            key: "codex:sub-100".to_string(),
            alias: Some("new-alias".to_string()),
            email: "testuser@example.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };

        let (is_update, old_key) = reg.add_or_update(acc2);
        assert!(is_update);
        assert_eq!(old_key, None);
        assert_eq!(reg.accounts.len(), 1);
        assert_eq!(reg.accounts[0].alias.as_deref(), Some("new-alias"));
        assert_eq!(reg.accounts[0].plan.as_deref(), Some("Pro"));
        assert_eq!(reg.accounts[0].added_at, original_time);
        assert!(reg.accounts[0].updated_at.is_some());

        // Third add with None alias should keep existing alias
        let acc3 = Account {
            key: "codex:sub-100".to_string(),
            alias: None,
            email: "TESTUSER@EXAMPLE.COM".to_string(),
            plan: Some("Team".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };

        let (is_update, _) = reg.add_or_update(acc3);
        assert!(is_update);
        assert_eq!(reg.accounts.len(), 1);
        assert_eq!(reg.accounts[0].alias.as_deref(), Some("new-alias"));
        assert_eq!(reg.accounts[0].plan.as_deref(), Some("Team"));

        // Fourth add with changed key for the same email
        reg.active = Some("codex:sub-100".to_string());
        let acc4 = Account {
            key: "codex:sub-200".to_string(),
            alias: None,
            email: "testuser@example.com".to_string(),
            plan: Some("Team".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };

        let (is_update, old_key) = reg.add_or_update(acc4);
        assert!(is_update);
        assert_eq!(old_key, Some("codex:sub-100".to_string()));
        assert_eq!(reg.accounts.len(), 1);
        assert_eq!(reg.accounts[0].key, "codex:sub-200");
        assert_eq!(reg.active.as_deref(), Some("codex:sub-200"));
    }
}
