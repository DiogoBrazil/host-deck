//! Sincroniza a listagem de modelos do provedor com `agent_model_cache`.

use std::sync::Mutex;

use chrono::Utc;
use rusqlite::Connection;

use super::provider::AgentProvider;
use crate::domain::ModelCacheEntry;
use crate::error::AppResult;
use crate::infra::agent_repository;

/// Busca `list_models()` no provedor e substitui o cache persistido.
///
/// O lock do banco só é tomado depois do `await`: nada de segurar um
/// `Mutex` síncrono através de I/O de rede.
pub async fn refresh_model_cache(
    conn: &Mutex<Connection>,
    provider: &dyn AgentProvider,
) -> AppResult<Vec<ModelCacheEntry>> {
    let models = provider.list_models().await.map_err(crate::error::AppError::from)?;
    let fetched_at = Utc::now().to_rfc3339();
    let entries: Vec<ModelCacheEntry> = models
        .into_iter()
        .map(|m| m.into_cache_entry(provider.id(), fetched_at.clone()))
        .collect();

    let mut guard = conn.lock().unwrap();
    agent_repository::replace_model_cache(&mut guard, provider.id(), &entries)?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::domain::ModelInfo;
    use crate::agent::provider::mock::MockProvider;
    use crate::domain::agent_provider::ValidatedProvider;
    use crate::domain::ProviderKind;
    use crate::infra::db::Db;

    #[tokio::test]
    async fn refresh_persists_listing_in_cache() {
        let db = Db::open_in_memory().unwrap();
        let record = {
            let conn = db.0.lock().unwrap();
            agent_repository::insert(
                &conn,
                &ValidatedProvider {
                    kind: ProviderKind::Openrouter,
                    label: "OpenRouter".into(),
                    base_url: None,
                    model: None,
                },
            )
            .unwrap()
        };

        let mut provider = MockProvider::new(&record.id);
        provider.models = vec![ModelInfo {
            id: "anthropic/claude-sonnet-5".into(),
            display_name: Some("Claude Sonnet 5".into()),
            max_input_tokens: Some(1_000_000),
            max_output_tokens: Some(128_000),
            capabilities: serde_json::json!({"supported_parameters": ["tools"]}),
        }];

        let entries = refresh_model_cache(&db.0, &provider).await.unwrap();
        assert_eq!(entries.len(), 1);

        let conn = db.0.lock().unwrap();
        let cached = agent_repository::list_model_cache(&conn, &record.id).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].model_id, "anthropic/claude-sonnet-5");
        assert_eq!(
            cached[0].capabilities,
            r#"{"supported_parameters":["tools"]}"#
        );
    }
}
