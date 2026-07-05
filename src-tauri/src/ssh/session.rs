use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use russh::ChannelMsg;
use russh::client::Handle;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::ssh::client::TofuHandler;
use crate::ssh::events::TerminalEvent;
use crate::ssh::registry::SessionInput;

/// Abre PTY + shell e faz a ponte bidirecional entre a sessão SSH e o
/// frontend (saída via `Channel`, entrada via `mpsc`).
pub async fn open_shell_and_bridge(
    handle: Handle<TofuHandler>,
    cols: u32,
    rows: u32,
    events: Channel<TerminalEvent>,
    mut input_rx: mpsc::Receiver<SessionInput>,
    on_finished: impl FnOnce() + Send + 'static,
) -> AppResult<()> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(AppError::from)?;

    channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;

    // Bridge roda em task própria; o command `ssh_connect` retorna em seguida.
    tauri::async_runtime::spawn(async move {
        let reason = loop {
            tokio::select! {
                input = input_rx.recv() => match input {
                    Some(SessionInput::Data(bytes)) => {
                        if channel.data(&bytes[..]).await.is_err() {
                            let _ = events.send(TerminalEvent::Error {
                                message: "Falha ao enviar dados: conexão perdida.".into(),
                            });
                            break "conexão perdida".to_string();
                        }
                    }
                    Some(SessionInput::Resize { cols, rows }) => {
                        let _ = channel.window_change(cols, rows, 0, 0).await;
                    }
                    Some(SessionInput::Close) | None => {
                        let _ = channel.eof().await;
                        break "desconectado pelo usuário".to_string();
                    }
                },
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        let _ = events.send(TerminalEvent::Output {
                            data: B64.encode(data),
                        });
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        let _ = events.send(TerminalEvent::Output {
                            data: B64.encode(data),
                        });
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        // aguarda Close/Eof subsequente; registra o status
                        log::info!("sessão encerrou com status {exit_status}");
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        break "sessão encerrada pelo servidor".to_string();
                    }
                    _ => {}
                },
            }
        };

        let _ = events.send(TerminalEvent::Closed { reason });
        on_finished();
    });

    Ok(())
}
