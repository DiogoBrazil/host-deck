use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use russh::client::{self, AuthResult, Handle};
use russh::keys::ssh_key::PublicKey;
use russh::keys::{PrivateKeyWithHashAlg, decode_secret_key};
use tauri::ipc::Channel;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::ssh::events::TerminalEvent;
use crate::ssh::host_key::{self, Verdict};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum time allowed for the first-connection host-key prompt.
const HOST_KEY_CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);

pub enum AuthSpec {
    Password(Zeroizing<String>),
    PrivateKey {
        path: String,
        passphrase: Option<Zeroizing<String>>,
    },
}

pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthSpec,
}

/// russh handler that enforces TOFU host-key verification.
pub struct TofuHandler {
    db: Arc<Mutex<Connection>>,
    host: String,
    port: u16,
    events: Channel<TerminalEvent>,
    /// Receives the user's decision from `confirm_host_key`.
    confirm_rx: Option<oneshot::Receiver<bool>>,
}

impl client::Handler for TofuHandler {
    type Error = AppError;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        match host_key::verify(&self.db, &self.host, self.port, key)? {
            Verdict::Known => {
                log::info!("host {}:{} já conhecido", self.host, self.port);
                Ok(true)
            }
            Verdict::Mismatch { stored_fingerprint } => Err(AppError::Ssh(format!(
                "ALERTA DE SEGURANÇA: a chave do servidor {}:{} mudou. \
                 Registrada: {stored_fingerprint}. Recebida: {}. \
                 Isso pode indicar um ataque man-in-the-middle. \
                 Se o servidor foi reinstalado legitimamente, remova o host conhecido e conecte novamente.",
                self.host,
                self.port,
                host_key::fingerprint(key),
            ))),
            Verdict::Unknown => {
                let Some(confirm_rx) = self.confirm_rx.take() else {
                    return Ok(false);
                };
                log::info!(
                    "host {}:{} desconhecido; aguardando confirmação do fingerprint",
                    self.host,
                    self.port
                );
                self.events
                    .send(TerminalEvent::HostKeyPrompt {
                        fingerprint: host_key::fingerprint(key),
                        key_type: host_key::key_type(key),
                    })
                    .map_err(|e| AppError::Internal(format!("enviando evento: {e}")))?;

                let accepted =
                    match tokio::time::timeout(HOST_KEY_CONFIRM_TIMEOUT, confirm_rx).await {
                        Ok(Ok(accepted)) => accepted,
                        _ => false,
                    };
                log::info!("confirmação do host key: aceito={accepted}");

                if accepted {
                    host_key::save(&self.db, &self.host, self.port, key)?;
                }
                Ok(accepted)
            }
        }
    }
}

impl From<russh::Error> for AppError {
    fn from(e: russh::Error) -> Self {
        friendly_ssh_error(&e)
    }
}

fn friendly_ssh_error(e: &russh::Error) -> AppError {
    let msg = match e {
        russh::Error::NotAuthenticated => {
            "Autenticação recusada pelo servidor. Verifique usuário e credenciais.".to_string()
        }
        russh::Error::UnknownKey => {
            "Chave do servidor recusada (fingerprint não confirmado).".to_string()
        }
        russh::Error::ConnectionTimeout => "Tempo de conexão esgotado.".to_string(),
        russh::Error::Disconnect => "O servidor encerrou a conexão.".to_string(),
        other => format!("Falha na conexão SSH: {other}"),
    };
    AppError::Ssh(msg)
}

/// Connects, verifies the host key, authenticates, and returns the session handle.
pub async fn connect(
    db: Arc<Mutex<Connection>>,
    params: ConnectParams,
    events: Channel<TerminalEvent>,
    confirm_rx: oneshot::Receiver<bool>,
) -> AppResult<Handle<TofuHandler>> {
    let addr = format!("{}:{}", params.host, params.port);
    log::info!("conectando TCP em {addr}");
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| AppError::Ssh(format!("Tempo esgotado ao conectar em {addr}.")))?
        .map_err(|e| AppError::Ssh(format!("Não foi possível conectar em {addr}: {e}.")))?;

    let config = Arc::new(client::Config::default());
    let handler = TofuHandler {
        db,
        host: params.host.clone(),
        port: params.port,
        events,
        confirm_rx: Some(confirm_rx),
    };

    // The SSH handshake can block on TOFU confirmation, so this timeout includes
    // the full prompt window plus the network connection allowance.
    log::info!("iniciando handshake SSH com {addr}");
    let mut handle = tokio::time::timeout(
        HOST_KEY_CONFIRM_TIMEOUT + CONNECT_TIMEOUT,
        client::connect_stream(config, stream, handler),
    )
    .await
    .map_err(|_| AppError::Ssh("Tempo esgotado no handshake SSH.".into()))??;
    log::info!("handshake concluído com {addr}; autenticando usuário {}", params.username);

    let auth_result = match params.auth {
        AuthSpec::Password(password) => {
            handle
                .authenticate_password(params.username.clone(), password.to_string())
                .await?
        }
        AuthSpec::PrivateKey { path, passphrase } => {
            let pem = Zeroizing::new(std::fs::read_to_string(&path).map_err(|e| {
                AppError::Ssh(format!("Não foi possível ler a chave privada {path}: {e}."))
            })?);
            let key = decode_secret_key(&pem, passphrase.as_deref().map(|p| p as &str))
                .map_err(|e| {
                    AppError::Ssh(format!(
                        "Não foi possível decodificar a chave privada: {e}. \
                         Se a chave tem passphrase, confira se ela foi informada corretamente."
                    ))
                })?;
            let best_hash = handle.best_supported_rsa_hash().await?.flatten();
            handle
                .authenticate_publickey(
                    params.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), best_hash),
                )
                .await?
        }
    };

    match auth_result {
        AuthResult::Success => Ok(handle),
        AuthResult::Failure { .. } => Err(AppError::Ssh(
            "Autenticação recusada pelo servidor. Verifique usuário e credenciais.".into(),
        )),
    }
}
