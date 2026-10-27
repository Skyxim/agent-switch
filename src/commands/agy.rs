use crate::agent::AgyAgent;
use crate::commands::common::{
    execute_add, execute_current, execute_list, execute_rm, execute_switch, execute_usage,
};
use crate::commands::AgyCommands;
use crate::crypto::CryptoProvider;
use crate::vault::Store;
use anyhow::Result;

pub async fn handle_agy(
    store: &Store,
    crypto: &dyn CryptoProvider,
    cmd: AgyCommands,
) -> Result<()> {
    let agent = AgyAgent::new();
    match cmd {
        AgyCommands::Add { name } => execute_add(&agent, store, crypto, name.as_deref()),
        AgyCommands::List => execute_list(&agent, store, false),
        AgyCommands::Switch { name } => execute_switch(&agent, store, crypto, &name),
        AgyCommands::Current => execute_current(&agent, store),
        AgyCommands::Rm { name } => execute_rm(&agent, store, &name),
        AgyCommands::Usage => execute_usage(&agent, store, crypto, false).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_handle_agy_dispatch() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;

        let res = handle_agy(&store, &crypto, AgyCommands::List).await;
        assert!(res.is_ok());
    }
}
