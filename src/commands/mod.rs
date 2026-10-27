pub mod agy;
pub mod codex;
pub mod common;
pub mod export;
pub mod import;
pub mod info;
pub mod list;
pub mod update;
pub mod usage;

pub use agy::handle_agy;
pub use codex::handle_codex;
pub use export::{handle_export, ExportArgs};
pub use import::{handle_import, ImportArgs};
pub use info::handle_info;
pub use list::handle_list;
pub use update::{detect_install_source, handle_update, InstallSource, UpdateArgs};
pub use usage::handle_usage;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::io::{self, IsTerminal, Write};

pub fn prompt_password(cli_password: Option<&str>, is_export: bool) -> Result<String> {
    if let Some(p) = cli_password {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if cfg!(test) || !io::stdin().is_terminal() {
        bail!("No encryption/decryption key provided. Pass via -k/--key or enter interactively.");
    }

    let stdin = io::stdin();
    if is_export {
        print!("Enter archive encryption key: ");
        io::stdout().flush()?;
        let mut pass1 = String::new();
        stdin.read_line(&mut pass1)?;
        let pass1 = pass1.trim_end_matches(&['\r', '\n'][..]).to_string();
        if pass1.is_empty() {
            bail!("Encryption key cannot be empty");
        }

        print!("Confirm archive encryption key: ");
        io::stdout().flush()?;
        let mut pass2 = String::new();
        stdin.read_line(&mut pass2)?;
        let pass2 = pass2.trim_end_matches(&['\r', '\n'][..]).to_string();
        if pass1 != pass2 {
            bail!("Keys do not match");
        }
        Ok(pass1)
    } else {
        print!("Enter archive decryption key: ");
        io::stdout().flush()?;
        let mut pass = String::new();
        stdin.read_line(&mut pass)?;
        let pass = pass.trim_end_matches(&['\r', '\n'][..]).to_string();
        if pass.is_empty() {
            bail!("Decryption key cannot be empty");
        }
        Ok(pass)
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "ags",
    about = "Fast, encrypted CLI switcher for AI agent credentials (Codex & Antigravity)",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage Codex (ChatGPT) accounts
    Codex {
        #[command(subcommand)]
        subcommand: CodexCommands,
    },

    /// Manage Antigravity (agy) accounts
    Agy {
        #[command(subcommand)]
        subcommand: AgyCommands,
    },

    /// List saved accounts across all agents or specified agent
    #[command(alias = "ls")]
    List {
        /// Target agent (codex, agy; if omitted, lists all)
        agent: Option<String>,
    },

    /// View quota and usage across all accounts and agents
    Usage {
        /// Target agent (codex, agy; if omitted, checks both)
        agent: Option<String>,
    },

    /// Print system status, active store engine, and account summaries
    Info,

    /// Export portable encrypted backup archive (standalone key required)
    Export(ExportArgs),

    /// Import credentials from portable encrypted backup archive
    Import(ImportArgs),

    /// Check and self-update to latest release from GitHub Releases
    Update(UpdateArgs),
}

#[derive(Subcommand, Debug)]
pub enum CodexCommands {
    /// Save the current Codex credentials (~/.codex/auth.json)
    Add {
        /// Optional alias for this account
        name: Option<String>,
    },

    /// List all saved Codex accounts
    #[command(alias = "ls")]
    List,

    /// Switch to a saved Codex account (supports alias, email, index, or '-' for previous)
    #[command(alias = "sw")]
    Switch {
        /// Account alias, email, 1-based index, or '-'
        name: String,
    },

    /// Display details of current active Codex account
    Current,

    /// Delete a saved Codex account
    #[command(alias = "remove")]
    Rm {
        /// Account alias, email, or index
        name: String,
    },

    /// Query quota and usage from ChatGPT backend
    Usage,
}

#[derive(Subcommand, Debug)]
pub enum AgyCommands {
    /// Save current Antigravity credentials
    Add {
        /// Optional alias for this account
        name: Option<String>,
    },

    /// List all saved Antigravity accounts
    #[command(alias = "ls")]
    List,

    /// Switch to a saved Antigravity account (supports alias, email, index, or '-' for previous)
    #[command(alias = "sw")]
    Switch {
        /// Account alias, email, 1-based index, or '-'
        name: String,
    },

    /// Display details of current active Antigravity account
    Current,

    /// Delete a saved Antigravity account
    #[command(alias = "remove")]
    Rm {
        /// Account alias, email, or index
        name: String,
    },

    /// Query quota and usage from Google Cloud Code backend
    Usage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_password_cli_priority() {
        let res = prompt_password(Some("cli-password-123"), false).unwrap();
        assert_eq!(res, "cli-password-123");
    }

    #[test]
    fn test_prompt_password_missing_in_non_terminal_fails() {
        // Missing CLI argument in non-TTY environment must return error
        let res = prompt_password(None, false);
        assert!(res.is_err());
    }
}
