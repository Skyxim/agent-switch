pub mod env;
pub mod plain;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreEngine {
    Env,
    Plain,
    System,
}

impl StoreEngine {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "env" => Ok(StoreEngine::Env),
            "plain" | "none" => Ok(StoreEngine::Plain),
            "system" | "keyring" => Ok(StoreEngine::System),
            other => bail!(
                "Unknown store engine '{}'. Supported engines: env, plain, system (planned)",
                other
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            StoreEngine::Env => "env",
            StoreEngine::Plain => "plain",
            StoreEngine::System => "system",
        }
    }
}

pub trait CryptoProvider: Send + Sync {
    /// Unique identifier for this provider (e.g., "env", "plain")
    fn name(&self) -> &'static str;

    /// Human-readable description including key status
    fn description(&self) -> String;

    /// Encrypt plaintext bytes into ciphertext with framing
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>>;

    /// Decrypt framed ciphertext bytes back into plaintext
    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
}

pub struct CryptoManager;

impl CryptoManager {
    /// Select provider by name or fallback to "env", using environment key
    pub fn get_provider(name: Option<&str>) -> Box<dyn CryptoProvider> {
        Self::create_provider(name, None)
    }

    /// Select provider by name with explicit key override (for migration or tests)
    pub fn create_provider(
        name: Option<&str>,
        explicit_key: Option<&str>,
    ) -> Box<dyn CryptoProvider> {
        let env_provider = std::env::var("AGS_CRYPTO_PROVIDER").ok();
        let selected = name.or(env_provider.as_deref()).unwrap_or("env");

        match StoreEngine::parse(selected) {
            Ok(StoreEngine::Plain) => Box::new(plain::PlaintextCryptoProvider),
            Ok(StoreEngine::System) => {
                // system keyring engine planned for future release
                eprintln!("Warning: 'system' store engine is planned for a future release; falling back to 'env'");
                if let Some(key) = explicit_key {
                    Box::new(env::EnvCryptoProvider::with_explicit_key(Some(key)))
                } else {
                    Box::new(env::EnvCryptoProvider::new())
                }
            }
            _ => {
                if let Some(key) = explicit_key {
                    Box::new(env::EnvCryptoProvider::with_explicit_key(Some(key)))
                } else {
                    Box::new(env::EnvCryptoProvider::new())
                }
            }
        }
    }
}
