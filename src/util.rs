use anyhow::{Context, Result};
use base64::prelude::*;
use std::fs;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Write data to file atomically using a temporary file in the same directory,
/// setting restrictive permissions (0600 on Unix) to protect sensitive credentials.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory: {}", parent.display()))?;
    }

    let temp_path = path.with_extension(format!("tmp.{}", rand::random::<u32>()));

    fs::write(&temp_path, data)
        .with_context(|| format!("Failed to write temporary file: {}", temp_path.display()))?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&temp_path)?.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(&temp_path, perms);
    }

    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "Failed to atomically rename {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

/// Decode the payload of a JWT token (middle segment) without validating the signature.
pub fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let unpadded = parts[1].trim_end_matches('=');
    let decoded = BASE64_URL_SAFE_NO_PAD.decode(unpadded).ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// Format relative time or RFC3339 timestamp
pub fn format_time_rfc3339(dt: &chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Global lock used across tests when mutating process environment variables
#[cfg(test)]
pub static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_write() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("sub/target.txt");
        let data = b"hello secure world";
        atomic_write(&target, data).unwrap();
        assert_eq!(fs::read(&target).unwrap(), data);
    }

    #[test]
    fn test_decode_jwt_payload() {
        // Simple JWT: {"sub":"1234567890","email":"test@example.com"}
        let header = BASE64_URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = BASE64_URL_SAFE_NO_PAD
            .encode(b"{\"sub\":\"1234567890\",\"email\":\"test@example.com\"}");
        let jwt = format!("{}.{}.fake_sig", header, payload);

        let parsed = decode_jwt_payload(&jwt).unwrap();
        assert_eq!(parsed["sub"], "1234567890");
        assert_eq!(parsed["email"], "test@example.com");
    }
}
