use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppUsageReport {
    /// Application name, e.g. "Codex", "Antigravity"
    pub app_name: String,
    /// List of account usage reports for this application
    pub accounts: Vec<AccountUsageReport>,
}

impl AppUsageReport {
    pub fn new(app_name: impl Into<String>) -> Self {
        Self {
            app_name: app_name.into(),
            accounts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountUsageReport {
    /// 1-based sequential index within the application
    pub index: usize,
    /// Account alias if configured
    pub alias: Option<String>,
    /// Account email
    pub email: String,
    /// Plan or tier (e.g. "TEAM", "Google AI Pro", "FREE", etc.)
    pub tier: Option<String>,
    /// Whether this is the active account in the application
    pub is_active: bool,
    /// Error message if quota retrieval failed for this account
    pub error: Option<String>,
    /// Grouped quota windows
    pub groups: Vec<QuotaGroup>,
}

impl AccountUsageReport {
    pub fn new(index: usize, email: impl Into<String>) -> Self {
        Self {
            index,
            alias: None,
            email: email.into(),
            tier: None,
            is_active: false,
            error: None,
            groups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaGroup {
    /// Optional group name, e.g. "Gemini", "Claude & GPT", or None for ungrouped (e.g. Codex)
    pub name: Option<String>,
    /// List of quota windows in this group
    pub quotas: Vec<QuotaWindow>,
}

impl QuotaGroup {
    pub fn new(name: Option<impl Into<String>>) -> Self {
        Self {
            name: name.map(Into::into),
            quotas: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuotaWindow {
    /// Window type, e.g. "5h", "weekly", "monthly", "daily"
    pub window_type: String,
    /// Display label for rendering, e.g. "5h", "Weekly", "5-Hour Window"
    pub display_label: String,
    /// Remaining quota percentage (0.0 ~ 100.0)
    pub remaining_percent: f64,
    /// Used quota percentage (0.0 ~ 100.0)
    pub used_percent: f64,
    /// Original RFC3339 reset time if provided by API
    pub reset_time: Option<String>,
    /// Humanized relative time string, e.g. "resets in 4h 31m"
    pub resets_in_text: Option<String>,
}

impl QuotaWindow {
    pub fn new(
        window_type: impl Into<String>,
        display_label: impl Into<String>,
        remaining_percent: f64,
        used_percent: f64,
        reset_time: Option<String>,
        resets_in_text: Option<String>,
    ) -> Self {
        Self {
            window_type: window_type.into(),
            display_label: display_label.into(),
            remaining_percent: remaining_percent.clamp(0.0, 100.0),
            used_percent: used_percent.clamp(0.0, 100.0),
            reset_time,
            resets_in_text,
        }
    }
}

/// Convert duration in seconds to a humanized "resets in X" string.
pub fn format_resets_in_seconds(secs: i64) -> String {
    if secs <= 0 {
        return "resets soon".to_string();
    }

    let days = secs / 86400;
    let rem_s = secs % 86400;
    let hours = rem_s / 3600;
    let mins = (rem_s % 3600) / 60;

    if days > 0 {
        if hours > 0 {
            format!("resets in {}d {}h", days, hours)
        } else {
            format!("resets in {}d", days)
        }
    } else if hours > 0 {
        if mins > 0 {
            format!("resets in {}h {}m", hours, mins)
        } else {
            format!("resets in {}h", hours)
        }
    } else if mins > 0 {
        format!("resets in {}m", mins)
    } else {
        "resets soon".to_string()
    }
}

/// Convert an RFC3339 timestamp to a humanized "resets in X" relative string.
pub fn format_resets_in_rfc3339(reset_time: &str) -> Option<String> {
    if reset_time.trim().is_empty() {
        return None;
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(reset_time) {
        let now = Utc::now();
        let diff = dt.with_timezone(&Utc) - now;
        let secs = diff.num_seconds();
        if secs > 0 {
            Some(format_resets_in_seconds(secs))
        } else {
            Some("resets soon".to_string())
        }
    } else {
        Some(format!("resets at {}", reset_time))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_serialization_and_deserialization() {
        let mut report = AppUsageReport::new("Antigravity");
        let mut acc = AccountUsageReport::new(1, "test@example.com");
        acc.alias = Some("work".to_string());
        acc.tier = Some("Google AI Pro".to_string());
        acc.is_active = true;

        let mut group = QuotaGroup::new(Some("Gemini"));
        group.quotas.push(QuotaWindow::new(
            "5h",
            "5h",
            35.2,
            64.8,
            Some("2026-10-26T18:00:00Z".to_string()),
            Some("resets in 3h 30m".to_string()),
        ));
        group.quotas.push(QuotaWindow::new(
            "weekly",
            "Weekly",
            23.4,
            76.6,
            Some("2026-11-01T14:32:00Z".to_string()),
            Some("resets in 5d 23h".to_string()),
        ));
        acc.groups.push(group);
        report.accounts.push(acc);

        let json = serde_json::to_string(&report).expect("Failed to serialize AppUsageReport");
        let deserialized: AppUsageReport =
            serde_json::from_str(&json).expect("Failed to deserialize AppUsageReport");

        assert_eq!(report, deserialized);
        assert_eq!(deserialized.app_name, "Antigravity");
        assert_eq!(deserialized.accounts.len(), 1);
        assert_eq!(deserialized.accounts[0].groups.len(), 1);
        assert_eq!(deserialized.accounts[0].groups[0].quotas.len(), 2);
    }

    #[test]
    fn test_format_resets_in_seconds() {
        assert_eq!(format_resets_in_seconds(0), "resets soon");
        assert_eq!(format_resets_in_seconds(-10), "resets soon");
        assert_eq!(format_resets_in_seconds(45), "resets soon");
        assert_eq!(format_resets_in_seconds(120), "resets in 2m");
        assert_eq!(format_resets_in_seconds(3600), "resets in 1h");
        assert_eq!(format_resets_in_seconds(5400), "resets in 1h 30m");
        assert_eq!(format_resets_in_seconds(86400), "resets in 1d");
        assert_eq!(format_resets_in_seconds(97200), "resets in 1d 3h");
    }

    #[test]
    fn test_format_resets_in_rfc3339() {
        assert_eq!(format_resets_in_rfc3339(""), None);
        assert_eq!(format_resets_in_rfc3339("   "), None);

        let future = Utc::now() + chrono::Duration::hours(2) + chrono::Duration::minutes(15);
        let text = format_resets_in_rfc3339(&future.to_rfc3339()).unwrap();
        assert!(text.contains("resets in 2h"));

        let past = Utc::now() - chrono::Duration::minutes(10);
        let text_past = format_resets_in_rfc3339(&past.to_rfc3339()).unwrap();
        assert_eq!(text_past, "resets soon");
    }
}
