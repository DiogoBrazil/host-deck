//! Agente de IA: abstração de provedor (Fase 1).
//!
//! O laço agêntico (Fase 2) fala apenas o dialeto interno de `domain`;
//! cada adapter em `providers` traduz de/para o formato do provedor.

// O consumidor deste módulo é o laço agêntico da Fase 2; até ele existir,
// tudo aqui parece morto para o compilador. Remover quando a Fase 2 chegar.
#![allow(dead_code)]

pub mod domain;
pub mod models;
pub mod provider;
pub mod providers;
mod sse;

use crate::domain::{AgentProvider as ProviderRecord, ProviderKind};
use provider::AgentProvider;
use providers::anthropic::AnthropicProvider;
use providers::openai::OpenAiCompatProvider;

/// Instancia o adapter correspondente ao registro persistido.
///
/// A chave de API vem do keyring (ver `infra::credential_store::api_key_ref`)
/// e só existe em memória, dentro do adapter.
pub fn build_provider(record: &ProviderRecord, api_key: String) -> Box<dyn AgentProvider> {
    match record.kind {
        ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
            record.id.clone(),
            record.base_url.clone(),
            api_key,
        )),
        ProviderKind::Openai => Box::new(OpenAiCompatProvider::openai(
            record.id.clone(),
            record.base_url.clone(),
            api_key,
        )),
        ProviderKind::Openrouter => Box::new(OpenAiCompatProvider::openrouter(
            record.id.clone(),
            record.base_url.clone(),
            api_key,
        )),
    }
}
