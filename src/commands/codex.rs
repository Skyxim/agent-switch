use crate::agent::CodexAgent;
use crate::commands::common::{
    execute_add, execute_current, execute_list, execute_rm, execute_switch, execute_usage,
};
use crate::commands::CodexCommands;
use crate::crypto::CryptoProvider;
use crate::vault::Store;
use anyhow::Result;

pub async fn handle_codex(
    store: &Store,
    crypto: &dyn CryptoProvider,
    cmd: CodexCommands,
) -> Result<()> {
    let agent = CodexAgent::new();
    match cmd {
        CodexCommands::Add { name } => execute_add(&agent, store, crypto, name.as_deref()),
        CodexCommands::List => execute_list(&agent, store, false),
        CodexCommands::Switch { name } => execute_switch(&agent, store, crypto, &name),
        CodexCommands::Current => execute_current(&agent, store),
        CodexCommands::Rm { name } => execute_rm(&agent, store, &name),
        CodexCommands::Usage => execute_usage(&agent, store, crypto, false).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_handle_codex_dispatch() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;

        let res = handle_codex(&store, &crypto, CodexCommands::List).await;
        assert!(res.is_ok());
    }
}
