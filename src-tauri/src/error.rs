use serde::Serialize;

use crate::domain::FieldError;

/// Erro da aplicação, serializado para o frontend com tag `kind`.
/// As mensagens são exibidas ao usuário: nunca devem conter segredos.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum AppError {
    #[error("dados inválidos")]
    Validation(Vec<FieldError>),

    #[error("conexão não encontrada")]
    NotFound,

    #[error("erro no banco de dados: {0}")]
    Database(String),

    #[error("armazenamento seguro indisponível: {0}")]
    CredentialStoreUnavailable(String),

    #[error("erro no armazenamento seguro: {0}")]
    CredentialStore(String),

    #[error("{0}")]
    Ssh(String),

    #[error("erro interno: {0}")]
    Internal(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl From<Vec<FieldError>> for AppError {
    fn from(errors: Vec<FieldError>) -> Self {
        AppError::Validation(errors)
    }
}

pub type AppResult<T> = Result<T, AppError>;
