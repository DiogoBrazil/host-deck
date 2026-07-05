use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

use crate::domain::AuthMethod;
use crate::error::{AppError, AppResult};
use crate::infra::credential_store::password_ref;
use crate::infra::db::Db;
use crate::infra::sqlite_repository as repo;
use crate::ssh::client::{AuthSpec, ConnectParams, connect};
use crate::ssh::events::TerminalEvent;
use crate::ssh::registry::{SessionHandle, SessionInput, SessionRegistry};
use crate::ssh::session::open_shell_and_bridge;
use crate::state::CredStore;

/// Conecta usando as credenciais salvas e abre o shell interativo.
/// Retorna o `session_id` usado pelos demais commands da sessão.
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

    let auth = match conn_data.auth_method {
        AuthMethod::Password => {
            let secret_ref = conn_data
                .password_secret_key
                .clone()
                .unwrap_or_else(|| password_ref(&connection_id));
            let password = store.0.get(&secret_ref)?.ok_or_else(|| {
                AppError::Ssh(
                    "Senha não encontrada no armazenamento seguro. \
                     Edite a conexão e cadastre a senha novamente."
                        .into(),
                )
            })?;
            AuthSpec::Password(Zeroizing::new(password))
        }
        AuthMethod::PrivateKey => {
            let path = conn_data.identity_file.clone().ok_or_else(|| {
                AppError::Ssh("Conexão sem caminho de chave privada cadastrado.".into())
            })?;
            let passphrase = match &conn_data.key_passphrase_secret_key {
                Some(secret_ref) => store.0.get(secret_ref)?.map(Zeroizing::new),
                None => None,
            };
            AuthSpec::PrivateKey { path, passphrase }
        }
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
            auth,
        },
        cols,
        rows,
        on_event,
    )
    .await
}

/// Fallback quando o keyring está indisponível: conecta com senha informada
/// na hora, mantida apenas em memória.
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

/// Resposta do usuário ao prompt de fingerprint (TOFU).
#[tauri::command]
pub async fn confirm_host_key(
    registry: State<'_, SessionRegistry>,
    session_id: String,
    accept: bool,
) -> AppResult<()> {
    if let Some(tx) = registry.take_host_key_tx(&session_id) {
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
    // session_id é gerado pelo frontend para que a UI possa confirmar o host
    // key enquanto esta chamada ainda está pendente. Se colidir com uma sessão
    // ativa, recusa (não deveria acontecer com UUID v4).
    if registry.input_sender(&session_id).is_some() {
        return Err(AppError::Ssh("Sessão já em uso.".into()));
    }

    let (input_tx, input_rx) = mpsc::channel::<SessionInput>(64);
    let (host_key_tx, host_key_rx) = oneshot::channel::<bool>();

    // Registra antes do connect: o prompt TOFU precisa achar a sessão.
    registry.insert(
        session_id.clone(),
        SessionHandle {
            input_tx,
            host_key_tx: Some(host_key_tx),
        },
    );
    log::info!("[{session_id}] iniciando conexão para {connection_id}");

    let result = async {
        let handle = connect(db.handle(), params, on_event.clone(), host_key_rx).await?;
        log::info!("[{session_id}] autenticado; abrindo shell");

        {
            let conn = db.0.lock().unwrap();
            repo::touch_last_connected(&conn, connection_id)?;
        }

        let sid = session_id.clone();
        let registry_for_cleanup = registry.clone();
        open_shell_and_bridge(handle, cols, rows, on_event.clone(), input_rx, move || {
            registry_for_cleanup.remove(&sid);
        })
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
