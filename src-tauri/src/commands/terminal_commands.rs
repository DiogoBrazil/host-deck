use std::sync::{Arc, Mutex};

use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::infra::db::Db;
use crate::infra::sqlite_repository as repo;
use crate::ssh::auth::resolve_auth;
use crate::ssh::client::{AuthSpec, ConnectParams, TerminalPrompter, connect};
use crate::ssh::events::TerminalEvent;
use crate::ssh::registry::{SessionHandle, SessionInput, SessionRegistry};
use crate::ssh::scrollback::Scrollback;
use crate::ssh::session::open_shell_and_bridge;
use crate::sftp::registry::SftpRegistry;
use crate::state::CredStore;

/// Opens an interactive SSH shell using stored credentials.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_connect(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    registry: State<'_, SessionRegistry>,
    session_id: String,
    connection_id: String,
    cols: u32,
    rows: u32,
    on_event: Channel<TerminalEvent>,
) -> AppResult<String> {
    let conn_data = {
        let conn = db.0.lock().unwrap();
        repo::get(&conn, &connection_id)?
    };

    let auth = resolve_auth(&conn_data, &store.0)?;

    start_session(
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
        cols,
        rows,
        on_event,
    )
    .await
}

/// Opens an SSH shell with an in-memory password when the keyring is unavailable.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ssh_connect_with_password(
    db: State<'_, Db>,
    registry: State<'_, SessionRegistry>,
    session_id: String,
    connection_id: String,
    password: String,
    cols: u32,
    rows: u32,
    on_event: Channel<TerminalEvent>,
) -> AppResult<String> {
    let conn_data = {
        let conn = db.0.lock().unwrap();
        repo::get(&conn, &connection_id)?
    };

    start_session(
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
        cols,
        rows,
        on_event,
    )
    .await
}

#[tauri::command]
pub async fn ssh_send_data(
    registry: State<'_, SessionRegistry>,
    session_id: String,
    data: String,
) -> AppResult<()> {
    let sender = registry
        .input_sender(&session_id)
        .ok_or(AppError::NotFound)?;
    sender
        .send(SessionInput::Data(data.into_bytes()))
        .await
        .map_err(|_| AppError::Ssh("Sessão não está mais ativa.".into()))
}

#[tauri::command]
pub async fn ssh_resize(
    registry: State<'_, SessionRegistry>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> AppResult<()> {
    let sender = registry
        .input_sender(&session_id)
        .ok_or(AppError::NotFound)?;
    sender
        .send(SessionInput::Resize { cols, rows })
        .await
        .map_err(|_| AppError::Ssh("Sessão não está mais ativa.".into()))
}

#[tauri::command]
pub async fn ssh_disconnect(
    registry: State<'_, SessionRegistry>,
    session_id: String,
) -> AppResult<()> {
    if let Some(sender) = registry.input_sender(&session_id) {
        let _ = sender.send(SessionInput::Close).await;
    }
    Ok(())
}

/// Sends the user's TOFU host-key decision to the pending session.
///
/// Shared by terminal and SFTP sessions: `known_hosts` is common to both, so a
/// single command routes the decision to whichever registry holds the pending
/// confirmation for this `session_id`.
#[tauri::command]
pub async fn confirm_host_key(
    registry: State<'_, SessionRegistry>,
    sftp_registry: State<'_, SftpRegistry>,
    session_id: String,
    accept: bool,
) -> AppResult<()> {
    if let Some(tx) = registry.take_host_key_tx(&session_id) {
        let _ = tx.send(accept);
    }
    if let Some(tx) = sftp_registry.take_host_key_tx(&session_id) {
        let _ = tx.send(accept);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn start_session(
    db: &Db,
    registry: &SessionRegistry,
    session_id: String,
    connection_id: &str,
    params: ConnectParams,
    cols: u32,
    rows: u32,
    on_event: Channel<TerminalEvent>,
) -> AppResult<String> {
    // The frontend generates the session id before invoking ssh_connect so it
    // can answer TOFU prompts while this command is still pending.
    if registry.input_sender(&session_id).is_some() {
        return Err(AppError::Ssh("Sessão já em uso.".into()));
    }

    let (input_tx, input_rx) = mpsc::channel::<SessionInput>(64);
    let (host_key_tx, host_key_rx) = oneshot::channel::<bool>();
    let scrollback = Arc::new(Mutex::new(Scrollback::default()));

    // Register before connecting; TOFU prompts must be routed to this session.
    registry.insert(
        session_id.clone(),
        SessionHandle {
            input_tx,
            host_key_tx: Some(host_key_tx),
            ssh: None,
            scrollback: scrollback.clone(),
        },
    );
    log::info!("[{session_id}] iniciando conexão para {connection_id}");

    let result = async {
        let prompter = Box::new(TerminalPrompter(on_event.clone()));
        let handle = Arc::new(connect(db.handle(), params, prompter, host_key_rx).await?);
        log::info!("[{session_id}] autenticado; abrindo shell");

        // Retained so the agent can open `exec` channels on this connection.
        registry.set_ssh_handle(&session_id, handle.clone());

        {
            let conn = db.0.lock().unwrap();
            repo::touch_last_connected(&conn, connection_id)?;
        }

        let sid = session_id.clone();
        let registry_for_cleanup = registry.clone();
        open_shell_and_bridge(
            &handle,
            cols,
            rows,
            on_event.clone(),
            scrollback,
            input_rx,
            move || {
                registry_for_cleanup.remove(&sid);
            },
        )
        .await?;

        Ok::<(), AppError>(())
    }
    .await;

    match result {
        Ok(()) => {
            let _ = on_event.send(TerminalEvent::Connected {
                session_id: session_id.clone(),
            });
            Ok(session_id)
        }
        Err(err) => {
            log::warn!("[{session_id}] falha na conexão: {}", err);
            registry.remove(&session_id);
            Err(err)
        }
    }
}
