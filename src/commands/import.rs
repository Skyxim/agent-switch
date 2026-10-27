use anyhow::{bail, Result};
use clap::Args;
use colored::Colorize;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    /// Path to the encrypted archive file (.agx)
    pub file: PathBuf,

    /// Archive decryption key (will prompt interactively if not provided)
    #[arg(short, long, visible_alias = "key")]
    pub password: Option<String>,

    /// Limit import to specific agent (e.g. codex, agy; defaults to all)
    #[arg(long)]
    pub agent: Option<String>,

    /// Overwrite local accounts with matching ID or email (default: true)
    #[arg(long, default_value_t = true)]
    pub overwrite: bool,

    /// Automatically backup and clean incompatible local data without confirmation
    #[arg(long, default_value_t = false)]
    pub clean: bool,
}

pub fn execute(base_dir: &Path, args: ImportArgs) -> Result<()> {
    if !args.file.exists() {
        bail!("Backup file does not exist: {}", args.file.display());
    }

    let password = crate::commands::prompt_password(args.password.as_deref(), false)?;

    let global_config = crate::config::load_global_config(base_dir).unwrap_or_default();
    let local_crypto = crate::crypto::CryptoManager::create_provider(
        global_config.crypto_provider.as_deref(),
        None,
    );

    let target_agents: Vec<String> = match &args.agent {
        Some(a) => {
            if let Some(agent) = crate::agent::get_agent(a) {
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

    let mut clean_incompatible = args.clean;
    if !clean_incompatible {
        for agent in &target_agents {
            let agent_dir = base_dir.join(agent);
            if let crate::archive::CompatibilityStatus::Incompatible(reason) =
                crate::archive::check_agent_compatibility(&agent_dir, &*local_crypto)
            {
                println!(
                    "{} Existing incompatible [{}] data detected in local storage ({}).",
                    "⚠".yellow().bold(),
                    agent.bold(),
                    reason.dimmed()
                );
                print!(
                    "? Backup and clean incompatible local data to proceed with import? (y/N): "
                );
                io::stdout().flush()?;
                let mut answer = String::new();
                io::stdin().read_line(&mut answer)?;
                let trimmed = answer.trim().to_lowercase();
                if trimmed == "y" || trimmed == "yes" {
                    clean_incompatible = true;
                } else {
                    bail!("Operation cancelled. Incompatible local data left unchanged. Pass --clean or enter 'y' to clean.");
                }
            }
        }
    }

    let opts = crate::archive::ImportOptions {
        base_dir: base_dir.to_path_buf(),
        input_path: args.file.clone(),
        password,
        target_agents: args.agent.map(|a| vec![a.to_lowercase()]),
        clean_incompatible,
        overwrite: args.overwrite,
    };

    let report = crate::archive::run_import(&opts)?;

    println!(
        "{} {}",
        "✔".green().bold(),
        "Account backup archive imported successfully!"
            .green()
            .bold()
    );
    println!(
        "  Source archive: {}",
        args.file.display().to_string().cyan()
    );
    for (agent, (new_count, merged_count)) in &report.agent_details {
        println!(
            "  • {}: added {}, merged {}",
            agent.bold(),
            new_count.to_string().cyan().bold(),
            merged_count.to_string().yellow().bold()
        );
    }
    println!(
        "  Summary: Processed {} accounts (added {}, merged {})",
        report.total_accounts.to_string().bold(),
        report.total_new.to_string().cyan(),
        report.total_merged.to_string().yellow()
    );

    if !report.backup_dirs.is_empty() {
        for b in &report.backup_dirs {
            println!(
                "  Snapshot backup saved to: {}",
                b.display().to_string().dimmed()
            );
        }
    }

    if !report.cleaned_agents.is_empty() {
        println!(
            "  {} Cleaned incompatible data for: {}",
            "✔".green(),
            report.cleaned_agents.join(", ").yellow()
        );
    }

    println!(
        "  {}",
        "[Storage] Credentials re-encrypted and saved using current storage settings.".dimmed()
    );

    Ok(())
}

pub use execute as handle_import;
