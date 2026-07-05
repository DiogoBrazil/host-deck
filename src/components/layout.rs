use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::connection_form::ConnectionForm;
use crate::components::connection_list::ConnectionList;
use crate::components::terminal_panel::TerminalPanel;
use crate::models::SshConnection;

/// Estado do modal de formulário: fechado, criar ou editar.
#[derive(Clone, PartialEq)]
enum FormState {
    Closed,
    New,
    Edit(SshConnection),
}

#[component]
pub fn Layout() -> impl IntoView {
    let connections = RwSignal::new(Vec::<SshConnection>::new());
    let search = RwSignal::new(String::new());
    let selected_id = RwSignal::new(Option::<String>::None);
    let form_state = RwSignal::new(FormState::Closed);
    let deleting = RwSignal::new(Option::<SshConnection>::None);
    let error_banner = RwSignal::new(Option::<String>::None);
    // Conexão com terminal aberto (uma por vez no MVP).
    let active_session = RwSignal::new(Option::<SshConnection>::None);

    let reload = move || {
        spawn_local(async move {
            match api::list_connections().await {
                Ok(list) => connections.set(list),
                Err(err) => error_banner.set(Some(err.message())),
            }
        });
    };

    // Carga inicial.
    reload();

    let on_saved = Callback::new(move |conn: SshConnection| {
        form_state.set(FormState::Closed);
        selected_id.set(Some(conn.id));
        reload();
    });

    let on_edit = Callback::new(move |conn: SshConnection| {
        form_state.set(FormState::Edit(conn));
    });

    let on_delete = Callback::new(move |conn: SshConnection| {
        deleting.set(Some(conn));
    });

    let on_connect = Callback::new(move |conn: SshConnection| {
        selected_id.set(Some(conn.id.clone()));
        active_session.set(Some(conn));
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
                {move || match active_session.get() {
                    Some(conn) => {
                        view! {
                            <TerminalPanel
                                connection=conn
                                on_close=Callback::new(move |_| active_session.set(None))
                            />
                        }
                            .into_any()
                    }
                    None => {
                        view! {
                            <div class="main-placeholder">
                                <p class="title">"Bem-vindo ao HostDeck"</p>
                                <p>"Selecione uma conexão e clique em ▶ para abrir o terminal."</p>
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
