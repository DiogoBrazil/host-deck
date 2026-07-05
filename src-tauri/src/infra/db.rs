use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

/// Migrations aplicadas em ordem; a posição no array + 1 é o `user_version`
/// esperado. Nunca remover ou reordenar entradas — apenas adicionar ao final.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_create_ssh_connections.sql"),
    include_str!("../../migrations/002_create_known_hosts.sql"),
];

pub struct Db(pub Arc<Mutex<Connection>>);

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| AppError::Internal(format!("criando diretório de dados: {e}")))?;
        }
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        run_migrations(&conn)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// Handle clonável para uso fora do `tauri::State` (ex.: tasks SSH).
    pub fn handle(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.0)
    }
}

fn run_migrations(conn: &Connection) -> AppResult<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as u32;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", version)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_and_are_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.0.lock().unwrap();

        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as u32);

        // Rodar de novo não deve falhar (nenhuma migration reaplicada).
        run_migrations(&conn).unwrap();
    }
}
