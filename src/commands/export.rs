use anyhow::Result;
use clap::Args;
use colored::Colorize;
use std::path::{Path, PathBuf};

#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    /// Output archive file path (default: ./ags-backup-<YYYYMMDD_HHMMSS>.agx)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Archive encryption key (will prompt interactively if not provided)
    #[arg(short, long, visible_alias = "key")]
    pub password: Option<String>,

    /// Target agent to export (e.g. codex, agy; defaults to all)
    #[arg(long)]
    pub agent: Option<String>,
}

pub fn execute(base_dir: &Path, args: ExportArgs) -> Result<()> {
    let password = crate::commands::prompt_password(args.password.as_deref(), true)?;

    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let output_path = args
        .output
        .unwrap_or_else(|| PathBuf::from(format!("./ags-backup-{}.agx", ts)));

    let target_agents: Vec<String> = match args.agent {
        Some(a) => {
            if let Some(agent) = crate::agent::get_agent(&a) {
                vec![agent.id().to_string()]
            } else {
                vec![a.to_lowercase()]
            }
        }
        None => crate::agent::all_agents()
            .iter()
            .map(|a| a.id().to_string())
            .collect(),
    };

    let opts = crate::archive::ExportOptions {
        base_dir: base_dir.to_path_buf(),
        output_path,
        password,
        target_agents,
    };

    let report = crate::archive::run_export(&opts)?;

    println!(
        "{} {}",
        "✔".green().bold(),
        "Account backup archive exported successfully!"
            .green()
            .bold()
    );
    println!(
        "  Archive file: {}",
        report.output_path.display().to_string().cyan().bold()
    );
    println!("  File size: {:.1} KB", (report.file_size as f64) / 1024.0);
    println!("  Accounts by agent:");
    for agent in &report.exported_agents {
        let count = report.agent_counts.get(agent).copied().unwrap_or(0);
        println!(
            "    • {}: {}",
            agent.bold(),
            count.to_string().cyan().bold()
        );
    }
    println!(
        "  Total accounts: {}",
        report.total_accounts.to_string().bold()
    );
    println!();
    println!(
        "  {}",
        "[Security Notice] Archive is encrypted with a standalone key. Use 'ags import' to restore on any machine."
            .yellow()
    );

    Ok(())
}

pub use execute as handle_export;
