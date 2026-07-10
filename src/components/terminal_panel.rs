use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

use crate::bindings::terminal::{TerminalHandle, start_terminal};
use crate::components::agent_panel::AgentPanel;
use crate::models::SshConnection;

#[derive(Clone, PartialEq)]
enum Status {
    Connecting,
    Connected,
    Closed(String),
    Error(String),
}

#[component]
pub fn TerminalPanel(
    connection: SshConnection,
    instance_id: u64,
    on_close: Callback<()>,
) -> impl IntoView {
    let status = RwSignal::new(Status::Connecting);
    let host_key_prompt = RwSignal::new(Option::<HostKeyInfo>::None);
    // TerminalHandle is not Send, so it must stay in the local reactive runtime.
    let handle = StoredValue::new_local(Option::<TerminalHandle>::None);
    // Session id do backend, disponível assim que o handle existe; o painel
    // do agente endereça a sessão por ele.
    let session_id = RwSignal::new(Option::<String>::None);
    let agent_open = RwSignal::new(false);

    let conn_id = connection.id.clone();
    let container_id = format!("terminal-container-{instance_id}");
    let effect_container_id = container_id.clone();
    let title = format!(
        "{} — {}@{}:{}",
        connection.name, connection.username, connection.host, connection.port
    );
    let agent_connection = StoredValue::new(connection);

    Effect::new(move |_| {
        let conn_id = conn_id.clone();
        let container_id = effect_container_id.clone();

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

        // Keep the JS callback alive for the lifetime of the terminal instance.
        let on_status = on_status.into_js_value();
        spawn_local(async move {
            match start_terminal(&container_id, &conn_id, &on_status).await {
                Ok(h) => {
                    let h = h.unchecked_into::<TerminalHandle>();
                    session_id.set(Some(h.get_session_id()));
                    handle.set_value(Some(h));
                }
                Err(err) => {
                    status.set(Status::Error(format!("Falha ao iniciar terminal: {err:?}")))
                }
            }
        });
    });

    on_cleanup(move || {
        handle.update_value(|h| {
            if let Some(h) = h.take() {
                h.dispose();
            }
        });
    });

    let leave = move |_| {
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
                        class:btn-primary=move || agent_open.get()
                        disabled=move || session_id.get().is_none()
                        on:click=move |_| agent_open.update(|open| *open = !*open)
                    >
                        "Agente"
                    </button>
                    <button class="btn btn-sm" on:click=leave>
                        {move || {
                            if status.get() == Status::Connected {
                                "Desconectar"
                            } else {
                                "Fechar"
                            }
                        }}
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

            <div class="terminal-body">
                <div id=container_id class="terminal-container"></div>
                {move || {
                    agent_open
                        .get()
                        .then(|| {
                            view! {
                                <AgentPanel
                                    connection=agent_connection.get_value()
                                    session_id=session_id.into()
                                    on_close=Callback::new(move |_| agent_open.set(false))
                                />
                            }
                        })
                }}
            </div>

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
