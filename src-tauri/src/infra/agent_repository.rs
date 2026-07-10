use chrono::Utc;
use rusqlite::{Connection, Row, params};
use uuid::Uuid;

use crate::domain::{AgentProvider, ModelCacheEntry, ProviderKind};
use crate::domain::agent_provider::ValidatedProvider;
use crate::error::{AppError, AppResult};

const PROVIDER_COLUMNS: &str = "id, kind, label, base_url, model, api_key_ref, created_at";

fn row_to_provider(row: &Row<'_>) -> rusqlite::Result<AgentProvider> {
    let kind_raw: String = row.get("kind")?;
    let kind = ProviderKind::parse(&kind_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("kind inválido: {kind_raw}").into(),
        )
    })?;

    Ok(AgentProvider {
        id: row.get("id")?,
        kind,
        label: row.get("label")?,
        base_url: row.get("base_url")?,
        model: row.get("model")?,
        api_key_ref: row.get("api_key_ref")?,
        created_at: row.get("created_at")?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<AgentProvider>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM agent_providers ORDER BY label COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([], row_to_provider)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<AgentProvider> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROVIDER_COLUMNS} FROM agent_providers WHERE id = ?1"
    ))?;
    stmt.query_row(params![id], row_to_provider)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            other => other.into(),
        })
}

pub fn insert(conn: &Connection, v: &ValidatedProvider) -> AppResult<AgentProvider> {
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    conn.execute(
        "INSERT INTO agent_providers (id, kind, label, base_url, model, api_key_ref, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
        params![id, v.kind.as_str(), v.label, v.base_url, v.model, now],
    )?;

    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, v: &ValidatedProvider) -> AppResult<AgentProvider> {
    let changed = conn.execute(
        "UPDATE agent_providers SET kind = ?2, label = ?3, base_url = ?4, model = ?5 \
         WHERE id = ?1",
        params![id, v.kind.as_str(), v.label, v.base_url, v.model],
    )?;

    if changed == 0 {
        return Err(AppError::NotFound);
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM agent_providers WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Records the keyring reference after the API key is stored, mirroring
/// `sqlite_repository::set_secret_refs`.
pub fn set_api_key_ref(conn: &Connection, id: &str, api_key_ref: Option<&str>) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE agent_providers SET api_key_ref = ?2 WHERE id = ?1",
        params![id, api_key_ref],
    )?;
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// Replaces the cached model list of a provider with a fresh fetch.
pub fn replace_model_cache(
    conn: &mut Connection,
    provider_id: &str,
    models: &[ModelCacheEntry],
) -> AppResult<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM agent_model_cache WHERE provider_id = ?1",
        params![provider_id],
    )?;
    for m in models {
        tx.execute(
            "INSERT INTO agent_model_cache \
             (provider_id, model_id, display_name, max_input_tokens, max_output_tokens, capabilities, fetched_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                provider_id,
                m.model_id,
                m.display_name,
                m.max_input_tokens,
                m.max_output_tokens,
                m.capabilities,
                m.fetched_at,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn list_model_cache(conn: &Connection, provider_id: &str) -> AppResult<Vec<ModelCacheEntry>> {
    let mut stmt = conn.prepare(
        "SELECT provider_id, model_id, display_name, max_input_tokens, max_output_tokens, \
         capabilities, fetched_at \
         FROM agent_model_cache WHERE provider_id = ?1 ORDER BY model_id",
    )?;
    let rows = stmt.query_map(params![provider_id], |row| {
        Ok(ModelCacheEntry {
            provider_id: row.get("provider_id")?,
            model_id: row.get("model_id")?,
            display_name: row.get("display_name")?,
            max_input_tokens: row.get("max_input_tokens")?,
            max_output_tokens: row.get("max_output_tokens")?,
            capabilities: row.get("capabilities")?,
            fetched_at: row.get("fetched_at")?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::Db;

    fn sample(label: &str) -> ValidatedProvider {
        ValidatedProvider {
            kind: ProviderKind::Anthropic,
            label: label.into(),
            base_url: None,
            model: Some("claude-sonnet-5".into()),
        }
    }

    fn model(provider_id: &str, model_id: &str) -> ModelCacheEntry {
        ModelCacheEntry {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            display_name: Some(model_id.to_uppercase()),
            max_input_tokens: Some(200_000),
            max_output_tokens: Some(64_000),
            capabilities: r#"{"thinking":true}"#.into(),
            fetched_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn crud_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();

        let created = insert(&conn, &sample("Anthropic")).unwrap();
        assert_eq!(created.kind, ProviderKind::Anthropic);
        assert!(created.api_key_ref.is_none());

        let mut edited = sample("Gateway");
        edited.base_url = Some("https://gw.interno".into());
        let updated = update(&conn, &created.id, &edited).unwrap();
        assert_eq!(updated.label, "Gateway");
        assert_eq!(updated.base_url.as_deref(), Some("https://gw.interno"));
        assert_eq!(updated.created_at, created.created_at);

        assert_eq!(list(&conn).unwrap().len(), 1);

        delete(&conn, &created.id).unwrap();
        assert!(matches!(get(&conn, &created.id), Err(AppError::NotFound)));
    }

    #[test]
    fn api_key_ref_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();
        let created = insert(&conn, &sample("Anthropic")).unwrap();

        let key_ref = crate::infra::credential_store::api_key_ref(&created.id);
        set_api_key_ref(&conn, &created.id, Some(&key_ref)).unwrap();
        assert_eq!(get(&conn, &created.id).unwrap().api_key_ref, Some(key_ref));

        set_api_key_ref(&conn, &created.id, None).unwrap();
        assert!(get(&conn, &created.id).unwrap().api_key_ref.is_none());

        assert!(matches!(
            set_api_key_ref(&conn, "nao-existe", None),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn accepts_openrouter_kind_after_migration_006() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();

        let mut input = sample("OpenRouter");
        input.kind = ProviderKind::Openrouter;
        let created = insert(&conn, &input).unwrap();
        assert_eq!(created.kind, ProviderKind::Openrouter);

        // The table rebuild in migration 006 must leave foreign_keys enabled.
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn rejects_invalid_kind_via_check_constraint() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();
        let result = conn.execute(
            "INSERT INTO agent_providers (id, kind, label, created_at) \
             VALUES ('x', 'gemini', 'X', '2026-01-01')",
            [],
        );
        assert!(result.is_err(), "CHECK deve rejeitar kind desconhecido");
    }

    #[test]
    fn model_cache_replace_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let mut conn = db.0.lock().unwrap();
        let provider = insert(&conn, &sample("Anthropic")).unwrap();

        replace_model_cache(
            &mut conn,
            &provider.id,
            &[
                model(&provider.id, "claude-sonnet-5"),
                model(&provider.id, "claude-opus-4-8"),
            ],
        )
        .unwrap();
        assert_eq!(list_model_cache(&conn, &provider.id).unwrap().len(), 2);

        // A fresh fetch replaces the previous entries.
        replace_model_cache(&mut conn, &provider.id, &[model(&provider.id, "claude-haiku-4-5")])
            .unwrap();
        let cached = list_model_cache(&conn, &provider.id).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].model_id, "claude-haiku-4-5");
        assert_eq!(cached[0].capabilities, r#"{"thinking":true}"#);

        // Deleting the provider clears its cache (ON DELETE CASCADE).
        delete(&conn, &provider.id).unwrap();
        assert!(list_model_cache(&conn, &provider.id).unwrap().is_empty());
    }
}
