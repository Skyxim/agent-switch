use agent_switch::commands::{
    handle_agy, handle_codex, handle_export, handle_import, handle_info, handle_list,
    handle_update, handle_usage, Cli, Commands,
};
use agent_switch::config::{get_base_dir, load_global_config};
use agent_switch::crypto::CryptoManager;
use agent_switch::vault::Store;
use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let base_dir = get_base_dir();
    let global_config = load_global_config(&base_dir)?;
    let crypto = CryptoManager::get_provider(global_config.crypto_provider.as_deref());

    match cli.command {
        Commands::Codex { subcommand } => {
            let store = Store::new(base_dir.join("codex"));
            handle_codex(&store, &*crypto, subcommand).await?;
        }
        Commands::Agy { subcommand } => {
            let store = Store::new(base_dir.join("agy"));
            handle_agy(&store, &*crypto, subcommand).await?;
        }
        Commands::List { agent } => {
            handle_list(&base_dir, agent.as_deref())?;
        }
        Commands::Usage { agent } => {
            handle_usage(&base_dir, &*crypto, agent.as_deref()).await?;
        }
        Commands::Info => {
            handle_info(&base_dir, &*crypto)?;
        }
        Commands::Export(args) => {
            handle_export(&base_dir, args)?;
        }
        Commands::Import(args) => {
            handle_import(&base_dir, args)?;
        }
        Commands::Update(args) => {
            handle_update(args).await?;
        }
    }

    Ok(())
}
