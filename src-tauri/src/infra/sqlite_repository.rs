use chrono::Utc;
use rusqlite::{Connection, Row, params};
use uuid::Uuid;

use crate::domain::{AuthMethod, SshConnection, ssh_connection::ValidatedConnection};
use crate::error::{AppError, AppResult};

const COLUMNS: &str = "id, name, host, port, username, auth_method, identity_file, group_name, \
     notes, password_secret_key, key_passphrase_secret_key, last_connected_at, created_at, updated_at";

fn row_to_connection(row: &Row<'_>) -> rusqlite::Result<SshConnection> {
    let auth_raw: String = row.get("auth_method")?;
    let auth_method = AuthMethod::parse(&auth_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            format!("auth_method inválido: {auth_raw}").into(),
        )
    })?;

    Ok(SshConnection {
        id: row.get("id")?,
        name: row.get("name")?,
        host: row.get("host")?,
        port: row.get("port")?,
        username: row.get("username")?,
        auth_method,
        identity_file: row.get("identity_file")?,
        group_name: row.get("group_name")?,
        notes: row.get("notes")?,
        password_secret_key: row.get("password_secret_key")?,
        key_passphrase_secret_key: row.get("key_passphrase_secret_key")?,
        last_connected_at: row.get("last_connected_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<SshConnection>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM ssh_connections ORDER BY group_name, name COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([], row_to_connection)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: &str) -> AppResult<SshConnection> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM ssh_connections WHERE id = ?1"))?;
    stmt.query_row(params![id], row_to_connection)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            other => other.into(),
        })
}

pub fn insert(conn: &Connection, v: &ValidatedConnection) -> AppResult<SshConnection> {
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    conn.execute(
        "INSERT INTO ssh_connections \
         (id, name, host, port, username, auth_method, identity_file, group_name, notes, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![
            id,
            v.name,
            v.host,
            v.port,
            v.username,
            v.auth_method.as_str(),
            v.identity_file,
            v.group_name,
            v.notes,
            now,
        ],
    )?;

    get(conn, &id)
}

pub fn update(conn: &Connection, id: &str, v: &ValidatedConnection) -> AppResult<SshConnection> {
    let now = Utc::now().to_rfc3339();

    let changed = conn.execute(
        "UPDATE ssh_connections SET \
         name = ?2, host = ?3, port = ?4, username = ?5, auth_method = ?6, \
         identity_file = ?7, group_name = ?8, notes = ?9, updated_at = ?10 \
         WHERE id = ?1",
        params![
            id,
            v.name,
            v.host,
            v.port,
            v.username,
            v.auth_method.as_str(),
            v.identity_file,
            v.group_name,
            v.notes,
            now,
        ],
    )?;

    if changed == 0 {
        return Err(AppError::NotFound);
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: &str) -> AppResult<()> {
    let changed = conn.execute("DELETE FROM ssh_connections WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

pub fn set_secret_refs(
    conn: &Connection,
    id: &str,
    password_secret_key: Option<&str>,
    key_passphrase_secret_key: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "UPDATE ssh_connections SET password_secret_key = ?2, key_passphrase_secret_key = ?3 WHERE id = ?1",
        params![id, password_secret_key, key_passphrase_secret_key],
    )?;
    Ok(())
}

pub fn touch_last_connected(conn: &Connection, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE ssh_connections SET last_connected_at = ?2 WHERE id = ?1",
        params![id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::Db;

    fn sample(name: &str) -> ValidatedConnection {
        ValidatedConnection {
            name: name.into(),
            host: "10.0.0.1".into(),
            port: 22,
            username: "root".into(),
            auth_method: AuthMethod::Password,
            identity_file: None,
            group_name: "Geral".into(),
            notes: None,
        }
    }

    #[test]
    fn crud_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();

        let created = insert(&conn, &sample("VPS Teste")).unwrap();
        assert_eq!(created.name, "VPS Teste");
        assert!(created.password_secret_key.is_none());

        let fetched = get(&conn, &created.id).unwrap();
        assert_eq!(fetched.id, created.id);

        let mut edited = sample("VPS Editada");
        edited.port = 2222;
        let updated = update(&conn, &created.id, &edited).unwrap();
        assert_eq!(updated.name, "VPS Editada");
        assert_eq!(updated.port, 2222);
        assert_eq!(updated.created_at, created.created_at);

        assert_eq!(list(&conn).unwrap().len(), 1);

        delete(&conn, &created.id).unwrap();
        assert!(matches!(get(&conn, &created.id), Err(AppError::NotFound)));
    }

    #[test]
    fn secret_refs_and_last_connected() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();
        let created = insert(&conn, &sample("VPS")).unwrap();

        set_secret_refs(&conn, &created.id, Some("ssh-password:abc"), None).unwrap();
        touch_last_connected(&conn, &created.id).unwrap();

        let fetched = get(&conn, &created.id).unwrap();
        assert_eq!(fetched.password_secret_key.as_deref(), Some("ssh-password:abc"));
        assert!(fetched.last_connected_at.is_some());
    }

    #[test]
    fn update_missing_returns_not_found() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();
        assert!(matches!(
            update(&conn, "nao-existe", &sample("X")),
            Err(AppError::NotFound)
        ));
        assert!(matches!(delete(&conn, "nao-existe"), Err(AppError::NotFound)));
    }
}
