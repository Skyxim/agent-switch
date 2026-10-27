use base64::prelude::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Codex DTOs
// ---------------------------------------------------------------------------

/// Strongly-typed JWT Claims for Codex id_token / access_token payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CodexJwtClaims {
    #[serde(default)]
    pub sub: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub iss: Option<String>,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
}

impl CodexJwtClaims {
    /// Attempt to parse JWT claims from an encoded JWT string.
    pub fn from_jwt(jwt: &str) -> Option<Self> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() < 2 {
            return None;
        }
        let unpadded = parts[1].trim_end_matches('=');
        let decoded = BASE64_URL_SAFE_NO_PAD.decode(unpadded).ok()?;
        serde_json::from_slice(&decoded).ok()
    }
}

/// Strongly-typed response from Codex usage endpoint (`https://chatgpt.com/backend-api/wham/usage`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CodexUsageResponse {
    #[serde(default)]
    pub plan_type: Option<String>,
    #[serde(default)]
    pub rate_limit: Option<CodexRateLimit>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CodexRateLimit {
    #[serde(default)]
    pub primary_window: Option<CodexQuotaWindowDto>,
    #[serde(default)]
    pub secondary_window: Option<CodexQuotaWindowDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CodexQuotaWindowDto {
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub limit_window_seconds: Option<i64>,
    #[serde(default)]
    pub window_seconds: Option<i64>,
    #[serde(default)]
    pub reset_after_seconds: Option<i64>,
}

impl CodexQuotaWindowDto {
    /// Whether this window has valid rate limit or usage information.
    pub fn is_valid(&self) -> bool {
        self.used_percent.is_some()
            || self.reset_after_seconds.is_some()
            || self.limit_window_seconds.is_some()
            || self.window_seconds.is_some()
    }

    /// Effective window duration in seconds.
    pub fn effective_window_seconds(&self) -> Option<i64> {
        self.limit_window_seconds.or(self.window_seconds)
    }

    /// Convert into domain `QuotaWindow`.
    pub fn to_quota_window(&self) -> Option<crate::model::QuotaWindow> {
        let used_pct = self.used_percent.unwrap_or(0.0).clamp(0.0, 100.0);
        let rem_pct = (100.0 - used_pct).clamp(0.0, 100.0);

        let secs_opt = self.effective_window_seconds();
        let (window_type, display_label) = match secs_opt {
            Some(18000) => ("5h".to_string(), "5h".to_string()),
            Some(604800) => ("weekly".to_string(), "Weekly".to_string()),
            Some(86400) => ("daily".to_string(), "Daily".to_string()),
            Some(2592000) => ("monthly".to_string(), "Monthly".to_string()),
            Some(s) if s > 0 && s % 86400 == 0 => {
                let d = s / 86400;
                (format!("{}d", d), format!("{}-Day", d))
            }
            Some(s) if s > 0 && s % 3600 == 0 => {
                let h = s / 3600;
                (format!("{}h", h), format!("{}-Hour", h))
            }
            Some(s) if s > 0 => (format!("{}s", s), format!("{}-Sec", s)),
            _ => ("window".to_string(), "Window".to_string()),
        };

        let reset_after_s = self.reset_after_seconds;
        let resets_in_text = reset_after_s.map(crate::model::format_resets_in_seconds);
        let reset_time =
            reset_after_s.map(|s| (Utc::now() + chrono::Duration::seconds(s)).to_rfc3339());

        Some(crate::model::QuotaWindow::new(
            window_type,
            display_label,
            rem_pct,
            used_pct,
            reset_time,
            resets_in_text,
        ))
    }
}

impl CodexUsageResponse {
    /// Parse and sort quota windows from the response.
    pub fn parse_quota_windows(&self) -> Vec<crate::model::QuotaWindow> {
        let mut quotas = Vec::new();
        if let Some(rate_limit) = &self.rate_limit {
            if let Some(primary) = &rate_limit.primary_window {
                if primary.is_valid() {
                    if let Some(qw) = primary.to_quota_window() {
                        quotas.push(qw);
                    }
                }
            }
            if let Some(secondary) = &rate_limit.secondary_window {
                if secondary.is_valid() {
                    if let Some(qw) = secondary.to_quota_window() {
                        quotas.push(qw);
                    }
                }
            }
        }

        quotas.sort_by_key(|q| match q.window_type.as_str() {
            "5h" => 0,
            "daily" => 1,
            "weekly" => 2,
            "monthly" => 3,
            _ => 4,
        });

        quotas
    }
}

// ---------------------------------------------------------------------------
// Antigravity (AGY) DTOs
// ---------------------------------------------------------------------------

/// Response from Google OAuth token refresh endpoint (`https://oauth2.googleapis.com/token`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GoogleOAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Fallback / legacy `oauth_creds.json` format used by Gemini / Antigravity CLI.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GeminiOAuthCredentialsFile {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub id_token_email: Option<String>,
    #[serde(default)]
    pub expiry_date: Option<i64>,
    #[serde(default)]
    pub token_type: Option<String>,
}

/// Response from `loadCodeAssist` endpoint (`https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyLoadCodeAssistResponse {
    #[serde(default, rename = "cloudaicompanionProject")]
    pub cloudaicompanion_project: Option<AgyCompanionProject>,
    #[serde(default, rename = "paidTier")]
    pub paid_tier: Option<AgyTierInfo>,
    #[serde(default, rename = "currentTier")]
    pub current_tier: Option<AgyTierInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AgyCompanionProject {
    Id(String),
    Object {
        #[serde(default)]
        id: Option<String>,
    },
}

impl AgyCompanionProject {
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Id(s) => Some(s.as_str()),
            Self::Object { id } => id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyTierInfo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl AgyTierInfo {
    pub fn display_tier(&self) -> Option<&str> {
        self.name.as_deref().or(self.id.as_deref())
    }
}

impl AgyLoadCodeAssistResponse {
    pub fn project_id(&self) -> Option<String> {
        self.cloudaicompanion_project
            .as_ref()
            .and_then(|p| p.project_id().map(|s| s.to_string()))
    }

    pub fn tier_name(&self) -> String {
        self.paid_tier
            .as_ref()
            .and_then(|t| t.display_tier())
            .or_else(|| self.current_tier.as_ref().and_then(|t| t.display_tier()))
            .unwrap_or("Standard")
            .to_string()
    }
}

/// Response from `retrieveUserQuotaSummary` endpoint (`https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyUserQuotaSummaryResponse {
    #[serde(default)]
    pub groups: Vec<AgyQuotaGroupDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyQuotaGroupDto {
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub buckets: Vec<AgyQuotaBucketDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyQuotaBucketDto {
    #[serde(default, rename = "bucketId", alias = "bucket_id")]
    pub bucket_id: Option<String>,
    #[serde(default, rename = "displayName", alias = "display_name")]
    pub display_name: Option<String>,
    #[serde(default, rename = "modelId", alias = "model_id")]
    pub model_id: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default, rename = "remainingFraction", alias = "remaining_fraction")]
    pub remaining_fraction: Option<f64>,
    #[serde(default, rename = "resetTime", alias = "reset_time")]
    pub reset_time: Option<String>,
    #[serde(default, rename = "tokenType", alias = "token_type")]
    pub token_type: Option<String>,
}

impl AgyQuotaBucketDto {
    /// Derive human-readable window label matching AGY conventions without time-based guessing.
    pub fn derive_window_label(&self) -> String {
        // 1. Explicit token_type check
        if let Some(ref tt) = self.token_type {
            let lower_tt = tt.to_lowercase();
            if tt.eq_ignore_ascii_case("WTUS")
                || lower_tt.contains("week")
                || lower_tt.contains("weekly")
            {
                return "Weekly Window".to_string();
            }
            if lower_tt.contains("5h") || lower_tt.contains("18000") || lower_tt.contains("five") {
                return "5-Hour Window".to_string();
            }
        }

        // 2. Check text keywords (window / bucket_id / display_name)
        let window_str = self.window.as_deref().unwrap_or("");
        let bucket_id = self.bucket_id.as_deref().unwrap_or("");
        let display_name = self.display_name.as_deref().unwrap_or("");
        let text = format!("{} {} {}", window_str, bucket_id, display_name).to_lowercase();

        if text.contains("604800") || text.contains("week") || text.contains("weekly") {
            "Weekly Window".to_string()
        } else if text.contains("18000") || text.contains("5h") || text.contains("five") {
            "5-Hour Window".to_string()
        } else if !display_name.is_empty() {
            display_name.to_string()
        } else if !window_str.is_empty() {
            format!("{}-Hour Window", window_str)
        } else {
            "Weekly Window".to_string()
        }
    }
}

/// Response from direct `retrieveUserQuota` endpoint (`https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AgyUserQuotaDirectResponse {
    #[serde(default)]
    pub buckets: Vec<AgyQuotaBucketDto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_jwt_claims_parsing() {
        let payload_json = r#"{"sub":"user_12345","email":"dev@openai.com","plan":"team"}"#;
        let header = BASE64_URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = BASE64_URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        let token = format!("{}.{}.sig", header, payload);

        let claims = CodexJwtClaims::from_jwt(&token).expect("Failed to parse claims");
        assert_eq!(claims.sub.as_deref(), Some("user_12345"));
        assert_eq!(claims.email.as_deref(), Some("dev@openai.com"));
        assert_eq!(claims.plan.as_deref(), Some("team"));
    }

    #[test]
    fn test_codex_usage_response_deserialization() {
        let json_data = r#"{
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 25.5,
                    "limit_window_seconds": 18000,
                    "reset_after_seconds": 3600
                },
                "secondary_window": {
                    "used_percent": 50.0,
                    "window_seconds": 604800,
                    "reset_after_seconds": 86400
                }
            }
        }"#;

        let resp: CodexUsageResponse = serde_json::from_str(json_data).unwrap();
        assert_eq!(resp.plan_type.as_deref(), Some("plus"));

        let windows = resp.parse_quota_windows();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].window_type, "5h");
        assert_eq!(windows[0].display_label, "5h");
        assert_eq!(windows[0].used_percent, 25.5);
        assert_eq!(windows[0].remaining_percent, 74.5);
        assert_eq!(windows[1].window_type, "weekly");
        assert_eq!(windows[1].display_label, "Weekly");
    }

    #[test]
    fn test_agy_token_and_load_code_assist_dto() {
        let oauth_json = r#"{
            "access_token": "ya29.sample_token",
            "refresh_token": "1//refresh_sample",
            "expires_in": 3600,
            "token_type": "Bearer"
        }"#;
        let token_resp: GoogleOAuthTokenResponse = serde_json::from_str(oauth_json).unwrap();
        assert_eq!(token_resp.access_token, "ya29.sample_token");
        assert_eq!(
            token_resp.refresh_token.as_deref(),
            Some("1//refresh_sample")
        );
        assert_eq!(token_resp.expires_in, Some(3600));

        let code_assist_json = r#"{
            "cloudaicompanionProject": "sample-project-id",
            "paidTier": {
                "id": "tier_pro",
                "name": "Google AI Pro"
            }
        }"#;
        let ca_resp: AgyLoadCodeAssistResponse = serde_json::from_str(code_assist_json).unwrap();
        assert_eq!(ca_resp.project_id().as_deref(), Some("sample-project-id"));
        assert_eq!(ca_resp.tier_name(), "Google AI Pro");

        let object_project_json = r#"{
            "cloudaicompanionProject": {
                "id": "project-from-object"
            },
            "currentTier": {
                "name": "Standard"
            }
        }"#;
        let ca_obj_resp: AgyLoadCodeAssistResponse =
            serde_json::from_str(object_project_json).unwrap();
        assert_eq!(
            ca_obj_resp.project_id().as_deref(),
            Some("project-from-object")
        );
        assert_eq!(ca_obj_resp.tier_name(), "Standard");
    }

    #[test]
    fn test_agy_quota_summary_dto() {
        let summary_json = r#"{
            "groups": [
                {
                    "displayName": "Gemini models",
                    "buckets": [
                        {
                            "bucketId": "gemini-5h",
                            "window": "18000",
                            "remainingFraction": 0.85,
                            "resetTime": "2026-10-27T02:00:00Z"
                        }
                    ]
                }
            ]
        }"#;

        let summary: AgyUserQuotaSummaryResponse = serde_json::from_str(summary_json).unwrap();
        assert_eq!(summary.groups.len(), 1);
        let group = &summary.groups[0];
        assert_eq!(group.display_name.as_deref(), Some("Gemini models"));
        assert_eq!(group.buckets.len(), 1);
        let bucket = &group.buckets[0];
        assert_eq!(bucket.derive_window_label(), "5-Hour Window");
        assert_eq!(bucket.remaining_fraction, Some(0.85));
    }

    #[test]
    fn test_derive_window_label_metadata_matching() {
        let now = Utc::now();
        let in_4_days = (now + chrono::Duration::hours(118)).to_rfc3339();
        let in_2_hours =
            (now + chrono::Duration::hours(2) + chrono::Duration::minutes(43)).to_rfc3339();
        let in_10_hours = (now + chrono::Duration::hours(10)).to_rfc3339();

        // 1. Bucket with "weekly" in window -> "Weekly Window"
        let bucket_weekly = AgyQuotaBucketDto {
            bucket_id: None,
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: Some("weekly".to_string()),
            remaining_fraction: Some(0.192),
            reset_time: Some(in_4_days.clone()),
            token_type: None,
        };
        assert_eq!(bucket_weekly.derive_window_label(), "Weekly Window");

        // 2. Bucket with "5h" in window -> "5-Hour Window"
        let bucket_5h = AgyQuotaBucketDto {
            bucket_id: Some("gemini-window".to_string()),
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: Some("5h".to_string()),
            remaining_fraction: Some(0.192),
            reset_time: Some(in_4_days),
            token_type: None,
        };
        assert_eq!(bucket_5h.derive_window_label(), "5-Hour Window");

        // 3. Bucket with "gemini-5h" in bucket_id -> "5-Hour Window"
        let bucket_gemini_5h = AgyQuotaBucketDto {
            bucket_id: Some("gemini-5h".to_string()),
            display_name: None,
            model_id: Some("gemini-2.5-flash".to_string()),
            window: None,
            remaining_fraction: Some(0.65),
            reset_time: Some(in_2_hours.clone()),
            token_type: None,
        };
        assert_eq!(bucket_gemini_5h.derive_window_label(), "5-Hour Window");

        // 4. Bucket with WTUS token_type -> "Weekly Window"
        let bucket_wtus = AgyQuotaBucketDto {
            bucket_id: None,
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: None,
            remaining_fraction: Some(0.188),
            reset_time: Some(in_10_hours.clone()),
            token_type: Some("WTUS".to_string()),
        };
        assert_eq!(bucket_wtus.derive_window_label(), "Weekly Window");

        // 5. Bucket with no keywords -> default "Weekly Window"
        let bucket_fallback = AgyQuotaBucketDto {
            bucket_id: None,
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: None,
            remaining_fraction: Some(0.5),
            reset_time: Some(in_10_hours),
            token_type: None,
        };
        assert_eq!(bucket_fallback.derive_window_label(), "Weekly Window");

        // 6. Bucket with token_type containing "weekly" case-insensitively -> "Weekly Window"
        let bucket_weekly_token = AgyQuotaBucketDto {
            bucket_id: None,
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: None,
            remaining_fraction: Some(0.5),
            reset_time: Some(in_2_hours.clone()),
            token_type: Some("WEEKLY".to_string()),
        };
        assert_eq!(bucket_weekly_token.derive_window_label(), "Weekly Window");

        // 7. Bucket with WTUS token_type taking precedence over window text -> "Weekly Window"
        let bucket_wtus_5h = AgyQuotaBucketDto {
            bucket_id: None,
            display_name: None,
            model_id: Some("gemini-2.5-pro".to_string()),
            window: Some("5h".to_string()),
            remaining_fraction: Some(0.5),
            reset_time: Some(in_2_hours),
            token_type: Some("wtus".to_string()),
        };
        assert_eq!(bucket_wtus_5h.derive_window_label(), "Weekly Window");
    }
}
