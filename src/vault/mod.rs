pub mod registry;

use crate::crypto::CryptoProvider;
use crate::util::atomic_write;
use anyhow::{bail, Context, Result};
use chrono::Utc;
pub use registry::{Account, Registry, REGISTRY_VERSION};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// CredentialStore alias for Store, representing the encrypted snapshot storage engine.
pub type CredentialStore = Store;

#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn registry_path(&self) -> PathBuf {
        self.dir.join("registry.json")
    }

    pub fn accounts_dir(&self) -> PathBuf {
        self.dir.join("accounts")
    }

    pub fn snapshot_path(&self, key: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let hash = hasher.finalize();
        let prefix = hex::encode(&hash[..12]);
        self.dir.join("accounts").join(format!("{}.json", prefix))
    }

    pub fn has_account(&self, key: &str) -> bool {
        self.snapshot_path(key).exists()
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.dir.join("backups")
    }

    pub fn load_registry(&self) -> Result<Registry> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(Registry::default());
        }
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read registry: {}", path.display()))?;
        let reg: Registry = serde_json::from_slice(&data)
            .with_context(|| format!("Failed to parse registry: {}", path.display()))?;
        Ok(reg)
    }

    pub fn save_registry(&self, reg: &Registry) -> Result<()> {
        let path = self.registry_path();
        let data = serde_json::to_vec_pretty(reg)?;
        atomic_write(&path, &data)
            .with_context(|| format!("Failed to save registry: {}", path.display()))?;
        Ok(())
    }

    pub fn read_account(&self, key: &str, crypto: &dyn CryptoProvider) -> Result<Vec<u8>> {
        let path = self.snapshot_path(key);
        if !path.exists() {
            bail!("Account snapshot file not found: {}", path.display());
        }
        let ciphertext = fs::read(&path)
            .with_context(|| format!("Failed to read account snapshot: {}", path.display()))?;
        crypto
            .decrypt(&ciphertext)
            .with_context(|| format!("Failed to decrypt account {}", key))
    }

    pub fn write_account(&self, key: &str, data: &[u8], crypto: &dyn CryptoProvider) -> Result<()> {
        let path = self.snapshot_path(key);
        let ciphertext = crypto
            .encrypt(data)
            .with_context(|| format!("Failed to encrypt account {}", key))?;
        atomic_write(&path, &ciphertext)
            .with_context(|| format!("Failed to write account snapshot: {}", path.display()))?;
        Ok(())
    }

    pub fn delete_account(&self, key: &str) -> Result<()> {
        let path = self.snapshot_path(key);
        if path.exists() {
            fs::remove_file(&path).with_context(|| {
                format!("Failed to delete account snapshot: {}", path.display())
            })?;
        }
        Ok(())
    }

    pub fn backup_file(&self, data: &[u8], label: &str) -> Result<PathBuf> {
        let bdir = self.backups_dir();
        fs::create_dir_all(&bdir)?;
        let ts = Utc::now().format("%Y%m%d_%H%M%S");
        let safe_label: String = label
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let backup_path = bdir.join(format!("{}_{}.json", ts, safe_label));
        atomic_write(&backup_path, data)?;
        Ok(backup_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::plain::PlaintextCryptoProvider;
    use tempfile::tempdir;

    #[test]
    fn test_store_snapshots() {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let crypto = PlaintextCryptoProvider;

        let key = "account:test";
        let content = b"{\"secret\":\"data\"}";

        assert!(!store.has_account(key));
        store.write_account(key, content, &crypto).unwrap();
        assert!(store.has_account(key));
        let loaded = store.read_account(key, &crypto).unwrap();
        assert_eq!(loaded, content);

        store.delete_account(key).unwrap();
        assert!(!store.has_account(key));
        assert!(store.read_account(key, &crypto).is_err());
    }
}
