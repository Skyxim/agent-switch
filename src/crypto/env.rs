use super::plain::MAGIC_PLAIN;
use super::CryptoProvider;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{bail, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::env;

pub const MAGIC_V1: &[u8] = b"AGSV1";
pub const ENV_VAR_NAME: &str = "AGS_KEY";
pub const LEGACY_ENV_VAR_NAME: &str = "AGS_ENCRYPT_KEY";
const SCHEME_KEYRING: u8 = 2;

pub struct EnvCryptoProvider {
    key: Option<[u8; 32]>,
}

impl EnvCryptoProvider {
    pub fn new() -> Self {
        let key = Self::resolve_key_from_env();
        Self { key }
    }

    pub fn with_explicit_key(raw: Option<&str>) -> Self {
        let key = raw.and_then(Self::parse_key_str);
        Self { key }
    }

    fn resolve_key_from_env() -> Option<[u8; 32]> {
        if let Ok(val) = env::var(ENV_VAR_NAME) {
            if let Some(key) = Self::parse_key_str(&val) {
                return Some(key);
            }
        }
        if let Ok(val) = env::var(LEGACY_ENV_VAR_NAME) {
            return Self::parse_key_str(&val);
        }
        None
    }

    fn parse_key_str(raw: &str) -> Option<[u8; 32]> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.len() == 64 && hex::decode(trimmed).is_ok() {
            let bytes = hex::decode(trimmed).ok()?;
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes[..32]);
            Some(key)
        } else {
            let mut hasher = Sha256::new();
            hasher.update(trimmed.as_bytes());
            let result = hasher.finalize();
            let mut key = [0u8; 32];
            key.copy_from_slice(&result[..32]);
            Some(key)
        }
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }
}

impl Default for EnvCryptoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CryptoProvider for EnvCryptoProvider {
    fn name(&self) -> &'static str {
        "env"
    }

    fn description(&self) -> String {
        if self.key.is_some() {
            format!("env [AES-256-GCM: {} is set]", ENV_VAR_NAME)
        } else {
            format!("env [Plaintext: {} not set]", ENV_VAR_NAME)
        }
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        match &self.key {
            Some(k) => {
                let cipher = Aes256Gcm::new_from_slice(k)
                    .map_err(|e| anyhow::anyhow!("Failed to initialize cipher: {}", e))?;

                let mut nonce_bytes = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ciphertext = cipher
                    .encrypt(
                        nonce,
                        Payload {
                            msg: plaintext,
                            aad: MAGIC_V1,
                        },
                    )
                    .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

                let mut framed = Vec::with_capacity(MAGIC_V1.len() + 1 + 12 + ciphertext.len());
                framed.extend_from_slice(MAGIC_V1);
                framed.push(SCHEME_KEYRING);
                framed.extend_from_slice(&nonce_bytes);
                framed.extend_from_slice(&ciphertext);

                Ok(framed)
            }
            None => {
                // When AGS_KEY is empty / not set, store as plaintext with AGSPLAIN framing
                let mut out = Vec::with_capacity(MAGIC_PLAIN.len() + plaintext.len());
                out.extend_from_slice(MAGIC_PLAIN);
                out.extend_from_slice(plaintext);
                Ok(out)
            }
        }
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.starts_with(MAGIC_PLAIN) {
            return Ok(ciphertext[MAGIC_PLAIN.len()..].to_vec());
        }

        if ciphertext.starts_with(MAGIC_V1) {
            let key = match &self.key {
                Some(k) => k,
                None => {
                    bail!(
                        "Data is encrypted with AES-256-GCM (AGSV1), but {} is not set. Please set {} to decrypt.",
                        ENV_VAR_NAME,
                        ENV_VAR_NAME
                    );
                }
            };

            if ciphertext.len() < 5 + 1 + 12 + 16 {
                bail!("Ciphertext too short (corrupted or truncated)");
            }

            let _scheme = ciphertext[5];
            let nonce_bytes = &ciphertext[6..18];
            let encrypted_body = &ciphertext[18..];

            let cipher = Aes256Gcm::new_from_slice(key)
                .map_err(|e| anyhow::anyhow!("Failed to initialize cipher: {}", e))?;
            let nonce = Nonce::from_slice(nonce_bytes);

            let decrypted = cipher
                .decrypt(
                    nonce,
                    Payload {
                        msg: encrypted_body,
                        aad: MAGIC_V1,
                    },
                )
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Decryption failed ({} mismatch or corrupted data: {})",
                        ENV_VAR_NAME,
                        e
                    )
                })?;

            return Ok(decrypted);
        }

        // Unframed raw fallback
        Ok(ciphertext.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn test_env_crypto_empty_key_is_plaintext() {
        let provider = EnvCryptoProvider::with_explicit_key(None);
        assert!(!provider.has_key());
        assert_eq!(provider.description(), "env [Plaintext: AGS_KEY not set]");

        let plain = b"{\"user\":\"alice\"}";
        let enc = provider.encrypt(plain).unwrap();
        assert!(enc.starts_with(MAGIC_PLAIN));

        let dec = provider.decrypt(&enc).unwrap();
        assert_eq!(dec, plain);
    }

    #[test]
    fn test_env_crypto_with_key() {
        let provider = EnvCryptoProvider::with_explicit_key(Some("secret-passphrase"));
        assert!(provider.has_key());
        assert_eq!(provider.description(), "env [AES-256-GCM: AGS_KEY is set]");

        let plain = b"{\"user\":\"alice\"}";
        let enc = provider.encrypt(plain).unwrap();
        assert!(enc.starts_with(MAGIC_V1));

        let dec = provider.decrypt(&enc).unwrap();
        assert_eq!(dec, plain);

        // Attempting to decrypt with empty key should prompt that AGS_KEY is missing
        let no_key_provider = EnvCryptoProvider::with_explicit_key(None);
        let err = no_key_provider.decrypt(&enc).unwrap_err();
        assert!(err.to_string().contains("AGS_KEY is not set"));
    }

    #[test]
    fn test_env_crypto_env_var_priorities() {
        let _guard = ENV_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let prev_key = env::var(ENV_VAR_NAME).ok();
        let prev_legacy = env::var(LEGACY_ENV_VAR_NAME).ok();

        // 1. None set -> Plaintext
        env::remove_var(ENV_VAR_NAME);
        env::remove_var(LEGACY_ENV_VAR_NAME);
        let p_none = EnvCryptoProvider::new();
        assert!(!p_none.has_key());

        // 2. AGS_KEY set
        env::set_var(ENV_VAR_NAME, "test-key-primary");
        env::remove_var(LEGACY_ENV_VAR_NAME);
        let p_primary = EnvCryptoProvider::new();
        assert!(p_primary.has_key());

        // 3. Only legacy AGS_ENCRYPT_KEY set -> fallback succeeds
        env::remove_var(ENV_VAR_NAME);
        env::set_var(LEGACY_ENV_VAR_NAME, "test-key-legacy");
        let p_legacy = EnvCryptoProvider::new();
        assert!(p_legacy.has_key());

        // 4. Both set -> AGS_KEY takes precedence
        env::set_var(ENV_VAR_NAME, "primary-key");
        env::set_var(LEGACY_ENV_VAR_NAME, "legacy-key");
        let p_both = EnvCryptoProvider::new();
        let expected_key = EnvCryptoProvider::parse_key_str("primary-key");
        assert_eq!(p_both.key, expected_key);

        // Restore
        match prev_key {
            Some(v) => env::set_var(ENV_VAR_NAME, v),
            None => env::remove_var(ENV_VAR_NAME),
        }
        match prev_legacy {
            Some(v) => env::set_var(LEGACY_ENV_VAR_NAME, v),
            None => env::remove_var(LEGACY_ENV_VAR_NAME),
        }
    }
}
