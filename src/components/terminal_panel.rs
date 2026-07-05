use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

use crate::bindings::terminal::{TerminalHandle, start_terminal};
use crate::models::SshConnection;

#[derive(Clone, PartialEq)]
enum Status {
    Connecting,
    Connected,
    Closed(String),
    Error(String),
}

/// Painel do terminal embutido. Monta o xterm.js via glue JS e reflete o
/// estado da sessão SSH. Uma instância por conexão ativa (a `key` no pai
/// garante remontagem ao trocar de conexão).
#[component]
pub fn TerminalPanel(connection: SshConnection, on_close: Callback<()>) -> impl IntoView {
    let status = RwSignal::new(Status::Connecting);
    let host_key_prompt = RwSignal::new(Option::<HostKeyInfo>::None);
    // TerminalHandle não é Send; StoredValue local o guarda no runtime reativo.
    let handle = StoredValue::new_local(Option::<TerminalHandle>::None);

    let conn_id = connection.id.clone();
    let title = format!(
        "{} — {}@{}:{}",
        connection.name, connection.username, connection.host, connection.port
    );

    // Inicializa o terminal após o div existir no DOM.
    Effect::new(move |_| {
        let conn_id = conn_id.clone();

        // Callback de status vindo do JS: (status, detail?).
        let on_status = Closure::<dyn FnMut(String, Option<String>)>::new(
            move |kind: String, detail: Option<String>| match kind.as_str() {
                "connected" => status.set(Status::Connected),
                "closed" => status.set(Status::Closed(
                    detail.unwrap_or_else(|| "sessão encerrada".into()),
                )),
                "error" => status.set(Status::Error(detail.unwrap_or_default())),
                "hostKeyPrompt" => {
                    if let Some(info) = detail.and_then(|d| HostKeyInfo::parse(&d)) {
                        host_key_prompt.set(Some(info));
                    }
                }
                _ => {}
            },
        );

        // into_js_value mantém o closure vivo enquanto o terminal existir.
        let on_status = on_status.into_js_value();
        spawn_local(async move {
            match start_terminal("terminal-container", &conn_id, &on_status).await {
                Ok(h) => handle.set_value(Some(h.unchecked_into::<TerminalHandle>())),
                Err(err) => {
                    status.set(Status::Error(format!("Falha ao iniciar terminal: {err:?}")))
                }
            }
        });
    });

    let disconnect = move |_| {
        handle.with_value(|h| {
            if let Some(h) = h.as_ref() {
                h.disconnect();
            }
        });
    };

    let close_panel = move |_| {
        handle.update_value(|h| {
            if let Some(h) = h.take() {
                h.dispose();
            }
        });
        on_close.run(());
    };

    let confirm_host_key = move |accept: bool| {
        handle.with_value(|h| {
            if let Some(h) = h.as_ref() {
                h.confirm_host_key(accept);
            }
        });
        host_key_prompt.set(None);
    };
    let confirm_yes = confirm_host_key;

    let status_badge = move || match status.get() {
        Status::Connecting => ("connecting", "Conectando…"),
        Status::Connected => ("connected", "Conectado"),
        Status::Closed(_) => ("closed", "Desconectado"),
        Status::Error(_) => ("error", "Erro"),
    };

    view! {
        <div class="terminal-panel">
            <div class="terminal-header">
                <div class="terminal-title">
                    <span class=move || format!("status-dot {}", status_badge().0)></span>
                    <span>{title}</span>
                    <span class="status-label">{move || status_badge().1}</span>
                </div>
                <div class="terminal-actions">
                    <button
                        class="btn btn-sm"
                        on:click=disconnect
                        disabled=move || status.get() != Status::Connected
                    >
                        "Desconectar"
                    </button>
                    <button class="btn btn-sm" on:click=close_panel>
                        "Fechar"
                    </button>
                </div>
            </div>

            {move || match status.get() {
                Status::Error(msg) => {
                    Some(view! { <div class="terminal-banner error">{msg}</div> })
                }
                Status::Closed(reason) => {
                    Some(view! { <div class="terminal-banner closed">{reason}</div> })
                }
                _ => None,
            }}

            <div id="terminal-container" class="terminal-container"></div>

            {move || {
                host_key_prompt
                    .get()
                    .map(|info| {
                        let confirm_yes = confirm_yes.clone();
                        let confirm_no = confirm_host_key.clone();
                        view! {
                            <div class="modal-backdrop">
                                <div class="modal modal-sm">
                                    <h2>"Verificar host desconhecido"</h2>
                                    <p class="confirm-message">
                                        "Esta é a primeira conexão a este servidor. Confirme que o fingerprint abaixo corresponde ao servidor real antes de continuar."
                                    </p>
                                    <div class="fingerprint-box">
                                        <div><strong>"Tipo: "</strong>{info.key_type}</div>
                                        <div><strong>"Fingerprint: "</strong>{info.fingerprint}</div>
                                    </div>
                                    <div class="modal-actions">
                                        <button
                                            class="btn btn-danger"
                                            on:click=move |_| confirm_no(false)
                                        >
                                            "Recusar"
                                        </button>
                                        <button
                                            class="btn btn-primary"
                                            on:click=move |_| confirm_yes(true)
                                        >
                                            "Confiar e conectar"
                                        </button>
                                    </div>
                                </div>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[derive(Clone)]
struct HostKeyInfo {
    fingerprint: String,
    key_type: String,
}

impl HostKeyInfo {
    fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        Some(Self {
            fingerprint: value.get("fingerprint")?.as_str()?.to_string(),
            key_type: value.get("keyType")?.as_str()?.to_string(),
        })
    }
}
