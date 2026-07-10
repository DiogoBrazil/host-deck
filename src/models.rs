use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Password,
    PrivateKey,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SshConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub identity_file: Option<String>,
    pub group_name: String,
    pub notes: Option<String>,
    pub password_secret_key: Option<String>,
    pub key_passphrase_secret_key: Option<String>,
    /// AI provider bound to this server, when configured.
    #[serde(default)]
    pub provider_id: Option<String>,
    pub last_connected_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Create/update payload. Secrets are write-only and cleared after submit.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInput {
    pub name: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub auth_method: AuthMethod,
    pub identity_file: Option<String>,
    pub group_name: String,
    pub notes: Option<String>,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    pub save_passphrase: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
}

/// Frontend mirror of the backend `RemoteEntry` (an SFTP directory entry).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<i64>,
    pub permissions: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Openrouter,
}

impl ProviderKind {
    pub fn label(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::Openai => "OpenAI",
            ProviderKind::Openrouter => "OpenRouter",
        }
    }
}

/// Frontend mirror of the backend `AgentProvider` record.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AgentProvider {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// Keyring reference; presence means a key is stored.
    pub api_key_ref: Option<String>,
    pub created_at: String,
}

/// Create/update payload for a provider. The API key travels separately and
/// is write-only, like SSH passwords.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInput {
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

/// Frontend mirror of the backend `ModelCacheEntry`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelCacheEntry {
    pub provider_id: String,
    pub model_id: String,
    pub display_name: Option<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    /// Capability tree as returned by the provider, serialized as JSON.
    pub capabilities: String,
    pub fetched_at: String,
}

impl ModelCacheEntry {
    pub fn display(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.model_id)
    }

    /// Cost per million tokens (input, output) in USD, when the provider
    /// announces pricing (OpenRouter: USD per token in `pricing`).
    pub fn price_per_mtok(&self) -> Option<(f64, f64)> {
        let caps: serde_json::Value = serde_json::from_str(&self.capabilities).ok()?;
        let pricing = caps.get("pricing")?;
        let per_token = |key: &str| -> Option<f64> {
            let v = pricing.get(key)?;
            v.as_f64().or_else(|| v.as_str()?.parse().ok())
        };
        Some((per_token("prompt")? * 1e6, per_token("completion")? * 1e6))
    }
}

/// One streamed event of an agent turn (mirror of the backend `AgentEvent`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    Delta(StreamDelta),
    ToolUse {
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    CommandPrompt {
        call_id: String,
        tool: String,
        command: String,
    },
    Done {
        text: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StreamDelta {
    Text(String),
    Thinking(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// Frontend mirror of the backend `AppError` wire format.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum AppError {
    Validation(Vec<FieldError>),
    NotFound,
    Database(String),
    CredentialStoreUnavailable(String),
    CredentialStore(String),
    Ssh(String),
    Agent(String),
    Internal(String),
}

impl AppError {
    pub fn internal(msg: String) -> Self {
        AppError::Internal(msg)
    }

    pub fn from_js(value: JsValue) -> Self {
        if let Ok(err) = serde_wasm_bindgen::from_value::<AppError>(value.clone()) {
            return err;
        }
        AppError::Internal(
            value
                .as_string()
                .unwrap_or_else(|| "erro desconhecido na chamada ao backend".into()),
        )
    }

    /// User-facing message for non-form error displays.
    pub fn message(&self) -> String {
        match self {
            AppError::Validation(errors) => errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            AppError::NotFound => "Conexão não encontrada.".into(),
            AppError::Database(m) => format!("Erro no banco de dados: {m}"),
            AppError::CredentialStoreUnavailable(m) => {
                format!("Armazenamento seguro indisponível: {m}")
            }
            AppError::CredentialStore(m) => format!("Erro no armazenamento seguro: {m}"),
            AppError::Ssh(m) => m.clone(),
            AppError::Agent(m) => m.clone(),
            AppError::Internal(m) => format!("Erro interno: {m}"),
        }
    }
}
