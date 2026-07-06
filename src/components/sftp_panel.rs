use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

use crate::bindings::sftp::{SftpHandle, pick_open_path, pick_save_path, start_sftp};
use crate::components::confirm_dialog::ConfirmDialog;
use crate::models::{EntryKind, RemoteEntry, SshConnection};
use crate::sftp_api;

#[derive(Clone, PartialEq)]
enum Status {
    Connecting,
    Connected,
    Closed(String),
    Error(String),
}

#[derive(Clone)]
struct Progress {
    transfer_id: String,
    transferred: u64,
    total: u64,
}

#[component]
pub fn SftpPanel(
    connection: SshConnection,
    instance_id: u64,
    on_close: Callback<()>,
) -> impl IntoView {
    let status = RwSignal::new(Status::Connecting);
    let host_key_prompt = RwSignal::new(Option::<HostKeyInfo>::None);
    let session_id = RwSignal::new(Option::<String>::None);
    let current_path = RwSignal::new(String::from("/"));
    let entries = RwSignal::new(Vec::<RemoteEntry>::new());
    let banner = RwSignal::new(Option::<String>::None);
    let progress = RwSignal::new(Option::<Progress>::None);
    let deleting = RwSignal::new(Option::<RemoteEntry>::None);
    // SftpHandle is not Send, so it must stay in the local reactive runtime.
    let handle = StoredValue::new_local(Option::<SftpHandle>::None);

    let conn_id = connection.id.clone();
    let title = format!(
        "{} — {}@{}:{}",
        connection.name, connection.username, connection.host, connection.port
    );
    let _ = instance_id;

    // Loads a remote directory into the panel.
    let load = move |path: String| {
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match sftp_api::list_dir(&sid, &path).await {
                Ok(list) => {
                    entries.set(list);
                    current_path.set(path);
                }
                Err(e) => banner.set(Some(e.message())),
            }
        });
    };

    Effect::new(move |_| {
        let conn_id = conn_id.clone();

        let on_event = Closure::<dyn FnMut(String, Option<String>)>::new(
            move |event: String, detail: Option<String>| match event.as_str() {
                "connected" => {
                    status.set(Status::Connected);
                    if let Some(sid) = session_id.get_untracked() {
                        spawn_local(async move {
                            let home = sftp_api::realpath(&sid, ".")
                                .await
                                .unwrap_or_else(|_| "/".into());
                            match sftp_api::list_dir(&sid, &home).await {
                                Ok(list) => {
                                    entries.set(list);
                                    current_path.set(home);
                                }
                                Err(e) => banner.set(Some(e.message())),
                            }
                        });
                    }
                }
                "hostKeyPrompt" => {
                    if let Some(info) = detail.and_then(|d| HostKeyInfo::parse(&d)) {
                        host_key_prompt.set(Some(info));
                    }
                }
                "progress" => {
                    if let Some((transfer_id, transferred, total)) =
                        detail.as_deref().and_then(parse_progress)
                    {
                        progress.set(Some(Progress {
                            transfer_id,
                            transferred,
                            total,
                        }));
                    }
                }
                "transferDone" => {
                    progress.set(None);
                    load(current_path.get_untracked());
                }
                "error" => {
                    progress.set(None);
                    let msg = detail.unwrap_or_else(|| "erro no SFTP".into());
                    // Keep the panel usable; a transfer error is not fatal.
                    if status.get_untracked() == Status::Connecting {
                        status.set(Status::Error(msg));
                    } else {
                        banner.set(Some(msg));
                    }
                }
                "closed" => {
                    status.set(Status::Closed(
                        detail.unwrap_or_else(|| "sessão encerrada".into()),
                    ));
                }
                _ => {}
            },
        );

        // Keep the JS callback alive for the lifetime of the panel instance.
        let on_event = on_event.into_js_value();
        spawn_local(async move {
            match start_sftp(&conn_id, &on_event).await {
                Ok(h) => {
                    let h = h.unchecked_into::<SftpHandle>();
                    session_id.set(Some(h.get_session_id()));
                    handle.set_value(Some(h));
                }
                Err(err) => status.set(Status::Error(format!("Falha ao iniciar SFTP: {err:?}"))),
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

    let go_up = move |_| {
        let parent = parent_path(&current_path.get_untracked());
        load(parent);
    };
    let refresh = move |_| load(current_path.get_untracked());

    let open_entry = move |entry: RemoteEntry| {
        if entry.kind == EntryKind::Dir {
            load(entry.path.clone());
        }
    };

    let download = move |entry: RemoteEntry| {
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        spawn_local(async move {
            let Some(local) = pick_path(pick_save_path(&entry.name).await) else {
                return;
            };
            let transfer_id = new_id();
            if let Err(e) = sftp_api::download(&sid, &transfer_id, &entry.path, &local).await {
                banner.set(Some(e.message()));
            }
        });
    };

    let upload = move |_| {
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        let dir = current_path.get_untracked();
        spawn_local(async move {
            let Some(local) = pick_path(pick_open_path().await) else {
                return;
            };
            let name = local_basename(&local);
            let remote = join_path(&dir, &name);
            let transfer_id = new_id();
            if let Err(e) = sftp_api::upload(&sid, &transfer_id, &local, &remote).await {
                banner.set(Some(e.message()));
            }
        });
    };

    let make_dir = move |_| {
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        let Some(name) = prompt("Nome da nova pasta:", "") else {
            return;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let path = join_path(&current_path.get_untracked(), &name);
        spawn_local(async move {
            match sftp_api::mkdir(&sid, &path).await {
                Ok(()) => load(current_path.get_untracked()),
                Err(e) => banner.set(Some(e.message())),
            }
        });
    };

    let rename = move |entry: RemoteEntry| {
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        let Some(new_name) = prompt("Novo nome:", &entry.name) else {
            return;
        };
        let new_name = new_name.trim().to_string();
        if new_name.is_empty() || new_name == entry.name {
            return;
        }
        let to = join_path(&parent_path(&entry.path), &new_name);
        let from = entry.path.clone();
        spawn_local(async move {
            match sftp_api::rename(&sid, &from, &to).await {
                Ok(()) => load(current_path.get_untracked()),
                Err(e) => banner.set(Some(e.message())),
            }
        });
    };

    let confirm_delete = Callback::new(move |_| {
        let Some(entry) = deleting.get() else { return };
        deleting.set(None);
        let Some(sid) = session_id.get_untracked() else {
            return;
        };
        spawn_local(async move {
            let result = if entry.kind == EntryKind::Dir {
                sftp_api::remove_dir(&sid, &entry.path).await
            } else {
                sftp_api::remove_file(&sid, &entry.path).await
            };
            match result {
                Ok(()) => load(current_path.get_untracked()),
                Err(e) => banner.set(Some(e.message())),
            }
        });
    });

    let status_badge = move || match status.get() {
        Status::Connecting => ("connecting", "Conectando…"),
        Status::Connected => ("connected", "Conectado"),
        Status::Closed(_) => ("closed", "Desconectado"),
        Status::Error(_) => ("error", "Erro"),
    };
    let is_connected = move || status.get() == Status::Connected;

    view! {
        <div class="sftp-panel">
            <div class="terminal-header">
                <div class="terminal-title">
                    <span class=move || format!("status-dot {}", status_badge().0)></span>
                    <span>{title}</span>
                    <span class="status-label">{move || status_badge().1}</span>
                </div>
                <div class="terminal-actions">
                    <button class="btn btn-sm" on:click=leave>
                        {move || if is_connected() { "Desconectar" } else { "Fechar" }}
                    </button>
                </div>
            </div>

            {move || match status.get() {
                Status::Error(msg) => Some(view! { <div class="terminal-banner error">{msg}</div> }),
                Status::Closed(reason) => {
                    Some(view! { <div class="terminal-banner closed">{reason}</div> })
                }
                _ => None,
            }}

            {move || {
                banner
                    .get()
                    .map(|msg| {
                        view! {
                            <div class="error-banner">
                                <span>{msg}</span>
                                <button class="icon-btn" on:click=move |_| banner.set(None)>
                                    "✕"
                                </button>
                            </div>
                        }
                    })
            }}

            <Show when=is_connected>
                <div class="sftp-toolbar">
                    <button class="icon-btn" title="Subir" on:click=go_up>
                        "⬆"
                    </button>
                    <button class="icon-btn" title="Atualizar" on:click=refresh>
                        "⟳"
                    </button>
                    <span class="sftp-path">{move || current_path.get()}</span>
                    <div class="sftp-toolbar-actions">
                        <button class="btn btn-sm" on:click=make_dir>
                            "Nova pasta"
                        </button>
                        <button class="btn btn-sm btn-primary" on:click=upload>
                            "Enviar arquivo"
                        </button>
                    </div>
                </div>

                {move || {
                    progress
                        .get()
                        .map(|p| {
                            let pct = if p.total > 0 {
                                (p.transferred as f64 / p.total as f64 * 100.0).round() as u32
                            } else {
                                0
                            };
                            let transfer_id = p.transfer_id.clone();
                            let cancel = move |_| {
                                let Some(sid) = session_id.get_untracked() else {
                                    return;
                                };
                                let transfer_id = transfer_id.clone();
                                spawn_local(async move {
                                    let _ = sftp_api::cancel_transfer(&sid, &transfer_id).await;
                                });
                            };
                            view! {
                                <div class="sftp-progress">
                                    <div class="sftp-progress-bar">
                                        <div
                                            class="sftp-progress-fill"
                                            style=move || format!("width:{pct}%")
                                        ></div>
                                    </div>
                                    <span class="sftp-progress-label">
                                        {format!(
                                            "{pct}% ({} / {})",
                                            human_size(p.transferred),
                                            human_size(p.total),
                                        )}
                                    </span>
                                    <button class="icon-btn danger" title="Cancelar" on:click=cancel>
                                        "✕"
                                    </button>
                                </div>
                            }
                        })
                }}

                <div class="sftp-list">
                    <div class="sftp-row sftp-head">
                        <span class="sftp-col-name">"Nome"</span>
                        <span class="sftp-col-size">"Tamanho"</span>
                        <span class="sftp-col-date">"Modificado"</span>
                        <span class="sftp-col-actions"></span>
                    </div>
                    {move || {
                        let items = entries.get();
                        if items.is_empty() {
                            return view! {
                                <div class="sftp-empty">"Pasta vazia."</div>
                            }
                                .into_any();
                        }
                        items
                            .into_iter()
                            .map(|entry| {
                                let e_open = entry.clone();
                                let e_dl = entry.clone();
                                let e_rn = entry.clone();
                                let e_del = entry.clone();
                                let is_dir = entry.kind == EntryKind::Dir;
                                let icon = match entry.kind {
                                    EntryKind::Dir => "📁",
                                    EntryKind::Symlink => "🔗",
                                    EntryKind::File => "📄",
                                };
                                let open_entry = open_entry;
                                let download = download;
                                let rename = rename;
                                view! {
                                    <div class="sftp-row">
                                        <span
                                            class="sftp-col-name"
                                            class:clickable=is_dir
                                            on:click=move |_| open_entry(e_open.clone())
                                        >
                                            <span class="sftp-icon">{icon}</span>
                                            {entry.name.clone()}
                                        </span>
                                        <span class="sftp-col-size">
                                            {if is_dir {
                                                String::new()
                                            } else {
                                                human_size(entry.size)
                                            }}
                                        </span>
                                        <span class="sftp-col-date">
                                            {entry.modified.map(format_mtime).unwrap_or_default()}
                                        </span>
                                        <span class="sftp-col-actions">
                                            <Show when=move || !is_dir>
                                                <button
                                                    class="icon-btn"
                                                    title="Baixar"
                                                    on:click={
                                                        let e_dl = e_dl.clone();
                                                        move |ev| {
                                                            ev.stop_propagation();
                                                            download(e_dl.clone());
                                                        }
                                                    }
                                                >
                                                    "⬇"
                                                </button>
                                            </Show>
                                            <button
                                                class="icon-btn"
                                                title="Renomear"
                                                on:click={
                                                    let e_rn = e_rn.clone();
                                                    move |ev| {
                                                        ev.stop_propagation();
                                                        rename(e_rn.clone());
                                                    }
                                                }
                                            >
                                                "✎"
                                            </button>
                                            <button
                                                class="icon-btn danger"
                                                title="Remover"
                                                on:click={
                                                    let e_del = e_del.clone();
                                                    move |ev| {
                                                        ev.stop_propagation();
                                                        deleting.set(Some(e_del.clone()));
                                                    }
                                                }
                                            >
                                                "🗑"
                                            </button>
                                        </span>
                                    </div>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
                </div>
            </Show>

            {move || {
                host_key_prompt
                    .get()
                    .map(|info| {
                        let confirm = confirm_host_key;
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
                                            on:click=move |_| confirm(false)
                                        >
                                            "Recusar"
                                        </button>
                                        <button
                                            class="btn btn-primary"
                                            on:click=move |_| confirm(true)
                                        >
                                            "Confiar e conectar"
                                        </button>
                                    </div>
                                </div>
                            </div>
                        }
                    })
            }}

            {move || {
                deleting
                    .get()
                    .map(|entry| {
                        let kind = if entry.kind == EntryKind::Dir {
                            "a pasta"
                        } else {
                            "o arquivo"
                        };
                        view! {
                            <ConfirmDialog
                                title="Remover".to_string()
                                message=format!("Remover {} \"{}\"?", kind, entry.name)
                                confirm_label="Remover".to_string()
                                on_confirm=confirm_delete
                                on_cancel=Callback::new(move |_| deleting.set(None))
                            />
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

fn parse_progress(json: &str) -> Option<(String, u64, u64)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    Some((
        value.get("transferId")?.as_str()?.to_string(),
        value.get("transferred")?.as_u64()?,
        value.get("total")?.as_u64()?,
    ))
}

/// POSIX parent of an absolute path.
fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
    }
}

/// Joins a POSIX directory and a child name.
fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Last path component of a local (possibly Windows) path.
fn local_basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn format_mtime(secs: i64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(secs as f64 * 1000.0));
    date.to_locale_string("pt-BR", &JsValue::UNDEFINED)
        .as_string()
        .unwrap_or_default()
}

fn new_id() -> String {
    // The backend keys transfers by this id; uniqueness is all that matters.
    let now = js_sys::Date::now();
    let rnd = js_sys::Math::random();
    format!("t-{}-{}", now as u64, (rnd * 1e9) as u64)
}

/// Reads a `String` back from a dialog result (`null` becomes `None`).
fn pick_path(result: Result<JsValue, JsValue>) -> Option<String> {
    result.ok().and_then(|v| v.as_string())
}

/// Native browser prompt; returns `None` if the user cancels.
fn prompt(message: &str, default: &str) -> Option<String> {
    web_sys::window()?
        .prompt_with_message_and_default(message, default)
        .ok()
        .flatten()
}
