use serde::Serialize;

use crate::bindings::tauri::{invoke, invoke_no_args};
use crate::models::{
    AgentProvider, AppError, ConnectionInput, ModelCacheEntry, ProviderInput, SshConnection,
};

#[derive(Serialize)]
struct IdArgs<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct InputArgs<'a> {
    input: &'a ConnectionInput,
}

#[derive(Serialize)]
struct IdInputArgs<'a> {
    id: &'a str,
    input: &'a ConnectionInput,
}

pub async fn list_connections() -> Result<Vec<SshConnection>, AppError> {
    invoke_no_args("list_connections").await
}

pub async fn create_connection(input: &ConnectionInput) -> Result<SshConnection, AppError> {
    invoke("create_connection", &InputArgs { input }).await
}

pub async fn update_connection(
    id: &str,
    input: &ConnectionInput,
) -> Result<SshConnection, AppError> {
    invoke("update_connection", &IdInputArgs { id, input }).await
}

pub async fn delete_connection(id: &str) -> Result<(), AppError> {
    invoke("delete_connection", &IdArgs { id }).await
}

// Provedores de IA

#[derive(Serialize)]
struct ProviderArgs<'a> {
    input: &'a ProviderInput,
    #[serde(rename = "apiKey")]
    api_key: Option<&'a str>,
}

#[derive(Serialize)]
struct IdProviderArgs<'a> {
    id: &'a str,
    input: &'a ProviderInput,
    #[serde(rename = "apiKey")]
    api_key: Option<&'a str>,
}

#[derive(Serialize)]
struct ProviderIdArgs<'a> {
    #[serde(rename = "providerId")]
    provider_id: &'a str,
}

#[derive(Serialize)]
struct SetProviderArgs<'a> {
    #[serde(rename = "connectionId")]
    connection_id: &'a str,
    #[serde(rename = "providerId")]
    provider_id: Option<&'a str>,
}

#[derive(Serialize)]
struct SessionArgs<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
}

#[derive(Serialize)]
struct ConfirmCommandArgs<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    #[serde(rename = "callId")]
    call_id: &'a str,
    accept: bool,
}

pub async fn list_providers() -> Result<Vec<AgentProvider>, AppError> {
    invoke_no_args("list_providers").await
}

pub async fn create_provider(
    input: &ProviderInput,
    api_key: Option<&str>,
) -> Result<AgentProvider, AppError> {
    invoke("create_provider", &ProviderArgs { input, api_key }).await
}

pub async fn update_provider(
    id: &str,
    input: &ProviderInput,
    api_key: Option<&str>,
) -> Result<AgentProvider, AppError> {
    invoke("update_provider", &IdProviderArgs { id, input, api_key }).await
}

pub async fn delete_provider(id: &str) -> Result<(), AppError> {
    invoke("delete_provider", &IdArgs { id }).await
}

pub async fn list_cached_models(provider_id: &str) -> Result<Vec<ModelCacheEntry>, AppError> {
    invoke("list_cached_models", &ProviderIdArgs { provider_id }).await
}

/// Busca a listagem no provedor e substitui o cache persistido.
pub async fn refresh_models(provider_id: &str) -> Result<Vec<ModelCacheEntry>, AppError> {
    invoke("agent_refresh_models", &ProviderIdArgs { provider_id }).await
}

pub async fn set_connection_provider(
    connection_id: &str,
    provider_id: Option<&str>,
) -> Result<(), AppError> {
    invoke(
        "set_connection_provider",
        &SetProviderArgs {
            connection_id,
            provider_id,
        },
    )
    .await
}

// Agente

#[derive(Serialize)]
struct ConsentArgs {
    granted: bool,
}

pub async fn agent_cancel(session_id: &str) -> Result<(), AppError> {
    invoke("agent_cancel", &SessionArgs { session_id }).await
}

/// Exatamente o texto do terminal que iria ao provedor (cauda do scrollback,
/// sem ANSI, segredos redigidos); exibido no pedido de consentimento.
pub async fn agent_context_preview(session_id: &str) -> Result<String, AppError> {
    invoke("agent_context_preview", &SessionArgs { session_id }).await
}

pub async fn get_agent_consent() -> Result<bool, AppError> {
    invoke_no_args("get_agent_consent").await
}

pub async fn set_agent_consent(granted: bool) -> Result<(), AppError> {
    invoke("set_agent_consent", &ConsentArgs { granted }).await
}

pub async fn confirm_agent_command(
    session_id: &str,
    call_id: &str,
    accept: bool,
) -> Result<(), AppError> {
    invoke(
        "confirm_agent_command",
        &ConfirmCommandArgs {
            session_id,
            call_id,
            accept,
        },
    )
    .await
}
