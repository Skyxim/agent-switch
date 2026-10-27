use super::CryptoProvider;
use anyhow::{bail, Result};

pub const MAGIC_PLAIN: &[u8] = b"AGSPLAIN";

pub struct PlaintextCryptoProvider;

impl CryptoProvider for PlaintextCryptoProvider {
    fn name(&self) -> &'static str {
        "plain"
    }

    fn description(&self) -> String {
        "Plaintext storage without encryption (magic: AGSPLAIN)".to_string()
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(MAGIC_PLAIN.len() + plaintext.len());
        out.extend_from_slice(MAGIC_PLAIN);
        out.extend_from_slice(plaintext);
        Ok(out)
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.starts_with(MAGIC_PLAIN) {
            Ok(ciphertext[MAGIC_PLAIN.len()..].to_vec())
        } else if ciphertext.starts_with(b"AGSV1") {
            bail!("Data was encrypted with AGSV1; cannot decrypt using plain provider");
        } else {
            // Unframed legacy/plain data fallback
            Ok(ciphertext.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_encrypt_decrypt() {
        let p = PlaintextCryptoProvider;
        let data = b"hello secret token 12345";
        let encrypted = p.encrypt(data).unwrap();
        assert!(encrypted.starts_with(MAGIC_PLAIN));
        let decrypted = p.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, data);
    }
}
