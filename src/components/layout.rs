use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::connection_form::ConnectionForm;
use crate::components::connection_list::ConnectionList;
use crate::components::sftp_panel::SftpPanel;
use crate::components::terminal_panel::TerminalPanel;
use crate::models::SshConnection;

#[derive(Clone, PartialEq)]
enum FormState {
    Closed,
    New,
    Edit(SshConnection),
}

#[derive(Clone, PartialEq)]
struct ActiveSession {
    connection: SshConnection,
    instance_id: u64,
}

/// The active main-area view: a terminal or an SFTP file browser.
#[derive(Clone, PartialEq)]
enum ActiveView {
    Terminal(ActiveSession),
    Sftp(ActiveSession),
}

impl ActiveView {
    fn session_mut(&mut self) -> &mut ActiveSession {
        match self {
            ActiveView::Terminal(s) | ActiveView::Sftp(s) => s,
        }
    }
}

#[component]
pub fn Layout() -> impl IntoView {
    let connections = RwSignal::new(Vec::<SshConnection>::new());
    let search = RwSignal::new(String::new());
    let selected_id = RwSignal::new(Option::<String>::None);
    let form_state = RwSignal::new(FormState::Closed);
    let deleting = RwSignal::new(Option::<SshConnection>::None);
    let error_banner = RwSignal::new(Option::<String>::None);
    let active_view = RwSignal::new(Option::<ActiveView>::None);
    let session_counter = RwSignal::new(0_u64);

    let reload = move || {
        spawn_local(async move {
            match api::list_connections().await {
                Ok(list) => connections.set(list),
                Err(err) => error_banner.set(Some(err.message())),
            }
        });
    };

    reload();

    let upsert_connection = move |conn: SshConnection| {
        connections.update(|list| {
            match list.iter_mut().find(|item| item.id == conn.id) {
                Some(existing) => *existing = conn.clone(),
                None => list.push(conn.clone()),
            }
            list.sort_by(|a, b| {
                a.group_name
                    .cmp(&b.group_name)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        });

        active_view.update(|view| {
            if let Some(active) = view {
                let session = active.session_mut();
                if session.connection.id == conn.id {
                    session.connection = conn.clone();
                }
            }
        });
    };

    let next_session = move |conn: SshConnection| {
        let next = session_counter.get() + 1;
        session_counter.set(next);
        ActiveSession {
            connection: conn,
            instance_id: next,
        }
    };

    let open_terminal = move |conn: SshConnection| {
        active_view.set(Some(ActiveView::Terminal(next_session(conn))));
    };

    let open_sftp = move |conn: SshConnection| {
        active_view.set(Some(ActiveView::Sftp(next_session(conn))));
    };

    let on_saved = Callback::new(move |(conn, connect): (SshConnection, bool)| {
        form_state.set(FormState::Closed);
        selected_id.set(Some(conn.id.clone()));
        upsert_connection(conn.clone());
        reload();
        if connect {
            open_terminal(conn);
        }
    });

    let on_edit = Callback::new(move |conn: SshConnection| {
        form_state.set(FormState::Edit(conn));
    });

    let on_delete = Callback::new(move |conn: SshConnection| {
        deleting.set(Some(conn));
    });

    let on_connect = Callback::new(move |conn: SshConnection| {
        selected_id.set(Some(conn.id.clone()));
        open_terminal(conn);
    });

    let on_open_sftp = Callback::new(move |conn: SshConnection| {
        selected_id.set(Some(conn.id.clone()));
        open_sftp(conn);
    });

    let confirm_delete = Callback::new(move |_| {
        let Some(conn) = deleting.get() else { return };
        deleting.set(None);
        spawn_local(async move {
            match api::delete_connection(&conn.id).await {
                Ok(()) => {
                    if selected_id.get().as_deref() == Some(conn.id.as_str()) {
                        selected_id.set(None);
                    }
                    reload();
                }
                Err(err) => error_banner.set(Some(err.message())),
            }
        });
    });

    view! {
        <div class="app-layout">
            <aside class="sidebar">
                <div class="sidebar-header">
                    <span class="logo">"HostDeck"</span>
                </div>
                <div class="sidebar-body">
                    <ConnectionList
                        connections=connections.into()
                        search=search
                        selected_id=selected_id
                        on_edit=on_edit
                        on_delete=on_delete
                        on_connect=on_connect
                        on_open_sftp=on_open_sftp
                    />
                </div>
                <div class="sidebar-footer">
                    <button
                        class="btn btn-primary btn-block"
                        on:click=move |_| form_state.set(FormState::New)
                    >
                        "+ Nova conexão"
                    </button>
                </div>
            </aside>

            <main class="main-area">
                {move || {
                    error_banner
                        .get()
                        .map(|msg| {
                            view! {
                                <div class="error-banner">
                                    <span>{msg}</span>
                                    <button
                                        class="icon-btn"
                                        on:click=move |_| error_banner.set(None)
                                    >
                                        "✕"
                                    </button>
                                </div>
                            }
                        })
                }}
                {move || match active_view.get() {
                    Some(ActiveView::Terminal(session)) => {
                        view! {
                            <TerminalPanel
                                connection=session.connection
                                instance_id=session.instance_id
                                on_close=Callback::new(move |_| active_view.set(None))
                            />
                        }
                            .into_any()
                    }
                    Some(ActiveView::Sftp(session)) => {
                        view! {
                            <SftpPanel
                                connection=session.connection
                                instance_id=session.instance_id
                                on_close=Callback::new(move |_| active_view.set(None))
                            />
                        }
                            .into_any()
                    }
                    None => {
                        view! {
                            <div class="main-placeholder">
                                <p class="title">"Bem-vindo ao HostDeck"</p>
                                <p>
                                    "Selecione uma conexão e clique em ▶ para o terminal ou 📁 para os arquivos."
                                </p>
                            </div>
                        }
                            .into_any()
                    }
                }}
            </main>

            {move || match form_state.get() {
                FormState::Closed => None,
                FormState::New => {
                    Some(
                        view! {
                            <ConnectionForm
                                editing=None
                                on_saved=on_saved
                                on_cancel=Callback::new(move |_| form_state.set(FormState::Closed))
                            />
                        },
                    )
                }
                FormState::Edit(conn) => {
                    Some(
                        view! {
                            <ConnectionForm
                                editing=Some(conn)
                                on_saved=on_saved
                                on_cancel=Callback::new(move |_| form_state.set(FormState::Closed))
                            />
                        },
                    )
                }
            }}

            {move || {
                deleting
                    .get()
                    .map(|conn| {
                        view! {
                            <ConfirmDialog
                                title="Remover conexão".to_string()
                                message=format!(
                                    "Remover \"{}\" ({}@{})? A credencial salva no sistema também será removida.",
                                    conn.name, conn.username, conn.host,
                                )
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
