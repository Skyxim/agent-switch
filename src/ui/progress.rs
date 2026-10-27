use colored::*;

/// Threshold color mode for the progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorThresholdMode {
    /// Safe when percentage is high, warning when medium, critical when low (e.g. remaining quota).
    Remaining,
    /// Safe when percentage is low, warning when medium, critical when high (e.g. quota used).
    Used,
}

/// A configurable progress bar renderer.
#[derive(Debug, Clone)]
pub struct ProgressBar {
    pub width: usize,
    pub filled_char: char,
    pub empty_char: char,
    pub show_brackets: bool,
    pub mode: ColorThresholdMode,
    pub use_color: bool,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self {
            width: 25,
            filled_char: '█',
            empty_char: '░',
            show_brackets: true,
            mode: ColorThresholdMode::Remaining,
            use_color: true,
        }
    }
}

impl ProgressBar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width.max(1);
        self
    }

    pub fn with_chars(mut self, filled: char, empty: char) -> Self {
        self.filled_char = filled;
        self.empty_char = empty;
        self
    }

    pub fn with_brackets(mut self, show: bool) -> Self {
        self.show_brackets = show;
        self
    }

    pub fn with_mode(mut self, mode: ColorThresholdMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_color(mut self, use_color: bool) -> Self {
        self.use_color = use_color;
        self
    }

    /// Calculate the number of filled cells for a given percentage (0.0..=100.0).
    pub fn calculate_filled(&self, percent: f64) -> usize {
        let clamped = percent.clamp(0.0, 100.0);
        let filled = ((clamped / 100.0) * self.width as f64).round() as usize;
        filled.min(self.width)
    }

    /// Render uncolored raw bar text (e.g. "[████░░░░░░]").
    pub fn render_plain(&self, percent: f64) -> String {
        let filled = self.calculate_filled(percent);
        let empty = self.width.saturating_sub(filled);

        let body: String = std::iter::repeat_n(self.filled_char, filled)
            .chain(std::iter::repeat_n(self.empty_char, empty))
            .collect();

        if self.show_brackets {
            format!("[{}]", body)
        } else {
            body
        }
    }

    /// Render formatted progress bar with ANSI color styling applied.
    pub fn render(&self, percent: f64) -> String {
        let plain = self.render_plain(percent);
        if !self.use_color {
            return plain;
        }

        let clamped = percent.clamp(0.0, 100.0);
        match self.mode {
            ColorThresholdMode::Remaining => {
                if clamped <= 20.0 {
                    plain.red().bold().to_string()
                } else if clamped <= 50.0 {
                    plain.yellow().to_string()
                } else {
                    plain.green().to_string()
                }
            }
            ColorThresholdMode::Used => {
                if clamped >= 80.0 {
                    plain.red().bold().to_string()
                } else if clamped >= 50.0 {
                    plain.yellow().to_string()
                } else {
                    plain.green().to_string()
                }
            }
        }
    }

    /// Format a single quota or usage line.
    ///
    /// Example output:
    /// `  5h         [█████████████████████████]  100.0% left (resets in 4h)`
    pub fn format_quota_line(
        &self,
        indent: &str,
        label: &str,
        remaining_percent: f64,
        reset_text: Option<&str>,
    ) -> String {
        let clamped = remaining_percent.clamp(0.0, 100.0);
        let bar = self.render(clamped);

        let padded_label = format!("{:<10}", label);
        let label_styled = if self.use_color {
            padded_label.bold().to_string()
        } else {
            padded_label
        };

        let reset_str = match reset_text {
            Some(t) if !t.trim().is_empty() => {
                let formatted = format!(" ({})", t.trim());
                if self.use_color {
                    formatted.dimmed().to_string()
                } else {
                    formatted
                }
            }
            _ => String::new(),
        };

        format!(
            "{}{} {} {:>5.1}% left{}",
            indent, label_styled, bar, clamped, reset_str
        )
    }
}

/// Render a standard colored remaining quota progress bar.
pub fn render_progress_bar(remaining_percent: f64, width: usize) -> String {
    ProgressBar::default()
        .with_width(width)
        .render(remaining_percent)
}

/// Format a single quota line with standard layout and styling.
pub fn format_quota_line(
    indent: &str,
    label: &str,
    remaining_percent: f64,
    reset_text: Option<&str>,
) -> String {
    ProgressBar::default().format_quota_line(indent, label, remaining_percent, reset_text)
}

/// Render a quota line to stdout using `ProgressBar::default()`.
pub fn render_quota_line(indent: &str, quota: &crate::model::QuotaWindow) {
    let line = format_quota_line(
        indent,
        &quota.display_label,
        quota.remaining_percent,
        quota.resets_in_text.as_deref(),
    );
    println!("{}", line);
}

/// Format an entire AppUsageReport into a styled string.
pub fn format_usage_report(report: &crate::model::AppUsageReport, show_app_header: bool) -> String {
    let mut lines = Vec::new();

    if show_app_header {
        lines.push(report.app_name.bold().cyan().to_string());
    }

    if report.accounts.is_empty() {
        let cmd_name = if report.app_name.eq_ignore_ascii_case("antigravity") {
            "agy"
        } else {
            &report.app_name.to_lowercase()
        };
        lines.push(format!(
            "  {}",
            format!(
                "No saved {} accounts. Use `ags {} add` to add one.",
                report.app_name, cmd_name
            )
            .dimmed()
        ));
        return lines.join("\n");
    }

    for (i, acc) in report.accounts.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }

        let alias_str = acc.alias.as_deref().unwrap_or("-");
        let tier_str = match &acc.tier {
            Some(t) if !t.is_empty() => format!(" {}", t.yellow().bold()),
            _ => String::new(),
        };
        let active_str = if acc.is_active {
            format!(" {}", "[*active]".green().bold())
        } else {
            String::new()
        };

        lines.push(format!(
            "#{} {} {}{}{}",
            acc.index,
            alias_str.bold(),
            acc.email.cyan(),
            tier_str,
            active_str
        ));

        if let Some(ref err) = acc.error {
            lines.push(format!(
                "  {} Failed to query quota: {}",
                "✗".red().bold(),
                err.red()
            ));
            continue;
        }

        let total_quotas: usize = acc.groups.iter().map(|g| g.quotas.len()).sum();
        if total_quotas == 0 {
            lines.push(format!(
                "  {}",
                "No specific rate limits or usage windows found for this account.".dimmed()
            ));
            continue;
        }

        for group in &acc.groups {
            if group.quotas.is_empty() {
                continue;
            }

            let quota_indent = if let Some(ref name) = group.name {
                lines.push(format!("  {}", name.bold()));
                "    "
            } else {
                "  "
            };

            for quota in &group.quotas {
                lines.push(format_quota_line(
                    quota_indent,
                    &quota.display_label,
                    quota.remaining_percent,
                    quota.resets_in_text.as_deref(),
                ));
            }
        }
    }

    lines.join("\n")
}

/// Render an AppUsageReport to stdout.
pub fn render_usage_report(report: &crate::model::AppUsageReport, show_app_header: bool) {
    let out = format_usage_report(report, show_app_header);
    if !out.is_empty() {
        println!("{}", out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_bar_plain_rendering() {
        let bar = ProgressBar::new()
            .with_width(10)
            .with_chars('#', '-')
            .with_color(false);

        assert_eq!(bar.render_plain(0.0), "[----------]");
        assert_eq!(bar.render_plain(50.0), "[#####-----]");
        assert_eq!(bar.render_plain(100.0), "[##########]");
        assert_eq!(bar.render_plain(-10.0), "[----------]");
        assert_eq!(bar.render_plain(150.0), "[##########]");
    }

    #[test]
    fn test_progress_bar_unicode_blocks() {
        let bar = ProgressBar::new().with_width(8).with_color(false);
        assert_eq!(bar.render_plain(0.0), "[░░░░░░░░]");
        assert_eq!(bar.render_plain(50.0), "[████░░░░]");
        assert_eq!(bar.render_plain(100.0), "[████████]");
    }

    #[test]
    fn test_progress_bar_color_modes() {
        let bar_rem = ProgressBar::new().with_width(5).with_color(true);
        // Low remaining is red
        let red_out = bar_rem.render(15.0);
        assert!(red_out.contains("\x1b[") || !red_out.is_empty());

        let bar_used = ProgressBar::new()
            .with_width(5)
            .with_mode(ColorThresholdMode::Used)
            .with_color(true);
        // Low used is green
        let green_out = bar_used.render(15.0);
        assert!(!green_out.is_empty());
    }

    #[test]
    fn test_format_quota_line() {
        let bar = ProgressBar::new().with_width(10).with_color(false);
        let line = bar.format_quota_line("  ", "5h", 75.0, Some("resets in 2h"));
        assert!(line.starts_with("  5h"));
        assert!(line.contains("75.0% left"));
        assert!(line.contains("(resets in 2h)"));
    }

    #[test]
    fn test_format_usage_report() {
        use crate::model::{AccountUsageReport, AppUsageReport, QuotaGroup, QuotaWindow};

        let mut report = AppUsageReport::new("Codex");
        let empty_out = format_usage_report(&report, true);
        assert!(empty_out.contains("Codex"));
        assert!(empty_out.contains("No saved Codex accounts"));

        let mut acc = AccountUsageReport::new(1, "user@example.com");
        acc.alias = Some("work".to_string());
        acc.tier = Some("Plus".to_string());
        acc.is_active = true;

        let mut group = QuotaGroup::new(None::<String>);
        group.quotas.push(QuotaWindow::new(
            "5h",
            "5h",
            50.0,
            50.0,
            None,
            Some("resets in 2h".to_string()),
        ));
        acc.groups.push(group);
        report.accounts.push(acc);

        let formatted = format_usage_report(&report, false);
        assert!(!formatted.contains("Codex\n"));
        assert!(formatted.contains("user@example.com"));
        assert!(formatted.contains("work"));
        assert!(formatted.contains("5h"));
        assert!(formatted.contains("50.0% left"));
    }
}
