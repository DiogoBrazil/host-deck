use serde::{Deserialize, Serialize};

use super::FieldError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Openrouter,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Openrouter => "openrouter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(ProviderKind::Anthropic),
            "openai" => Some(ProviderKind::Openai),
            "openrouter" => Some(ProviderKind::Openrouter),
            _ => None,
        }
    }
}

/// SQLite record for an AI provider configuration.
///
/// The API key is never stored here; `api_key_ref` is a keyring reference,
/// mirroring how `ssh_connections` handles passwords.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    /// Override for API-compatible gateways; `None` means the official endpoint.
    pub base_url: Option<String>,
    /// Default model; each connection may pick its own.
    pub model: Option<String>,
    pub api_key_ref: Option<String>,
    pub created_at: String,
}

/// Form payload received from the frontend.
///
/// The API key only exists in memory until it is stored in the keyring.
#[derive(Debug, Deserialize)]
pub struct ProviderInput {
    pub kind: ProviderKind,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Normalized and validated provider data ready for persistence.
#[derive(Debug)]
pub struct ValidatedProvider {
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

impl ProviderInput {
    pub fn validate(&self) -> Result<ValidatedProvider, Vec<FieldError>> {
        let mut errors = Vec::new();

        let base_url = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        if let Some(url) = &base_url {
            if !url.starts_with("http://") && !url.starts_with("https://") {
                errors.push(FieldError {
                    field: "base_url".into(),
                    message: "URL base deve começar com http:// ou https://.".into(),
                });
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let label = {
            let trimmed = self.label.trim();
            if trimmed.is_empty() {
                match self.kind {
                    ProviderKind::Anthropic => "Anthropic".to_string(),
                    ProviderKind::Openai => "OpenAI".to_string(),
                    ProviderKind::Openrouter => "OpenRouter".to_string(),
                }
            } else {
                trimmed.to_string()
            }
        };

        Ok(ValidatedProvider {
            kind: self.kind,
            label,
            base_url,
            model: self
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
        })
    }
}

/// Cached entry from a provider's `GET /v1/models`, persisted in
/// `agent_model_cache`. The UI derives its controls from `capabilities`
/// instead of a hardcoded model list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCacheEntry {
    pub provider_id: String,
    pub model_id: String,
    pub display_name: Option<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    /// Capability tree as returned by the provider, serialized as JSON.
    pub capabilities: String,
    /// Fetch timestamp (RFC 3339), used for cache invalidation.
    pub fetched_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input() -> ProviderInput {
        ProviderInput {
            kind: ProviderKind::Anthropic,
            label: String::new(),
            base_url: None,
            model: None,
        }
    }

    #[test]
    fn applies_default_label_from_kind() {
        let v = base_input().validate().unwrap();
        assert_eq!(v.label, "Anthropic");

        let mut input = base_input();
        input.kind = ProviderKind::Openai;
        assert_eq!(input.validate().unwrap().label, "OpenAI");

        input.kind = ProviderKind::Openrouter;
        assert_eq!(input.validate().unwrap().label, "OpenRouter");
    }

    #[test]
    fn keeps_explicit_label_and_normalizes_blanks() {
        let mut input = base_input();
        input.label = " Gateway interno ".into();
        input.base_url = Some("   ".into());
        input.model = Some(" claude-sonnet-5 ".into());
        let v = input.validate().unwrap();
        assert_eq!(v.label, "Gateway interno");
        assert!(v.base_url.is_none());
        assert_eq!(v.model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn rejects_base_url_without_scheme() {
        let mut input = base_input();
        input.base_url = Some("gateway.interno:8080".into());
        let errs = input.validate().unwrap_err();
        assert_eq!(errs[0].field, "base_url");
    }

    #[test]
    fn kind_serializes_as_snake_case() {
        // The frontend and the `kind` CHECK constraint depend on these names.
        assert_eq!(ProviderKind::Anthropic.as_str(), "anthropic");
        assert_eq!(ProviderKind::Openai.as_str(), "openai");
        assert_eq!(ProviderKind::Openrouter.as_str(), "openrouter");
        assert_eq!(ProviderKind::parse("openai"), Some(ProviderKind::Openai));
        assert_eq!(
            ProviderKind::parse("openrouter"),
            Some(ProviderKind::Openrouter)
        );
        assert_eq!(ProviderKind::parse("gemini"), None);

        let json = serde_json::to_string(&ProviderKind::Anthropic).unwrap();
        assert_eq!(json, "\"anthropic\"");
    }
}
