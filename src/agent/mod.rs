pub mod agy;
pub mod codex;
pub mod dto;
pub mod traits;

pub use agy::{default_creds_path, default_dir, default_token_path, AgyAgent};
pub use codex::{default_auth_path, CodexAgent};
pub use dto::*;
pub use traits::*;

/// Supported agent identifiers.
pub const SUPPORTED_AGENT_IDS: &[&str] = &["codex", "agy"];

/// Returns all supported agent application instances.
pub fn all_agents() -> Vec<Box<dyn AgentApp>> {
    vec![Box::new(CodexAgent::new()), Box::new(AgyAgent::new())]
}

/// Look up an agent application by its id or common name (case-insensitive).
pub fn get_agent(name: &str) -> Option<Box<dyn AgentApp>> {
    match name.trim().to_ascii_lowercase().as_str() {
        "codex" | "chatgpt" | "openai" => Some(Box::new(CodexAgent::new())),
        "agy" | "antigravity" | "gemini" => Some(Box::new(AgyAgent::new())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_agents_returns_registered_agents() {
        let agents = all_agents();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id(), "codex");
        assert_eq!(agents[1].id(), "agy");
    }

    #[test]
    fn test_get_agent_lookup() {
        assert!(get_agent("codex").is_some());
        assert_eq!(get_agent("codex").unwrap().id(), "codex");
        assert_eq!(get_agent("CODEX").unwrap().id(), "codex");
        assert_eq!(get_agent("chatgpt").unwrap().id(), "codex");
        assert_eq!(get_agent("openai").unwrap().id(), "codex");

        assert!(get_agent("agy").is_some());
        assert_eq!(get_agent("agy").unwrap().id(), "agy");
        assert_eq!(get_agent("Antigravity").unwrap().id(), "agy");
        assert_eq!(get_agent("gemini").unwrap().id(), "agy");

        assert!(get_agent("unknown").is_none());
        assert!(get_agent("").is_none());
    }
}
