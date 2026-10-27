use colored::*;

/// Text alignment within a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlignment {
    Left,
    Right,
    Center,
}

/// Strip ANSI escape sequences from a string to compute visible terminal width.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(&'[') = chars.peek() {
                chars.next(); // consume '['
                for next_c in chars.by_ref() {
                    if next_c.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Calculate the visible character width of a string (ignoring ANSI color codes).
pub fn visible_width(s: &str) -> usize {
    strip_ansi(s).chars().count()
}

/// Pad a string (which may contain ANSI escape sequences) to a given visible width.
pub fn pad_cell(s: &str, target_width: usize, alignment: TableAlignment) -> String {
    let vis = visible_width(s);
    if vis >= target_width {
        return s.to_string();
    }

    let diff = target_width - vis;
    match alignment {
        TableAlignment::Left => format!("{}{}", s, " ".repeat(diff)),
        TableAlignment::Right => format!("{}{}", " ".repeat(diff), s),
        TableAlignment::Center => {
            let left = diff / 2;
            let right = diff - left;
            format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
        }
    }
}

/// Table column definition.
#[derive(Debug, Clone)]
pub struct TableColumn {
    pub header: String,
    pub alignment: TableAlignment,
    pub min_width: usize,
}

impl TableColumn {
    pub fn new(header: impl Into<String>) -> Self {
        Self {
            header: header.into(),
            alignment: TableAlignment::Left,
            min_width: 0,
        }
    }

    pub fn with_alignment(mut self, alignment: TableAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn with_min_width(mut self, min_width: usize) -> Self {
        self.min_width = min_width;
        self
    }
}

/// A row of cells in a table.
#[derive(Debug, Clone, Default)]
pub struct TableRow {
    pub cells: Vec<String>,
}

impl TableRow {
    pub fn new(cells: Vec<String>) -> Self {
        Self { cells }
    }
}

/// Generic, styled terminal table renderer.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow>,
    pub column_spacing: usize,
    pub show_divider: bool,
}

impl Table {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            column_spacing: 2,
            show_divider: true,
        }
    }

    pub fn add_column(&mut self, column: TableColumn) {
        self.columns.push(column);
    }

    pub fn add_row(&mut self, row: TableRow) {
        self.rows.push(row);
    }

    /// Calculate effective column widths based on headers, minimums, and content.
    pub fn compute_widths(&self) -> Vec<usize> {
        let mut widths = Vec::with_capacity(self.columns.len());
        for (col_idx, col) in self.columns.iter().enumerate() {
            let mut max_w = visible_width(&col.header).max(col.min_width);
            for row in &self.rows {
                if let Some(cell) = row.cells.get(col_idx) {
                    max_w = max_w.max(visible_width(cell));
                }
            }
            widths.push(max_w);
        }
        widths
    }

    /// Render the table into a multi-line formatted string.
    pub fn render(&self) -> String {
        if self.columns.is_empty() {
            return String::new();
        }

        let widths = self.compute_widths();
        let spacing = " ".repeat(self.column_spacing);
        let mut lines = Vec::new();

        // 1. Header row
        let header_cells: Vec<String> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| pad_cell(&col.header, widths[i], col.alignment))
            .collect();
        lines.push(header_cells.join(&spacing));

        // 2. Divider line
        if self.show_divider {
            let total_width: usize = widths.iter().sum::<usize>()
                + (self.columns.len().saturating_sub(1) * self.column_spacing);
            lines.push("-".repeat(total_width).dimmed().to_string());
        }

        // 3. Body rows
        for row in &self.rows {
            let row_cells: Vec<String> = self
                .columns
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let cell_content = row.cells.get(i).map(|s| s.as_str()).unwrap_or("");
                    pad_cell(cell_content, widths[i], col.alignment)
                })
                .collect();
            lines.push(row_cells.join(&spacing));
        }

        lines.join("\n")
    }

    pub fn print(&self) {
        let rendered = self.render();
        if !rendered.is_empty() {
            println!("{}", rendered);
        }
    }
}

/// Representation of an account item for table display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountTableItem {
    pub index: usize,
    pub is_active: bool,
    pub alias: Option<String>,
    pub email: String,
    pub plan: Option<String>,
    pub added_at: Option<String>,
    pub store_status: Option<String>,
}

/// Rendering options for an account list table.
#[derive(Debug, Clone, Default)]
pub struct AccountTableOptions {
    pub app_display_name: Option<String>,
    pub show_app_header: bool,
    pub show_store_status: bool,
    pub empty_message: Option<String>,
}

/// Render a list of account display items to a formatted string.
pub fn render_account_table(items: &[AccountTableItem], options: &AccountTableOptions) -> String {
    let mut out = Vec::new();

    if options.show_app_header {
        if let Some(ref name) = options.app_display_name {
            out.push(name.bold().cyan().to_string());
        }
    }

    if items.is_empty() {
        let app_name = options.app_display_name.as_deref().unwrap_or("account");
        let default_empty = format!(
            "No saved {} accounts. Use `ags {} add <name>` to add one.",
            app_name,
            app_name.to_lowercase()
        );
        let msg = options.empty_message.as_deref().unwrap_or(&default_empty);
        out.push(format!("  {}", msg.dimmed()));
        return out.join("\n");
    }

    // Build table columns: #, (active marker), Alias, Email, Plan, Added, [Store]
    let mut table = Table::new();
    table.column_spacing = 2;
    table.add_column(
        TableColumn::new("#")
            .with_alignment(TableAlignment::Right)
            .with_min_width(3),
    );
    table.add_column(
        TableColumn::new(" ")
            .with_alignment(TableAlignment::Center)
            .with_min_width(1),
    );
    table.add_column(
        TableColumn::new("Alias")
            .with_alignment(TableAlignment::Left)
            .with_min_width(15),
    );
    table.add_column(
        TableColumn::new("Email")
            .with_alignment(TableAlignment::Left)
            .with_min_width(30),
    );
    table.add_column(
        TableColumn::new("Plan")
            .with_alignment(TableAlignment::Left)
            .with_min_width(18),
    );
    table.add_column(
        TableColumn::new("Added")
            .with_alignment(TableAlignment::Left)
            .with_min_width(10),
    );

    if options.show_store_status {
        table.add_column(
            TableColumn::new("Store")
                .with_alignment(TableAlignment::Left)
                .with_min_width(12),
        );
    }

    for item in items {
        let marker = if item.is_active {
            "*".green().bold().to_string()
        } else {
            " ".to_string()
        };

        let alias_str = item.alias.as_deref().unwrap_or("-");
        let plan_str = item.plan.as_deref().unwrap_or("-");
        let added_str = item.added_at.as_deref().unwrap_or("-");

        let (alias_styled, email_styled) = if item.is_active {
            (
                alias_str.green().bold().to_string(),
                item.email.green().to_string(),
            )
        } else {
            (alias_str.to_string(), item.email.clone())
        };

        let mut cells = vec![
            item.index.to_string(),
            marker,
            alias_styled,
            email_styled,
            plan_str.to_string(),
            added_str.dimmed().to_string(),
        ];

        if options.show_store_status {
            let status = item.store_status.as_deref().unwrap_or("-");
            cells.push(status.to_string());
        }

        table.add_row(TableRow::new(cells));
    }

    out.push(table.render());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_and_visible_width() {
        let plain = "hello";
        assert_eq!(strip_ansi(plain), "hello");
        assert_eq!(visible_width(plain), 5);

        let colored_text = "world".green().bold().to_string();
        assert_eq!(strip_ansi(&colored_text), "world");
        assert_eq!(visible_width(&colored_text), 5);
    }

    #[test]
    fn test_pad_cell_alignments() {
        assert_eq!(pad_cell("abc", 5, TableAlignment::Left), "abc  ");
        assert_eq!(pad_cell("abc", 5, TableAlignment::Right), "  abc");
        assert_eq!(pad_cell("abc", 5, TableAlignment::Center), " abc ");
    }

    #[test]
    fn test_generic_table_render() {
        let mut table = Table::new();
        table.add_column(TableColumn::new("Name").with_min_width(6));
        table.add_column(
            TableColumn::new("Score")
                .with_alignment(TableAlignment::Right)
                .with_min_width(5),
        );
        table.add_row(TableRow::new(vec!["Alice".to_string(), "100".to_string()]));
        table.add_row(TableRow::new(vec!["Bob".to_string(), "95".to_string()]));

        let output = table.render();
        assert!(output.contains("Name"));
        assert!(output.contains("Score"));
        assert!(output.contains("Alice"));
        assert!(output.contains("100"));
    }

    #[test]
    fn test_account_table_empty_state() {
        let options = AccountTableOptions {
            app_display_name: Some("Codex".to_string()),
            show_app_header: true,
            ..Default::default()
        };
        let out = render_account_table(&[], &options);
        assert!(out.contains("Codex"));
        assert!(out.contains("No saved Codex accounts"));
    }

    #[test]
    fn test_account_table_with_items() {
        let items = vec![
            AccountTableItem {
                index: 1,
                is_active: true,
                alias: Some("work".to_string()),
                email: "work@example.com".to_string(),
                plan: Some("Pro".to_string()),
                added_at: Some("2026-10-26".to_string()),
                store_status: Some("Encrypted".to_string()),
            },
            AccountTableItem {
                index: 2,
                is_active: false,
                alias: None,
                email: "other@example.com".to_string(),
                plan: None,
                added_at: Some("2026-10-20".to_string()),
                store_status: Some("Encrypted".to_string()),
            },
        ];

        let options = AccountTableOptions {
            app_display_name: Some("Antigravity".to_string()),
            show_app_header: true,
            show_store_status: true,
            ..Default::default()
        };

        let rendered = render_account_table(&items, &options);
        assert!(rendered.contains("Antigravity"));
        assert!(rendered.contains("work@example.com"));
        assert!(rendered.contains("other@example.com"));
        assert!(rendered.contains("Pro"));
        assert!(rendered.contains("Encrypted"));
    }
}
