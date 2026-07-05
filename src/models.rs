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
            AppError::Internal(m) => format!("Erro interno: {m}"),
        }
    }
}
