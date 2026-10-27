use crate::model::{AppUsageReport, QuotaWindow};

/// Render an AppUsageReport to the console.
/// If `show_app_header` is true, prints the `AppName` header at the top (used for global `ags usage`).
/// If `show_app_header` is false, omits the top `AppName` header (used for `ags codex usage` / `ags agy usage`).
pub fn render_usage_report(report: &AppUsageReport, show_app_header: bool) {
    crate::ui::progress::render_usage_report(report, show_app_header);
}

/// Render a single quota window line with progress bar, remaining percentage, and reset description.
pub fn render_quota_line(indent: &str, quota: &QuotaWindow) {
    crate::ui::progress::render_quota_line(indent, quota);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountUsageReport, QuotaGroup};

    #[test]
    fn test_render_usage_report_no_panic() {
        let mut report = AppUsageReport::new("Antigravity");
        let mut acc = AccountUsageReport::new(1, "test@example.com");
        acc.alias = Some("work".to_string());
        acc.tier = Some("Google AI Pro".to_string());
        acc.is_active = true;

        let mut g1 = QuotaGroup::new(Some("Gemini"));
        g1.quotas.push(QuotaWindow::new(
            "5h",
            "5h",
            35.2,
            64.8,
            Some("2026-10-26T18:00:00Z".to_string()),
            Some("resets in 3h 30m".to_string()),
        ));
        g1.quotas.push(QuotaWindow::new(
            "weekly",
            "Weekly",
            23.4,
            76.6,
            Some("2026-11-01T14:32:00Z".to_string()),
            Some("resets in 5d 23h".to_string()),
        ));
        acc.groups.push(g1);

        let mut g2 = QuotaGroup::new(Some("Claude & GPT"));
        g2.quotas.push(QuotaWindow::new(
            "5h",
            "5h",
            1.4,
            98.6,
            Some("2026-10-26T15:32:00Z".to_string()),
            Some("resets in 18m".to_string()),
        ));
        g2.quotas.push(QuotaWindow::new(
            "weekly",
            "Weekly",
            0.0,
            100.0,
            Some("2026-11-01T14:32:00Z".to_string()),
            Some("resets in 5d 23h".to_string()),
        ));
        acc.groups.push(g2);

        let mut acc2 = AccountUsageReport::new(2, "failed@example.com");
        acc2.error = Some("401 Unauthorized: token expired".to_string());

        report.accounts.push(acc);
        report.accounts.push(acc2);

        // Render both with header and without header
        render_usage_report(&report, true);
        render_usage_report(&report, false);
    }
}
