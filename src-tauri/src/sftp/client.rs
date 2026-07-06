use russh::client::Handle;
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;

use crate::error::{AppError, AppResult};
use crate::ssh::client::TofuHandler;

/// Opens the SFTP subsystem over an already authenticated SSH connection.
///
/// Reuses `ssh::client::connect` upstream and only swaps the final step: instead
/// of a shell+PTY, it opens the `sftp` subsystem on a fresh channel.
pub async fn open_sftp(handle: &Handle<TofuHandler>) -> AppResult<SftpSession> {
    let channel = handle.channel_open_session().await.map_err(AppError::from)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(AppError::from)?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| AppError::Ssh(format!("Falha ao iniciar sessão SFTP: {e}.")))?;
    Ok(sftp)
}

/// Maps an SFTP protocol error to a user-facing `AppError` without leaking secrets.
pub fn map_sftp_err(e: SftpError) -> AppError {
    let msg = match e {
        SftpError::Status(status) => {
            let detail = status.error_message.trim();
            if detail.is_empty() {
                format!("Operação SFTP recusada ({}).", status.status_code)
            } else {
                format!("Operação SFTP recusada: {detail}.")
            }
        }
        SftpError::Timeout => "Tempo de resposta do SFTP esgotado.".to_string(),
        other => format!("Erro no SFTP: {other}."),
    };
    AppError::Ssh(msg)
}
