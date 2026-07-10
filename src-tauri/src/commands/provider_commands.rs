//! CRUD de provedores de IA para a tela de configuração (Fase 3).
//!
//! A chave de API segue o mesmo tratamento das credenciais SSH: vai para o
//! keyring do SO e o banco guarda apenas a referência (`api_key_ref`).

use tauri::State;

use crate::domain::{AgentProvider, ModelCacheEntry, ProviderInput};
use crate::error::AppResult;
use crate::infra::agent_repository as repo;
use crate::infra::credential_store::api_key_ref;
use crate::infra::db::Db;
use crate::infra::sqlite_repository;
use crate::state::CredStore;

#[tauri::command]
pub async fn list_providers(db: State<'_, Db>) -> AppResult<Vec<AgentProvider>> {
    let conn = db.0.lock().unwrap();
    repo::list(&conn)
}

#[tauri::command]
pub async fn create_provider(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    input: ProviderInput,
    api_key: Option<String>,
) -> AppResult<AgentProvider> {
    let validated = input.validate()?;
    let conn = db.0.lock().unwrap();
    let created = repo::insert(&conn, &validated)?;

    match persist_api_key(&conn, &store, &created.id, api_key.as_deref()) {
        Ok(()) => repo::get(&conn, &created.id),
        Err(err) => {
            // Sem chave utilizável o registro seria inerte; desfaz como o
            // create_connection faz com credenciais SSH.
            let _ = repo::delete(&conn, &created.id);
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn update_provider(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    id: String,
    input: ProviderInput,
    api_key: Option<String>,
) -> AppResult<AgentProvider> {
    let validated = input.validate()?;
    let conn = db.0.lock().unwrap();
    repo::update(&conn, &id, &validated)?;
    persist_api_key(&conn, &store, &id, api_key.as_deref())?;
    repo::get(&conn, &id)
}

/// Remove o provedor, sua chave no keyring e o cache de modelos (CASCADE);
/// conexões que o usavam ficam sem provedor (SET NULL).
#[tauri::command]
pub async fn delete_provider(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    id: String,
) -> AppResult<()> {
    store.0.delete(&api_key_ref(&id))?;
    let conn = db.0.lock().unwrap();
    repo::delete(&conn, &id)
}

/// Modelos do cache persistido (sem rede); `agent_refresh_models` renova.
#[tauri::command]
pub async fn list_cached_models(
    db: State<'_, Db>,
    provider_id: String,
) -> AppResult<Vec<ModelCacheEntry>> {
    let conn = db.0.lock().unwrap();
    repo::list_model_cache(&conn, &provider_id)
}

/// Vincula (ou desvincula) o provedor usado com uma conexão SSH.
#[tauri::command]
pub async fn set_connection_provider(
    db: State<'_, Db>,
    connection_id: String,
    provider_id: Option<String>,
) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    sqlite_repository::set_provider(&conn, &connection_id, provider_id.as_deref())
}

/// Guarda a chave no keyring e grava a referência no banco. Chave ausente ou
/// vazia mantém a atual (o formulário nunca reapresenta segredos).
fn persist_api_key(
    conn: &rusqlite::Connection,
    store: &CredStore,
    id: &str,
    api_key: Option<&str>,
) -> AppResult<()> {
    let Some(key) = api_key.filter(|k| !k.trim().is_empty()) else {
        return Ok(());
    };
    let entry = api_key_ref(id);
    store.0.set(&entry, key.trim())?;
    repo::set_api_key_ref(conn, id, Some(&entry))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::ProviderKind;
    use crate::infra::credential_store::mock::MockStore;
    use crate::infra::db::Db;

    fn input(kind: ProviderKind) -> ProviderInput {
        ProviderInput {
            kind,
            label: String::new(),
            base_url: None,
            model: None,
        }
    }

    fn setup() -> (Db, CredStore) {
        (
            Db::open_in_memory().unwrap(),
            CredStore(Arc::new(MockStore::default())),
        )
    }

    #[test]
    fn api_key_goes_to_store_and_db_keeps_only_the_ref() {
        let (db, store) = setup();
        let conn = db.0.lock().unwrap();
        let created = repo::insert(&conn, &input(ProviderKind::Anthropic).validate().unwrap())
            .unwrap();

        persist_api_key(&conn, &store, &created.id, Some(" sk-ant-teste ")).unwrap();

        let expected_ref = api_key_ref(&created.id);
        let fetched = repo::get(&conn, &created.id).unwrap();
        assert_eq!(fetched.api_key_ref.as_deref(), Some(expected_ref.as_str()));
        assert_eq!(
            store.0.get(&expected_ref).unwrap().as_deref(),
            Some("sk-ant-teste"),
        );
    }

    #[test]
    fn empty_or_absent_key_keeps_the_current_one() {
        let (db, store) = setup();
        let conn = db.0.lock().unwrap();
        let created = repo::insert(&conn, &input(ProviderKind::Openai).validate().unwrap())
            .unwrap();
        persist_api_key(&conn, &store, &created.id, Some("sk-original")).unwrap();

        persist_api_key(&conn, &store, &created.id, None).unwrap();
        persist_api_key(&conn, &store, &created.id, Some("   ")).unwrap();

        let entry = api_key_ref(&created.id);
        assert_eq!(store.0.get(&entry).unwrap().as_deref(), Some("sk-original"));
        assert!(repo::get(&conn, &created.id).unwrap().api_key_ref.is_some());
    }
}
