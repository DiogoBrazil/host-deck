use tauri::State;

use crate::domain::{AuthMethod, ConnectionInput, SshConnection};
use crate::error::AppResult;
use crate::infra::credential_store::{passphrase_ref, password_ref};
use crate::infra::db::Db;
use crate::infra::sqlite_repository as repo;
use crate::state::CredStore;

#[tauri::command]
pub async fn list_connections(db: State<'_, Db>) -> AppResult<Vec<SshConnection>> {
    let conn = db.0.lock().unwrap();
    repo::list(&conn)
}

#[tauri::command]
pub async fn get_connection(db: State<'_, Db>, id: String) -> AppResult<SshConnection> {
    let conn = db.0.lock().unwrap();
    repo::get(&conn, &id)
}

#[tauri::command]
pub async fn create_connection(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    input: ConnectionInput,
) -> AppResult<SshConnection> {
    let validated = input.validate(false)?;
    let conn = db.0.lock().unwrap();
    let created = repo::insert(&conn, &validated)?;

    match persist_secrets(&conn, &store, &created.id, &input) {
        Ok(()) => repo::get(&conn, &created.id),
        Err(err) => {
            // Roll back the row if credential persistence fails; otherwise the
            // connection would be saved without usable authentication material.
            let _ = repo::delete(&conn, &created.id);
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn update_connection(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    id: String,
    input: ConnectionInput,
) -> AppResult<SshConnection> {
    let validated = input.validate(true)?;
    let conn = db.0.lock().unwrap();

    let previous = repo::get(&conn, &id)?;
    repo::update(&conn, &id, &validated)?;

    if previous.auth_method != validated.auth_method {
        match validated.auth_method {
            AuthMethod::PrivateKey => store.0.delete(&password_ref(&id))?,
            AuthMethod::Password => store.0.delete(&passphrase_ref(&id))?,
        }
    }

    persist_secrets(&conn, &store, &id, &input)?;
    repo::get(&conn, &id)
}

#[tauri::command]
pub async fn delete_connection(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    id: String,
) -> AppResult<()> {
    store.0.delete(&password_ref(&id))?;
    store.0.delete(&passphrase_ref(&id))?;

    let conn = db.0.lock().unwrap();
    repo::delete(&conn, &id)
}

/// Stores secrets in the system keyring and writes only their references to SQLite.
fn persist_secrets(
    conn: &rusqlite::Connection,
    store: &CredStore,
    id: &str,
    input: &ConnectionInput,
) -> AppResult<()> {
    let current = repo::get(conn, id)?;
    let mut pwd_ref = current.password_secret_key.clone();
    let mut phrase_ref = current.key_passphrase_secret_key.clone();

    match input.auth_method {
        AuthMethod::Password => {
            if let Some(password) = input.password.as_deref().filter(|p| !p.is_empty()) {
                let entry = password_ref(id);
                store.0.set(&entry, password)?;
                pwd_ref = Some(entry);
            }
            phrase_ref = None;
        }
        AuthMethod::PrivateKey => {
            if input.save_passphrase {
                if let Some(phrase) = input.passphrase.as_deref().filter(|p| !p.is_empty()) {
                    let entry = passphrase_ref(id);
                    store.0.set(&entry, phrase)?;
                    phrase_ref = Some(entry);
                }
            }
            pwd_ref = None;
        }
    }

    repo::set_secret_refs(conn, id, pwd_ref.as_deref(), phrase_ref.as_deref())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::infra::credential_store::mock::MockStore;
    use crate::infra::db::Db;

    fn password_input(password: Option<&str>) -> ConnectionInput {
        ConnectionInput {
            name: String::new(),
            host: "10.0.0.1".into(),
            port: None,
            username: "root".into(),
            auth_method: AuthMethod::Password,
            identity_file: None,
            group_name: String::new(),
            notes: None,
            password: password.map(String::from),
            passphrase: None,
            save_passphrase: false,
        }
    }

    fn setup() -> (Db, CredStore) {
        (
            Db::open_in_memory().unwrap(),
            CredStore(Arc::new(MockStore::default())),
        )
    }

    #[test]
    fn password_goes_to_store_and_db_keeps_only_the_ref() {
        let (db, store) = setup();
        let conn = db.0.lock().unwrap();
        let input = password_input(Some("s3nh4"));
        let created = repo::insert(&conn, &input.validate(false).unwrap()).unwrap();

        persist_secrets(&conn, &store, &created.id, &input).unwrap();

        let expected_ref = password_ref(&created.id);
        let fetched = repo::get(&conn, &created.id).unwrap();
        assert_eq!(fetched.password_secret_key.as_deref(), Some(expected_ref.as_str()));
        assert_eq!(
            store.0.get(&expected_ref).unwrap().as_deref(),
            Some("s3nh4")
        );

        let dump: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT COALESCE(password_secret_key,'') || COALESCE(notes,'') || name || host FROM ssh_connections")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(Result::unwrap).collect()
        };
        assert!(dump.iter().all(|s| !s.contains("s3nh4")));
    }

    #[test]
    fn passphrase_saved_only_when_opted_in() {
        let (db, store) = setup();
        let conn = db.0.lock().unwrap();

        let key_file = std::env::temp_dir().join("hostdeck-test-key");
        std::fs::write(&key_file, "fake").unwrap();

        let mut input = password_input(None);
        input.auth_method = AuthMethod::PrivateKey;
        input.identity_file = Some(key_file.to_string_lossy().into_owned());
        input.passphrase = Some("frase".into());
        input.save_passphrase = false;

        let created = repo::insert(&conn, &input.validate(false).unwrap()).unwrap();
        persist_secrets(&conn, &store, &created.id, &input).unwrap();
        let fetched = repo::get(&conn, &created.id).unwrap();
        assert!(fetched.key_passphrase_secret_key.is_none());
        assert!(store.0.get(&passphrase_ref(&created.id)).unwrap().is_none());

        input.save_passphrase = true;
        persist_secrets(&conn, &store, &created.id, &input).unwrap();
        let fetched = repo::get(&conn, &created.id).unwrap();
        assert!(fetched.key_passphrase_secret_key.is_some());
        assert_eq!(
            store.0.get(&passphrase_ref(&created.id)).unwrap().as_deref(),
            Some("frase")
        );

        std::fs::remove_file(&key_file).ok();
    }

    #[test]
    fn switching_auth_method_clears_stale_refs() {
        let (db, store) = setup();
        let conn = db.0.lock().unwrap();
        let input = password_input(Some("s3nh4"));
        let created = repo::insert(&conn, &input.validate(false).unwrap()).unwrap();
        persist_secrets(&conn, &store, &created.id, &input).unwrap();

        let key_file = std::env::temp_dir().join("hostdeck-test-key2");
        std::fs::write(&key_file, "fake").unwrap();

        let mut switched = password_input(None);
        switched.auth_method = AuthMethod::PrivateKey;
        switched.identity_file = Some(key_file.to_string_lossy().into_owned());
        persist_secrets(&conn, &store, &created.id, &switched).unwrap();

        let fetched = repo::get(&conn, &created.id).unwrap();
        assert!(fetched.password_secret_key.is_none());

        std::fs::remove_file(&key_file).ok();
    }
}
