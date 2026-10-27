use crate::agent::dto::{CodexJwtClaims, CodexUsageResponse};
use crate::agent::traits::{
    AccountDisplayItem, AccountIdentity, AccountListModel, AgentApp, BoxFuture, CurrentAccountInfo,
    UsageReport,
};
use crate::crypto::CryptoProvider;
use crate::util::atomic_write;
use crate::vault::{Account, Store};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use colored::*;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Resolves the default path to the Codex auth file (`auth.json`):
/// 1. `CODEX_AUTH_PATH` (direct file path if set and non-empty)
/// 2. `CODEX_HOME/auth.json` (if `CODEX_HOME` is set and non-empty)
/// 3. `$HOME/.codex/auth.json` (via `dirs::home_dir()` or `HOME`)
///
/// Fallback: `.codex/auth.json`
pub fn default_auth_path() -> PathBuf {
    if let Ok(val) = env::var("CODEX_AUTH_PATH") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(val) = env::var("CODEX_HOME") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("auth.json");
        }
    }
    let home = dirs::home_dir().or_else(|| {
        env::var("HOME").ok().and_then(|h| {
            let trimmed = h.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        })
    });
    if let Some(user_home) = home {
        return user_home.join(".codex").join("auth.json");
    }
    PathBuf::from(".codex").join("auth.json")
}

pub struct CodexAgent {
    auth_path: PathBuf,
}

impl Default for CodexAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(default)]
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    #[serde(default)]
    tokens: Option<CodexTokens>,
    #[serde(default)]
    last_refresh: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CodexTokens {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

impl CodexAgent {
    pub fn new() -> Self {
        Self {
            auth_path: default_auth_path(),
        }
    }

    pub fn with_home(home: PathBuf) -> Self {
        Self {
            auth_path: home.join("auth.json"),
        }
    }

    pub fn with_auth_path(auth_path: PathBuf) -> Self {
        Self { auth_path }
    }

    pub fn default_auth_path() -> PathBuf {
        default_auth_path()
    }

    pub fn auth_path(&self) -> PathBuf {
        self.auth_path.clone()
    }

    /// Read raw auth.json content
    pub fn read_auth_raw(&self) -> Result<Vec<u8>> {
        let path = self.auth_path();
        if !path.exists() {
            bail!("Codex auth file not found at {}", path.display());
        }
        fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))
    }

    /// Parse identity from auth data
    pub fn parse_identity(data: &[u8]) -> Result<(String, String, Option<String>, Option<String>)> {
        let auth: CodexAuthFile =
            serde_json::from_slice(data).context("Failed to parse Codex auth.json schema")?;

        let mut email = String::new();
        let mut sub = String::new();
        let mut mode = "apikey".to_string();
        let mut plan = None;

        if let Some(tokens) = &auth.tokens {
            if tokens.access_token.is_some() {
                mode = "oauth".to_string();
            }
            if let Some(id_tok) = &tokens.id_token {
                if let Some(claims) = CodexJwtClaims::from_jwt(id_tok) {
                    if let Some(e) = claims.email {
                        email = e;
                    }
                    if let Some(s) = claims.sub {
                        sub = s;
                    }
                    if let Some(p) = claims.plan {
                        plan = Some(p);
                    }
                }
            }
        }

        if email.is_empty() {
            if auth.openai_api_key.is_some() {
                email = "api_key_user".to_string();
            } else {
                email = "unknown_codex_user".to_string();
            }
        }

        let key = if !sub.is_empty() {
            format!("codex:{}", sub)
        } else {
            format!("codex:{}", email)
        };

        Ok((key, email, plan, Some(mode)))
    }

    pub fn add(
        &self,
        alias: Option<&str>,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<(Account, bool)> {
        let raw_data = self.read_auth_raw()?;
        let (key, email, plan, mode) = Self::parse_identity(&raw_data)?;

        let mut reg = store.load_registry()?;
        let account = Account {
            key: key.clone(),
            alias: alias.map(|s| s.to_string()),
            email: email.clone(),
            plan,
            mode,
            added_at: Utc::now(),
            updated_at: None,
        };

        store.write_account(&key, &raw_data, crypto)?;
        let (is_update, old_key) = reg.add_or_update(account);
        if let Some(old_k) = old_key {
            if old_k != key {
                let _ = store.delete_account(&old_k);
            }
        }
        if reg.active.is_none() {
            reg.active = Some(key.clone());
        }
        store.save_registry(&reg)?;

        let saved_idx = reg
            .find_by_key_or_email(&key, &email)
            .context("Account not found in registry after saving")?;
        Ok((reg.accounts[saved_idx].clone(), is_update))
    }

    pub fn list(&self, store: &Store) -> Result<()> {
        self.list_with_header(store, false)
    }

    pub fn list_with_header(&self, store: &Store, show_app_header: bool) -> Result<()> {
        let reg = store.load_registry()?;
        if reg.accounts.is_empty() {
            if show_app_header {
                println!("{}", "Codex".bold().cyan());
                println!(
                    "  {}",
                    "No saved Codex accounts. Use `ags codex add <name>` to add one.".dimmed()
                );
            } else {
                println!(
                    "{}",
                    "No Codex accounts saved yet. Use `ags codex add <name>` to add one.".dimmed()
                );
            }
            return Ok(());
        }

        if show_app_header {
            println!("{}", "Codex".bold().cyan());
        }
        println!(
            "{:<3}   {:<15} {:<30} {:<18} {:<10}",
            "#", "Alias", "Email", "Plan", "Added"
        );
        println!("{}", "-".repeat(78).dimmed());

        for (idx, acc) in reg.accounts.iter().enumerate() {
            let is_active = reg.active.as_deref() == Some(&acc.key);
            let active_marker = if is_active {
                "*".green().bold().to_string()
            } else {
                " ".to_string()
            };
            let alias_str = acc.alias.as_deref().unwrap_or("-");
            let plan_str = acc.plan.as_deref().unwrap_or("-");
            let added_str = acc.added_at.format("%Y-%m-%d").to_string();

            let padded_alias = format!("{:<15}", alias_str);
            let padded_email = format!("{:<30}", acc.email);

            if is_active {
                println!(
                    "{:<3} {} {} {} {:<18} {:<10}",
                    idx + 1,
                    active_marker,
                    padded_alias.green().bold(),
                    padded_email.green(),
                    plan_str,
                    added_str.dimmed()
                );
            } else {
                println!(
                    "{:<3} {} {} {} {:<18} {:<10}",
                    idx + 1,
                    active_marker,
                    padded_alias,
                    padded_email,
                    plan_str,
                    added_str.dimmed()
                );
            }
        }
        Ok(())
    }

    pub fn switch(
        &self,
        query: &str,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<String> {
        let mut reg = store.load_registry()?;
        let idx = reg.find(query)?;
        let target_account = reg.accounts[idx].clone();

        // 1. If current auth file exists, back it up
        if let Ok(current_data) = self.read_auth_raw() {
            let backup_label = reg.active.as_deref().unwrap_or("current");
            let _ = store.backup_file(&current_data, backup_label);
        }

        // 2. Read and decrypt target account snapshot
        let target_data = store.read_account(&target_account.key, crypto)?;

        // 3. Atomically write to auth.json
        atomic_write(&self.auth_path(), &target_data)?;

        // 4. Update registry
        reg.previous = reg.active.take();
        reg.active = Some(target_account.key.clone());
        store.save_registry(&reg)?;

        let display_name = target_account
            .alias
            .as_deref()
            .unwrap_or(&target_account.email);
        Ok(format!(
            "Switched Codex to '{}' ({})",
            display_name, target_account.email
        ))
    }

    pub fn current(&self, store: &Store) -> Result<()> {
        let raw = self.read_auth_raw()?;
        let (key, email, plan, mode) = Self::parse_identity(&raw)?;
        let reg = store.load_registry()?;

        let matched_account = reg.accounts.iter().find(|a| a.key == key);
        let alias = matched_account
            .and_then(|a| a.alias.as_deref())
            .unwrap_or("-");
        let active_match = reg.active.as_deref() == Some(&key);

        println!("{}", "Current Codex Account:".bold());
        println!("  Email:       {}", email.cyan());
        println!("  Alias:       {}", alias.yellow());
        println!("  Mode:        {}", mode.unwrap_or_else(|| "-".to_string()));
        println!("  Plan:        {}", plan.unwrap_or_else(|| "-".to_string()));
        println!("  Key:         {}", key.dimmed());
        println!(
            "  In Registry: {}",
            if matched_account.is_some() {
                "Yes".green()
            } else {
                "No (untracked)".red()
            }
        );
        println!(
            "  Is Active:   {}",
            if active_match {
                "Yes".green()
            } else {
                "No".yellow()
            }
        );
        Ok(())
    }

    pub fn remove(&self, query: &str, store: &Store) -> Result<String> {
        let mut reg = store.load_registry()?;
        let idx = reg.find(query)?;
        let key = reg.accounts[idx].key.clone();
        let alias = reg.accounts[idx].alias.clone();
        let email = reg.accounts[idx].email.clone();

        store.delete_account(&key)?;
        reg.remove(&key);
        store.save_registry(&reg)?;

        let name = alias.as_deref().unwrap_or(&email);
        Ok(format!("Removed Codex account '{}' ({})", name, email))
    }

    pub async fn usage(&self, store: &Store, crypto: &dyn CryptoProvider) -> Result<()> {
        let report = self.collect_usage_report(store, crypto).await?;
        crate::render::render_usage_report(&report, false);
        Ok(())
    }

    pub async fn collect_usage_report(
        &self,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<crate::model::AppUsageReport> {
        let reg = store.load_registry()?;
        let mut report = crate::model::AppUsageReport::new("Codex");
        if reg.accounts.is_empty() {
            return Ok(report);
        }

        let reg_active = reg.active.clone();
        let client = reqwest::Client::new();
        let tasks = reg.accounts.into_iter().enumerate().map(|(idx, acc)| {
            let client = client.clone();
            let is_active = reg_active.as_deref() == Some(&acc.key);

            let cred_res: Result<(String, Option<String>)> = (|| {
                let data = if is_active {
                    if let Ok(raw) = self.read_auth_raw() {
                        if let Ok((k, e, _, _)) = Self::parse_identity(&raw) {
                            if k == acc.key || e.eq_ignore_ascii_case(&acc.email) {
                                raw
                            } else {
                                store.read_account(&acc.key, crypto)?
                            }
                        } else {
                            store.read_account(&acc.key, crypto)?
                        }
                    } else {
                        store.read_account(&acc.key, crypto)?
                    }
                } else {
                    store.read_account(&acc.key, crypto)?
                };
                Self::extract_token_info(&data)
            })();

            async move {
                let result = match cred_res {
                    Ok((access_tok, aid)) => {
                        Self::fetch_single_account_quota(&client, &access_tok, aid.as_deref()).await
                    }
                    Err(e) => Err(e),
                };
                (idx, acc, is_active, result)
            }
        });

        use futures_util::StreamExt;
        let mut results: Vec<_> = futures_util::stream::iter(tasks)
            .buffer_unordered(4)
            .collect()
            .await;

        results.sort_by_key(|(idx, _, _, _)| *idx);

        for (idx, acc, is_active, res) in results {
            let mut acc_report = crate::model::AccountUsageReport::new(idx + 1, &acc.email);
            acc_report.alias = acc.alias;
            acc_report.is_active = is_active;

            match res {
                Ok(val) => {
                    let plan = val
                        .plan_type
                        .as_deref()
                        .map(|s| s.to_uppercase())
                        .or(acc.plan);
                    acc_report.tier = plan;

                    let quotas = val.parse_quota_windows();
                    if !quotas.is_empty() {
                        acc_report
                            .groups
                            .push(crate::model::QuotaGroup { name: None, quotas });
                    }
                }
                Err(e) => {
                    acc_report.tier = acc.plan;
                    acc_report.error = Some(e.to_string());
                }
            }
            report.accounts.push(acc_report);
        }

        Ok(report)
    }

    pub fn parse_codex_quota_windows(val: &serde_json::Value) -> Vec<crate::model::QuotaWindow> {
        let mut quotas = Vec::new();

        if let Some(rate_limit) = val.get("rate_limit").filter(|v| !v.is_null()) {
            if let Some(primary) = rate_limit
                .get("primary_window")
                .filter(|w| Self::is_valid_window(w))
            {
                if let Some(qw) = Self::build_codex_window(primary) {
                    quotas.push(qw);
                }
            }
            if let Some(secondary) = rate_limit
                .get("secondary_window")
                .filter(|w| Self::is_valid_window(w))
            {
                if let Some(qw) = Self::build_codex_window(secondary) {
                    quotas.push(qw);
                }
            }
        }

        quotas.sort_by_key(|q| match q.window_type.as_str() {
            "5h" => 0,
            "daily" => 1,
            "weekly" => 2,
            "monthly" => 3,
            _ => 4,
        });

        quotas
    }

    fn build_codex_window(window: &serde_json::Value) -> Option<crate::model::QuotaWindow> {
        let used_pct = window
            .get("used_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .clamp(0.0, 100.0);
        let rem_pct = (100.0 - used_pct).clamp(0.0, 100.0);

        let secs_opt = window
            .get("limit_window_seconds")
            .or_else(|| window.get("window_seconds"))
            .and_then(|v| v.as_i64());

        let (window_type, display_label) = match secs_opt {
            Some(18000) => ("5h".to_string(), "5h".to_string()),
            Some(604800) => ("weekly".to_string(), "Weekly".to_string()),
            Some(86400) => ("daily".to_string(), "Daily".to_string()),
            Some(2592000) => ("monthly".to_string(), "Monthly".to_string()),
            Some(s) if s > 0 && s % 86400 == 0 => {
                let d = s / 86400;
                (format!("{}d", d), format!("{}-Day", d))
            }
            Some(s) if s > 0 && s % 3600 == 0 => {
                let h = s / 3600;
                (format!("{}h", h), format!("{}-Hour", h))
            }
            Some(s) if s > 0 => (format!("{}s", s), format!("{}-Sec", s)),
            _ => ("window".to_string(), "Window".to_string()),
        };

        let reset_after_s = window.get("reset_after_seconds").and_then(|v| v.as_i64());

        let resets_in_text = reset_after_s.map(crate::model::format_resets_in_seconds);
        let reset_time =
            reset_after_s.map(|s| (Utc::now() + chrono::Duration::seconds(s)).to_rfc3339());

        Some(crate::model::QuotaWindow::new(
            window_type,
            display_label,
            rem_pct,
            used_pct,
            reset_time,
            resets_in_text,
        ))
    }

    async fn fetch_single_account_quota(
        client: &reqwest::Client,
        access_token: &str,
        account_id: Option<&str>,
    ) -> Result<CodexUsageResponse> {
        let mut req = client
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("User-Agent", concat!("ags/", env!("CARGO_PKG_VERSION")));

        if let Some(aid) = account_id {
            req = req.header("ChatGPT-Account-ID", aid);
        }

        let resp = req
            .send()
            .await
            .context("Failed to query Codex usage endpoint")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                bail!(
                    "Codex usage API returned 401 Unauthorized: token has expired or session is invalid."
                );
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                bail!("Codex usage API rate limit reached (HTTP 429). Please try again later.");
            }
            bail!("Codex usage API returned {}: {}", status, body);
        }

        let usage_resp: CodexUsageResponse = resp
            .json()
            .await
            .context("Failed to parse Codex usage response")?;
        Ok(usage_resp)
    }

    fn extract_token_info(data: &[u8]) -> Result<(String, Option<String>)> {
        let auth: CodexAuthFile = serde_json::from_slice(data)?;
        let tokens = auth.tokens.context("No tokens found in auth file. If using API key mode, usage statistics are not supported by ChatGPT backend.")?;
        let access = tokens
            .access_token
            .context("No access token found in auth file. Please run `codex login`.")?;
        Ok((access, tokens.account_id))
    }

    pub fn display_usage_json(val: &serde_json::Value) {
        println!("{}", "Codex Quotas & Usage:".bold().cyan());
        Self::render_codex_account_quota(val);
    }

    pub fn render_codex_account_quota(val: &serde_json::Value) {
        if let Some(plan) = val.get("plan_type").and_then(|v| v.as_str()) {
            println!("  Plan: {}", plan.to_uppercase().yellow().bold());
        }

        let mut rendered_any = false;
        if let Some(rate_limit) = val.get("rate_limit").filter(|v| !v.is_null()) {
            if let Some(primary) = rate_limit
                .get("primary_window")
                .filter(|w| Self::is_valid_window(w))
            {
                let label = Self::derive_window_label("Primary Window", primary);
                Self::render_window(&label, primary);
                rendered_any = true;
            }
            if let Some(secondary) = rate_limit
                .get("secondary_window")
                .filter(|w| Self::is_valid_window(w))
            {
                let label = Self::derive_window_label("Secondary Window", secondary);
                Self::render_window(&label, secondary);
                rendered_any = true;
            }
        }

        if !rendered_any {
            println!(
                "  {}",
                "No specific rate limits or usage windows found for this account.".dimmed()
            );
        }
    }

    pub fn derive_window_label(fallback: &str, window: &serde_json::Value) -> String {
        let secs_opt = window
            .get("limit_window_seconds")
            .or_else(|| window.get("window_seconds"))
            .and_then(|v| v.as_i64());

        if let Some(secs) = secs_opt {
            if secs == 18000 {
                "5-Hour Window".to_string()
            } else if secs == 604800 {
                "Weekly Window".to_string()
            } else if secs == 86400 {
                "Daily Window".to_string()
            } else if secs > 0 && secs % 86400 == 0 {
                format!("{}-Day Window", secs / 86400)
            } else if secs > 0 && secs % 3600 == 0 {
                format!("{}-Hour Window", secs / 3600)
            } else if secs > 0 && secs % 60 == 0 {
                format!("{}-Minute Window", secs / 60)
            } else if secs > 0 {
                format!("{}-Second Window", secs)
            } else {
                fallback.to_string()
            }
        } else {
            fallback.to_string()
        }
    }

    pub fn is_valid_window(window: &serde_json::Value) -> bool {
        if window.is_null() {
            return false;
        }
        window.get("used_percent").is_some()
            || window.get("reset_after_seconds").is_some()
            || window.get("limit_window_seconds").is_some()
    }

    fn render_window(label: &str, window: &serde_json::Value) {
        let used_pct = window
            .get("used_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let reset_after_s = window
            .get("reset_after_seconds")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let bar_width = 25;
        let filled = ((used_pct / 100.0) * bar_width as f64).round() as usize;
        let filled = filled.min(bar_width);
        let empty = bar_width - filled;

        let bar_str = format!("[{}{}]", "=".repeat(filled), " ".repeat(empty));
        let colored_bar = if used_pct >= 80.0 {
            bar_str.red().bold()
        } else if used_pct >= 50.0 {
            bar_str.yellow()
        } else {
            bar_str.green()
        };

        let days = reset_after_s / 86400;
        let rem_s = reset_after_s % 86400;
        let hours = rem_s / 3600;
        let mins = (rem_s % 3600) / 60;
        let reset_str = if days > 0 {
            format!("resets in {}d {}h", days, hours)
        } else if hours > 0 {
            format!("resets in {}h {}m", hours, mins)
        } else if mins > 0 {
            format!("resets in {}m", mins)
        } else {
            "resets soon".to_string()
        };

        println!(
            "  {:<18} {} {:>5.1}% ({})",
            label.bold(),
            colored_bar,
            used_pct,
            reset_str.dimmed()
        );
    }
}

impl AgentApp for CodexAgent {
    fn id(&self) -> &str {
        "codex"
    }

    fn display_name(&self) -> &str {
        "Codex"
    }

    fn read_active_raw(&self) -> Result<Vec<u8>> {
        self.read_auth_raw()
    }

    fn write_active_raw(&self, data: &[u8]) -> Result<()> {
        atomic_write(&self.auth_path(), data)
    }

    fn parse_identity(&self, raw: &[u8]) -> Result<AccountIdentity> {
        let (key, email, plan, mode) = Self::parse_identity(raw)?;
        Ok(AccountIdentity {
            key,
            email,
            alias: None,
            plan,
            mode,
        })
    }

    fn add_account(
        &self,
        alias: Option<&str>,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<(Account, bool)> {
        self.add(alias, store, crypto)
    }

    fn switch_account(
        &self,
        query: &str,
        store: &Store,
        crypto: &dyn CryptoProvider,
    ) -> Result<String> {
        self.switch(query, store, crypto)
    }

    fn remove_account(&self, query: &str, store: &Store) -> Result<String> {
        self.remove(query, store)
    }

    fn list_accounts(&self, store: &Store) -> Result<AccountListModel> {
        let reg = store.load_registry()?;
        let mut model = AccountListModel::new(self.id(), self.display_name());
        model.active_key = reg.active.clone();

        for (idx, acc) in reg.accounts.iter().enumerate() {
            let is_active = reg.active.as_deref() == Some(&acc.key);
            model.accounts.push(AccountDisplayItem {
                index: idx + 1,
                key: acc.key.clone(),
                alias: acc.alias.clone(),
                email: acc.email.clone(),
                plan: acc.plan.clone(),
                mode: acc.mode.clone(),
                is_active,
                added_at: Some(acc.added_at),
                store_status: Some("Stored".to_string()),
            });
        }
        Ok(model)
    }

    fn current_account(&self, store: &Store) -> Result<CurrentAccountInfo> {
        let raw = self.read_active_raw()?;
        let identity = self.parse_identity(&raw)?;
        let reg = store.load_registry()?;

        let matched = reg
            .accounts
            .iter()
            .find(|a| a.key == identity.key || a.email.eq_ignore_ascii_case(&identity.email));
        let in_registry = matched.is_some();
        let is_active = reg.active.as_deref() == Some(&identity.key);
        let store_alias = matched.and_then(|a| a.alias.clone());

        Ok(CurrentAccountInfo {
            identity,
            in_registry,
            is_active,
            store_alias,
        })
    }

    fn fetch_usage<'a>(
        &'a self,
        store: &'a Store,
        crypto: &'a dyn CryptoProvider,
    ) -> BoxFuture<'a, Result<UsageReport>> {
        Box::pin(async move { self.collect_usage_report(store, crypto).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn test_codex_identity_and_switch() {
        let dir = tempdir().unwrap();
        let codex_home = dir.path().join("codex_home");
        fs::create_dir_all(&codex_home).unwrap();
        let agent = CodexAgent::with_home(codex_home.clone());

        let fake_auth = r#"{
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ1c2VyLTEyMyIsImVtYWlsIjoidXNlckBleGFtcGxlLmNvbSIsInBsYW4iOiJQcm8ifQ.fake",
                "access_token": "acc-token-123",
                "refresh_token": "ref-token-123"
            }
        }"#;

        fs::write(agent.auth_path(), fake_auth).unwrap();

        let store_dir = dir.path().join("agent_store");
        let store = Store::new(store_dir);
        let crypto = PlaintextCryptoProvider;

        // Add
        let (acc, is_update) = agent.add(Some("my-account"), &store, &crypto).unwrap();
        assert!(!is_update);
        assert_eq!(acc.email, "user@example.com");
        assert_eq!(acc.alias.as_deref(), Some("my-account"));

        // Check registry
        let reg = store.load_registry().unwrap();
        assert_eq!(reg.accounts.len(), 1);
        assert_eq!(reg.active.as_deref(), Some("codex:user-123"));

        // Add again with updated alias
        let (acc_updated, is_update2) =
            agent.add(Some("updated-account"), &store, &crypto).unwrap();
        assert!(is_update2);
        assert_eq!(acc_updated.email, "user@example.com");
        assert_eq!(acc_updated.alias.as_deref(), Some("updated-account"));
        let reg2 = store.load_registry().unwrap();
        assert_eq!(reg2.accounts.len(), 1);
        assert!(reg2.accounts[0].updated_at.is_some());

        // Switch to self by alias
        let msg = agent.switch("updated-account", &store, &crypto).unwrap();
        assert!(msg.contains("Switched Codex to 'updated-account'"));
    }

    #[test]
    fn test_codex_token_info_extraction() {
        let fake_oauth = r#"{
            "tokens": {
                "access_token": "acc-456",
                "account_id": "org-789"
            }
        }"#;
        let (acc, aid) = CodexAgent::extract_token_info(fake_oauth.as_bytes()).unwrap();
        assert_eq!(acc, "acc-456");
        assert_eq!(aid.as_deref(), Some("org-789"));

        let fake_apikey_only = r#"{
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "sk-fake"
        }"#;
        let err = CodexAgent::extract_token_info(fake_apikey_only.as_bytes()).unwrap_err();
        assert!(err.to_string().contains("API key mode"));
    }

    #[test]
    fn test_derive_window_label() {
        // 5 hours = 18000s
        let win_5h = serde_json::json!({ "limit_window_seconds": 18000 });
        assert_eq!(
            CodexAgent::derive_window_label("Primary Window", &win_5h),
            "5-Hour Window"
        );

        // Weekly = 604800s
        let win_weekly = serde_json::json!({ "limit_window_seconds": 604800 });
        assert_eq!(
            CodexAgent::derive_window_label("Secondary Window", &win_weekly),
            "Weekly Window"
        );

        // Daily = 86400s
        let win_daily = serde_json::json!({ "limit_window_seconds": 86400 });
        assert_eq!(
            CodexAgent::derive_window_label("Primary Window", &win_daily),
            "Daily Window"
        );

        // 2-Day = 172800s
        let win_2d = serde_json::json!({ "limit_window_seconds": 172800 });
        assert_eq!(
            CodexAgent::derive_window_label("Primary Window", &win_2d),
            "2-Day Window"
        );

        // 10-Hour = 36000s
        let win_10h = serde_json::json!({ "limit_window_seconds": 36000 });
        assert_eq!(
            CodexAgent::derive_window_label("Primary Window", &win_10h),
            "10-Hour Window"
        );

        // Fallback when missing
        let win_none = serde_json::json!({});
        assert_eq!(
            CodexAgent::derive_window_label("Fallback Window", &win_none),
            "Fallback Window"
        );
    }

    #[test]
    fn test_display_usage_json_various_accounts() {
        // 1. Account with standard 5h and weekly
        let full_val = serde_json::json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 12.5,
                    "reset_after_seconds": 7200,
                    "limit_window_seconds": 18000
                },
                "secondary_window": {
                    "used_percent": 45.0,
                    "reset_after_seconds": 86400,
                    "limit_window_seconds": 604800
                }
            }
        });
        CodexAgent::display_usage_json(&full_val);

        // 2. Account without 5h window (e.g. only Daily window)
        let single_daily = serde_json::json!({
            "plan_type": "team",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 30.0,
                    "reset_after_seconds": 3600,
                    "limit_window_seconds": 86400
                }
            }
        });
        CodexAgent::display_usage_json(&single_daily);

        // 3. Account with no rate_limit or unlimited/empty
        let empty_val = serde_json::json!({
            "plan_type": "enterprise",
            "rate_limit": {}
        });
        CodexAgent::display_usage_json(&empty_val);
    }

    #[tokio::test]
    async fn test_codex_usage_empty_registry() {
        let dir = tempdir().unwrap();
        let codex_home = dir.path().join("codex_home");
        let agent = CodexAgent::with_home(codex_home);
        let store = Store::new(dir.path().join("store"));
        let crypto = PlaintextCryptoProvider;

        // Must succeed without errors on empty registry
        let res = agent.usage(&store, &crypto).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_codex_usage_multiple_accounts_handles_errors_gracefully() {
        let dir = tempdir().unwrap();
        let codex_home = dir.path().join("codex_home");
        let agent = CodexAgent::with_home(codex_home);
        let store = Store::new(dir.path().join("store"));
        let crypto = PlaintextCryptoProvider;

        let mut reg = store.load_registry().unwrap();
        // Account 1: Missing snapshot file in store
        let acc1 = Account {
            key: "codex:user-1".to_string(),
            alias: Some("account-1".to_string()),
            email: "user1@example.com".to_string(),
            plan: Some("Pro".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };
        // Account 2: Invalid JSON in snapshot
        let acc2 = Account {
            key: "codex:user-2".to_string(),
            alias: Some("account-2".to_string()),
            email: "user2@example.com".to_string(),
            plan: Some("Plus".to_string()),
            mode: Some("oauth".to_string()),
            added_at: Utc::now(),
            updated_at: None,
        };
        reg.accounts.push(acc1);
        reg.accounts.push(acc2);
        reg.active = Some("codex:user-1".to_string());
        store.save_registry(&reg).unwrap();

        // Write corrupt snapshot for user-2
        store
            .write_account("codex:user-2", b"not-a-json", &crypto)
            .unwrap();

        // Must not bail/panic; both errors should be handled per-account
        let res = agent.usage(&store, &crypto).await;
        assert!(res.is_ok());
    }

    use crate::util::TEST_ENV_LOCK as ENV_LOCK;

    struct EnvGuard {
        saved_auth_path: Option<String>,
        saved_codex_home: Option<String>,
        saved_home: Option<String>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                saved_auth_path: env::var("CODEX_AUTH_PATH").ok(),
                saved_codex_home: env::var("CODEX_HOME").ok(),
                saved_home: env::var("HOME").ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.saved_auth_path {
                Some(val) => unsafe { env::set_var("CODEX_AUTH_PATH", val) },
                None => unsafe { env::remove_var("CODEX_AUTH_PATH") },
            }
            match &self.saved_codex_home {
                Some(val) => unsafe { env::set_var("CODEX_HOME", val) },
                None => unsafe { env::remove_var("CODEX_HOME") },
            }
            match &self.saved_home {
                Some(val) => unsafe { env::set_var("HOME", val) },
                None => unsafe { env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn test_codex_auth_path_priorities() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guard = EnvGuard::new();

        unsafe {
            env::remove_var("CODEX_AUTH_PATH");
            env::remove_var("CODEX_HOME");
        }

        // Priority 3: standard home
        let path = default_auth_path();
        assert!(path.ends_with(Path::new(".codex").join("auth.json")));
        let agent = CodexAgent::new();
        assert_eq!(agent.auth_path(), path);

        // Priority 2: CODEX_HOME overrides standard home
        unsafe {
            env::set_var("CODEX_HOME", "   /tmp/custom-codex   ");
        }
        assert_eq!(
            default_auth_path(),
            PathBuf::from("/tmp/custom-codex/auth.json")
        );
        let agent2 = CodexAgent::new();
        assert_eq!(
            agent2.auth_path(),
            PathBuf::from("/tmp/custom-codex/auth.json")
        );

        // Priority 1: CODEX_AUTH_PATH overrides CODEX_HOME
        unsafe {
            env::set_var("CODEX_AUTH_PATH", "   /tmp/explicit/my-auth.json   ");
        }
        assert_eq!(
            default_auth_path(),
            PathBuf::from("/tmp/explicit/my-auth.json")
        );
        let agent3 = CodexAgent::new();
        assert_eq!(
            agent3.auth_path(),
            PathBuf::from("/tmp/explicit/my-auth.json")
        );
    }

    #[test]
    fn test_codex_agent_app_trait_implementation() {
        let dir = tempdir().unwrap();
        let codex_home = dir.path().join("codex_home");
        fs::create_dir_all(&codex_home).unwrap();
        let agent = CodexAgent::with_home(codex_home.clone());
        let store = Store::new(dir.path().join("store"));
        let crypto = PlaintextCryptoProvider;

        // Test metadata
        assert_eq!(agent.id(), "codex");
        assert_eq!(agent.display_name(), "Codex");

        // Write active auth
        let fake_auth = r#"{
            "tokens": {
                "id_token": "eyJhbGciOiJub25lIn0.eyJzdWIiOiJzdWItOTk5IiwiZW1haWwiOiJhcHBAZXhhbXBsZS5jb20iLCJwbGFuIjoidGVhbSJ9.sig",
                "access_token": "acc-999"
            }
        }"#;
        agent.write_active_raw(fake_auth.as_bytes()).unwrap();

        // Read active raw
        let raw = agent.read_active_raw().unwrap();
        assert_eq!(raw, fake_auth.as_bytes());

        // Parse identity
        let identity = agent.parse_identity(&raw).unwrap();
        assert_eq!(identity.key, "codex:sub-999");
        assert_eq!(identity.email, "app@example.com");
        assert_eq!(identity.plan.as_deref(), Some("team"));

        // Add account via trait
        let (acc, is_update) = agent
            .add_account(Some("app_alias"), &store, &crypto)
            .unwrap();
        assert!(!is_update);
        assert_eq!(acc.alias.as_deref(), Some("app_alias"));

        // List accounts via trait (pure data model)
        let list_model = agent.list_accounts(&store).unwrap();
        assert_eq!(list_model.app_id, "codex");
        assert_eq!(list_model.len(), 1);
        assert_eq!(list_model.accounts[0].email, "app@example.com");
        assert!(list_model.accounts[0].is_active);

        // Current account via trait
        let current = agent.current_account(&store).unwrap();
        assert!(current.in_registry);
        assert!(current.is_active);
        assert_eq!(current.store_alias.as_deref(), Some("app_alias"));

        // Switch account
        let switch_msg = agent.switch_account("app_alias", &store, &crypto).unwrap();
        assert!(switch_msg.contains("Switched Codex"));

        // Remove account
        let rm_msg = agent.remove_account("app_alias", &store).unwrap();
        assert!(rm_msg.contains("Removed Codex account"));
        assert!(agent.list_accounts(&store).unwrap().is_empty());
    }
}
