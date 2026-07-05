use leptos::prelude::*;

use crate::models::SshConnection;

#[component]
pub fn ConnectionList(
    connections: Signal<Vec<SshConnection>>,
    search: RwSignal<String>,
    selected_id: RwSignal<Option<String>>,
    on_edit: Callback<SshConnection>,
    on_delete: Callback<SshConnection>,
    on_connect: Callback<SshConnection>,
) -> impl IntoView {
    let grouped = Memo::new(move |_| {
        let query = search.get().trim().to_lowercase();
        let mut groups: Vec<(String, Vec<SshConnection>)> = Vec::new();

        for conn in connections.get() {
            if !query.is_empty() {
                let haystack = format!(
                    "{} {} {}",
                    conn.name.to_lowercase(),
                    conn.host.to_lowercase(),
                    conn.group_name.to_lowercase()
                );
                if !haystack.contains(&query) {
                    continue;
                }
            }
            match groups.iter_mut().find(|(g, _)| *g == conn.group_name) {
                Some((_, items)) => items.push(conn),
                None => groups.push((conn.group_name.clone(), vec![conn])),
            }
        }
        groups
    });

    let is_empty = Memo::new(move |_| connections.get().is_empty());
    let nothing_found =
        Memo::new(move |_| !connections.get().is_empty() && grouped.get().is_empty());

    view! {
        <div class="connection-list">
            <div class="search-box">
                <input
                    type="text"
                    placeholder="Buscar por nome, host ou grupo…"
                    prop:value=move || search.get()
                    on:input=move |ev| search.set(event_target_value(&ev))
                />
            </div>

            <Show when=move || is_empty.get()>
                <div class="empty-state">
                    <p>"Nenhuma conexão cadastrada."</p>
                    <p class="hint">"Clique em \"Nova conexão\" para começar."</p>
                </div>
            </Show>

            <Show when=move || nothing_found.get()>
                <div class="empty-state">
                    <p>"Nada encontrado para essa busca."</p>
                </div>
            </Show>

            // Use a reactive outer block so item edits rerender even when group counts are unchanged.
            {move || {
                grouped
                    .get()
                    .into_iter()
                    .map(|(group, items)| {
                        view! {
                            <div class="group">
                                <div class="group-title">{group}</div>
                                <For
                                    each=move || items.clone()
                                    key=|conn| (conn.id.clone(), conn.updated_at.clone())
                                    children=move |conn| {
                                    let conn_select = conn.clone();
                                    let conn_edit = conn.clone();
                                    let conn_delete = conn.clone();
                                    let conn_connect = conn.clone();
                                    let id = conn.id.clone();
                                    let is_selected =
                                        move || selected_id.get().as_deref() == Some(id.as_str());
                                    view! {
                                        <div
                                            class="connection-item"
                                            class:selected=is_selected
                                            on:click=move |_| {
                                                selected_id.set(Some(conn_select.id.clone()))
                                            }
                                        >
                                            <div class="connection-info">
                                                <span class="connection-name">{conn.name.clone()}</span>
                                                <span class="connection-target">
                                                    {format!(
                                                        "{}@{}:{}",
                                                        conn.username, conn.host, conn.port,
                                                    )}
                                                </span>
                                            </div>
                                            <div class="connection-actions">
                                                <button
                                                    class="icon-btn connect"
                                                    title="Conectar"
                                                    on:click=move |ev| {
                                                        ev.stop_propagation();
                                                        on_connect.run(conn_connect.clone());
                                                    }
                                                >
                                                    "▶"
                                                </button>
                                                <button
                                                    class="icon-btn"
                                                    title="Editar"
                                                    on:click=move |ev| {
                                                        ev.stop_propagation();
                                                        on_edit.run(conn_edit.clone());
                                                    }
                                                >
                                                    "✎"
                                                </button>
                                                <button
                                                    class="icon-btn danger"
                                                    title="Remover"
                                                    on:click=move |ev| {
                                                        ev.stop_propagation();
                                                        on_delete.run(conn_delete.clone());
                                                    }
                                                >
                                                    "🗑"
                                                </button>
                                            </div>
                                        </div>
                                    }
                                    }
                                />
                            </div>
                        }
                    })
                    .collect_view()
            }}
        </div>
    }
}
