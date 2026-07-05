use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use russh::keys::HashAlg;
use russh::keys::ssh_key::PublicKey;
use uuid::Uuid;

use crate::error::AppResult;

/// Resultado da verificação TOFU da chave do servidor.
pub enum Verdict {
    /// Chave já conhecida e idêntica.
    Known,
    /// Host nunca visto: pedir confirmação ao usuário.
    Unknown,
    /// Chave DIFERENTE da registrada: possível man-in-the-middle.
    Mismatch { stored_fingerprint: String },
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

pub fn key_type(key: &PublicKey) -> String {
    key.algorithm().to_string()
}

pub fn verify(
    db: &Arc<Mutex<Connection>>,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> AppResult<Verdict> {
    let conn = db.lock().unwrap();
    let stored: Option<String> = conn
        .query_row(
            "SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2 AND key_type = ?3",
            params![host, port, key_type(key)],
            |row| row.get(0),
        )
        .optional()?;

    Ok(match stored {
        None => Verdict::Unknown,
        Some(fp) if fp == fingerprint(key) => Verdict::Known,
        Some(fp) => Verdict::Mismatch {
            stored_fingerprint: fp,
        },
    })
}

pub fn save(
    db: &Arc<Mutex<Connection>>,
    host: &str,
    port: u16,
    key: &PublicKey,
) -> AppResult<()> {
    let conn = db.lock().unwrap();
    let openssh = key
        .to_openssh()
        .map(|s| s.to_string())
        .unwrap_or_default();

    conn.execute(
        "INSERT INTO known_hosts (id, host, port, key_type, public_key, fingerprint, added_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(host, port, key_type) DO UPDATE SET \
           public_key = excluded.public_key, \
           fingerprint = excluded.fingerprint, \
           added_at = excluded.added_at",
        params![
            Uuid::new_v4().to_string(),
            host,
            port,
            key_type(key),
            openssh,
            fingerprint(key),
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}
