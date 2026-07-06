use std::sync::Arc;

use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use crate::domain::remote_entry::sort_entries;
use crate::domain::{EntryKind, RemoteEntry};
use crate::error::{AppError, AppResult};
use crate::infra::db::Db;
use crate::infra::sqlite_repository as repo;
use crate::sftp::client::{map_sftp_err, open_sftp};
use crate::sftp::events::{SftpEvent, SftpPrompter};
use crate::sftp::registry::{SftpRegistry, SftpSessionHandle};
use crate::sftp::transfer::{spawn_download, spawn_upload};
use crate::ssh::auth::resolve_auth;
use crate::ssh::client::{AuthSpec, ConnectParams, connect};
use crate::state::CredStore;

use russh_sftp::protocol::FileType;

/// Opens an SFTP session using stored credentials.
#[tauri::command]
pub async fn sftp_connect(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    registry: State<'_, SftpRegistry>,
    session_id: String,
    connection_id: String,
    on_event: Channel<SftpEvent>,
) -> AppResult<String> {
    let conn_data = {
        let conn = db.0.lock().unwrap();
        repo::get(&conn, &connection_id)?
    };

    let auth = resolve_auth(&conn_data, &store.0)?;

    start_sftp_session(
        &db,
        &registry,
        session_id,
        &connection_id,
        ConnectParams {
            host: conn_data.host,
            port: conn_data.port,
            username: conn_data.username,
            auth,
        },
        on_event,
    )
    .await
}

/// Opens an SFTP session with an in-memory password when the keyring is unavailable.
#[tauri::command]
pub async fn sftp_connect_with_password(
    db: State<'_, Db>,
    registry: State<'_, SftpRegistry>,
    session_id: String,
    connection_id: String,
    password: String,
    on_event: Channel<SftpEvent>,
) -> AppResult<String> {
    let conn_data = {
        let conn = db.0.lock().unwrap();
        repo::get(&conn, &connection_id)?
    };

    start_sftp_session(
        &db,
        &registry,
        session_id,
        &connection_id,
        ConnectParams {
            host: conn_data.host,
            port: conn_data.port,
            username: conn_data.username,
            auth: AuthSpec::Password(Zeroizing::new(password)),
        },
        on_event,
    )
    .await
}

/// Resolves a canonical path; `sftp_realpath(session, ".")` yields the home dir.
#[tauri::command]
pub async fn sftp_realpath(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    path: String,
) -> AppResult<String> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    sftp.canonicalize(&path).await.map_err(map_sftp_err)
}

/// Lists a remote directory (sorted, without `.`/`..`).
#[tauri::command]
pub async fn sftp_list_dir(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    path: String,
) -> AppResult<Vec<RemoteEntry>> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    let read_dir = sftp.read_dir(&path).await.map_err(map_sftp_err)?;

    let mut entries = Vec::new();
    for entry in read_dir {
        let meta = entry.metadata();
        let kind = match entry.file_type() {
            FileType::Dir => EntryKind::Dir,
            FileType::Symlink => EntryKind::Symlink,
            _ => EntryKind::File,
        };
        entries.push(RemoteEntry {
            name: entry.file_name(),
            path: entry.path(),
            kind,
            size: meta.size.unwrap_or(0),
            // Server permissions carry the file-type bits; keep only the mode.
            permissions: meta.permissions.map(|p| p & 0o7777),
            modified: meta.mtime.map(|t| t as i64),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

/// Downloads a remote file; progress is streamed over the session channel.
#[tauri::command]
pub async fn sftp_download(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    transfer_id: String,
    remote_path: String,
    local_path: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    let events = registry.events(&session_id).ok_or(AppError::NotFound)?;
    spawn_download(
        (*registry).clone(),
        sftp,
        events,
        transfer_id,
        remote_path,
        local_path,
    );
    Ok(())
}

/// Uploads a local file; progress is streamed over the session channel.
#[tauri::command]
pub async fn sftp_upload(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    transfer_id: String,
    local_path: String,
    remote_path: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    let events = registry.events(&session_id).ok_or(AppError::NotFound)?;
    spawn_upload(
        (*registry).clone(),
        sftp,
        events,
        transfer_id,
        local_path,
        remote_path,
    );
    Ok(())
}

/// Creates a remote directory.
#[tauri::command]
pub async fn sftp_mkdir(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    sftp.create_dir(&path).await.map_err(map_sftp_err)
}

/// Renames or moves a remote entry.
#[tauri::command]
pub async fn sftp_rename(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    from: String,
    to: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    sftp.rename(&from, &to).await.map_err(map_sftp_err)
}

/// Removes a remote file.
#[tauri::command]
pub async fn sftp_remove_file(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    sftp.remove_file(&path).await.map_err(map_sftp_err)
}

/// Removes an empty remote directory.
#[tauri::command]
pub async fn sftp_remove_dir(
    registry: State<'_, SftpRegistry>,
    session_id: String,
    path: String,
) -> AppResult<()> {
    let sftp = registry.session(&session_id).ok_or(AppError::NotFound)?;
    sftp.remove_dir(&path).await.map_err(map_sftp_err)
}

/// Cancels an in-flight transfer.
#[tauri::command]
pub async fn sftp_cancel_transfer(
    registry: State<'_, SftpRegistry>,
    #[allow(unused_variables)] session_id: String,
    transfer_id: String,
) -> AppResult<()> {
    registry.cancel_transfer(&transfer_id);
    Ok(())
}

/// Closes the SFTP session and releases the registry entry.
#[tauri::command]
pub async fn sftp_disconnect(
    registry: State<'_, SftpRegistry>,
    session_id: String,
) -> AppResult<()> {
    if let Some(handle) = registry.remove(&session_id) {
        let _ = handle.sftp.close().await;
        let _ = handle.events.send(SftpEvent::Closed {
            reason: "desconectado pelo usuário".into(),
        });
    }
    Ok(())
}

async fn start_sftp_session(
    db: &Db,
    registry: &SftpRegistry,
    session_id: String,
    connection_id: &str,
    params: ConnectParams,
    on_event: Channel<SftpEvent>,
) -> AppResult<String> {
    // The frontend generates the session id before invoking so it can answer
    // TOFU prompts while this command is still pending.
    if registry.has(&session_id) {
        return Err(AppError::Ssh("Sessão já em uso.".into()));
    }

    let (host_key_tx, host_key_rx) = oneshot::channel::<bool>();
    registry.register_host_key(session_id.clone(), host_key_tx);
    log::info!("[{session_id}] iniciando SFTP para {connection_id}");

    let result = async {
        let prompter = Box::new(SftpPrompter(on_event.clone()));
        let handle = connect(db.handle(), params, prompter, host_key_rx).await?;
        log::info!("[{session_id}] autenticado; abrindo subsistema SFTP");

        {
            let conn = db.0.lock().unwrap();
            repo::touch_last_connected(&conn, connection_id)?;
        }

        let sftp = open_sftp(&handle).await?;
        registry.insert(
            session_id.clone(),
            SftpSessionHandle {
                _ssh: handle,
                sftp: Arc::new(sftp),
                events: on_event.clone(),
            },
        );
        Ok::<(), AppError>(())
    }
    .await;

    match result {
        Ok(()) => {
            let _ = on_event.send(SftpEvent::Connected {
                session_id: session_id.clone(),
            });
            Ok(session_id)
        }
        Err(err) => {
            log::warn!("[{session_id}] falha na conexão SFTP: {err}");
            registry.remove(&session_id);
            Err(err)
        }
    }
}
