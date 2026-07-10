//! Flags de aplicação em `app_settings` (chave/valor).
//!
//! Segredos nunca entram aqui — continuam no keyring. Isto guarda apenas
//! estado de UX que precisa sobreviver ao restart, como o consentimento de
//! envio do terminal ao provedor de IA.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;

pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    let value = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::Db;

    #[test]
    fn set_creates_and_overwrites() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();

        assert_eq!(get(&conn, "consent").unwrap(), None);

        set(&conn, "consent", "granted").unwrap();
        assert_eq!(get(&conn, "consent").unwrap().as_deref(), Some("granted"));

        set(&conn, "consent", "revoked").unwrap();
        assert_eq!(get(&conn, "consent").unwrap().as_deref(), Some("revoked"));
    }
}
