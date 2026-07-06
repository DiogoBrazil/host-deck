use std::sync::Arc;
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::sftp::client::map_sftp_err;
use crate::sftp::events::SftpEvent;
use crate::sftp::registry::SftpRegistry;

/// Transfer chunk size. Small enough to report smooth progress, large enough to
/// keep syscall/packet overhead low.
const CHUNK: usize = 32 * 1024;
/// Minimum spacing between `Progress` events, to avoid flooding the IPC bridge.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Spawns a background download; the command returns immediately and progress
/// arrives over the session event channel.
pub fn spawn_download(
    registry: SftpRegistry,
    sftp: Arc<SftpSession>,
    events: Channel<SftpEvent>,
    transfer_id: String,
    remote_path: String,
    local_path: String,
) {
    let token = CancellationToken::new();
    registry.register_transfer(&transfer_id, token.clone());

    tauri::async_runtime::spawn(async move {
        let result = run_download(
            &sftp,
            &events,
            &transfer_id,
            &remote_path,
            &local_path,
            &token,
        )
        .await;
        registry.finish_transfer(&transfer_id);
        finish(&events, transfer_id, local_path, result);
    });
}

/// Spawns a background upload; see [`spawn_download`].
pub fn spawn_upload(
    registry: SftpRegistry,
    sftp: Arc<SftpSession>,
    events: Channel<SftpEvent>,
    transfer_id: String,
    local_path: String,
    remote_path: String,
) {
    let token = CancellationToken::new();
    registry.register_transfer(&transfer_id, token.clone());

    tauri::async_runtime::spawn(async move {
        let result = run_upload(
            &sftp,
            &events,
            &transfer_id,
            &local_path,
            &remote_path,
            &token,
        )
        .await;
        registry.finish_transfer(&transfer_id);
        finish(&events, transfer_id, remote_path, result);
    });
}

fn finish(
    events: &Channel<SftpEvent>,
    transfer_id: String,
    path: String,
    result: AppResult<()>,
) {
    match result {
        Ok(()) => {
            let _ = events.send(SftpEvent::TransferDone { transfer_id, path });
        }
        Err(err) => {
            let _ = events.send(SftpEvent::Error {
                message: err.to_string(),
            });
        }
    }
}

async fn run_download(
    sftp: &SftpSession,
    events: &Channel<SftpEvent>,
    transfer_id: &str,
    remote_path: &str,
    local_path: &str,
    token: &CancellationToken,
) -> AppResult<()> {
    let total = sftp
        .metadata(remote_path)
        .await
        .map_err(map_sftp_err)?
        .size
        .unwrap_or(0);

    let mut remote = sftp.open(remote_path).await.map_err(map_sftp_err)?;
    let mut local = tokio::fs::File::create(local_path)
        .await
        .map_err(|e| AppError::Ssh(format!("Não foi possível criar o arquivo local: {e}.")))?;

    let _ = events.send(SftpEvent::Progress {
        transfer_id: transfer_id.to_string(),
        transferred: 0,
        total,
    });

    let mut buf = vec![0u8; CHUNK];
    let mut transferred: u64 = 0;
    let mut last = Instant::now();

    loop {
        check_cancelled(token)?;
        let n = remote
            .read(&mut buf)
            .await
            .map_err(|e| AppError::Ssh(format!("Falha ao ler do servidor: {e}.")))?;
        if n == 0 {
            break;
        }
        local
            .write_all(&buf[..n])
            .await
            .map_err(|e| AppError::Ssh(format!("Falha ao gravar no disco: {e}.")))?;
        transferred += n as u64;
        emit_progress(events, transfer_id, transferred, total, &mut last);
    }

    local
        .flush()
        .await
        .map_err(|e| AppError::Ssh(format!("Falha ao finalizar o arquivo local: {e}.")))?;
    let _ = events.send(SftpEvent::Progress {
        transfer_id: transfer_id.to_string(),
        transferred,
        total,
    });
    Ok(())
}

async fn run_upload(
    sftp: &SftpSession,
    events: &Channel<SftpEvent>,
    transfer_id: &str,
    local_path: &str,
    remote_path: &str,
    token: &CancellationToken,
) -> AppResult<()> {
    let total = tokio::fs::metadata(local_path)
        .await
        .map(|m| m.len())
        .map_err(|e| AppError::Ssh(format!("Não foi possível abrir o arquivo local: {e}.")))?;

    let mut local = tokio::fs::File::open(local_path)
        .await
        .map_err(|e| AppError::Ssh(format!("Não foi possível abrir o arquivo local: {e}.")))?;
    let mut remote = sftp.create(remote_path).await.map_err(map_sftp_err)?;

    let _ = events.send(SftpEvent::Progress {
        transfer_id: transfer_id.to_string(),
        transferred: 0,
        total,
    });

    let mut buf = vec![0u8; CHUNK];
    let mut transferred: u64 = 0;
    let mut last = Instant::now();

    loop {
        check_cancelled(token)?;
        let n = local
            .read(&mut buf)
            .await
            .map_err(|e| AppError::Ssh(format!("Falha ao ler do disco: {e}.")))?;
        if n == 0 {
            break;
        }
        remote
            .write_all(&buf[..n])
            .await
            .map_err(|e| AppError::Ssh(format!("Falha ao enviar ao servidor: {e}.")))?;
        transferred += n as u64;
        emit_progress(events, transfer_id, transferred, total, &mut last);
    }

    // Properly close the remote handle so the server flushes the file.
    remote
        .shutdown()
        .await
        .map_err(|e| AppError::Ssh(format!("Falha ao finalizar o envio: {e}.")))?;
    let _ = events.send(SftpEvent::Progress {
        transfer_id: transfer_id.to_string(),
        transferred,
        total,
    });
    Ok(())
}

fn check_cancelled(token: &CancellationToken) -> AppResult<()> {
    if token.is_cancelled() {
        Err(AppError::Ssh("Transferência cancelada.".into()))
    } else {
        Ok(())
    }
}

fn emit_progress(
    events: &Channel<SftpEvent>,
    transfer_id: &str,
    transferred: u64,
    total: u64,
    last: &mut Instant,
) {
    if last.elapsed() >= PROGRESS_INTERVAL {
        let _ = events.send(SftpEvent::Progress {
            transfer_id: transfer_id.to_string(),
            transferred,
            total,
        });
        *last = Instant::now();
    }
}
