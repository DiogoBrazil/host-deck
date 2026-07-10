use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::models::{AgentProvider, AppError, FieldError, ModelCacheEntry, ProviderInput, ProviderKind};

/// Formats a per-Mtok price pair like `$3.00 / $15.00 por Mtok`.
fn format_price((input, output): (f64, f64)) -> String {
    format!("${input:.2} / ${output:.2} por Mtok")
}

/// Modal de configuração dos provedores de IA: cadastro da chave (keyring),
/// escolha do modelo padrão e custo por milhão de tokens quando anunciado.
#[component]
pub fn ProviderSettings(on_close: Callback<()>) -> impl IntoView {
    let providers = RwSignal::new(Vec::<AgentProvider>::new());
    // None = listagem; Some(None) = novo; Some(Some(p)) = editando p.
    let editing = RwSignal::new(Option::<Option<AgentProvider>>::None);
    let deleting = RwSignal::new(Option::<AgentProvider>::None);
    let error = RwSignal::new(Option::<String>::None);

    let reload = move || {
        spawn_local(async move {
            match api::list_providers().await {
                Ok(list) => providers.set(list),
                Err(err) => error.set(Some(err.message())),
            }
        });
    };
    reload();

    let confirm_delete = Callback::new(move |_| {
        let Some(provider) = deleting.get() else { return };
        deleting.set(None);
        spawn_local(async move {
            match api::delete_provider(&provider.id).await {
                Ok(()) => reload(),
                Err(err) => error.set(Some(err.message())),
            }
        });
    });

    view! {
        <div class="modal-backdrop" on:click=move |_| on_close.run(())>
            <div class="modal modal-lg" on:click=|ev| ev.stop_propagation()>
                {move || match editing.get() {
                    None => {
                        view! {
                            <h2>"Provedores de IA"</h2>
                            {move || {
                                error.get().map(|msg| view! { <div class="form-error-banner">{msg}</div> })
                            }}
                            <div class="provider-list">
                                {move || {
                                    let list = providers.get();
                                    if list.is_empty() {
                                        view! {
                                            <div class="empty-state">
                                                <p>"Nenhum provedor cadastrado."</p>
                                                <p class="hint">
                                                    "Cadastre Anthropic, OpenAI ou OpenRouter para usar o agente no terminal. A chave fica no armazenamento seguro do sistema."
                                                </p>
                                            </div>
                                        }
                                            .into_any()
                                    } else {
                                        list.into_iter()
                                            .map(|p| {
                                                let edit = p.clone();
                                                let del = p.clone();
                                                view! {
                                                    <div class="provider-item">
                                                        <div class="provider-info">
                                                            <span class="provider-label">{p.label.clone()}</span>
                                                            <span class="provider-meta">
                                                                {p.kind.label()}
                                                                {p.model
                                                                    .clone()
                                                                    .map(|m| format!(" · {m}"))
                                                                    .unwrap_or_default()}
                                                                {if p.api_key_ref.is_some() {
                                                                    ""
                                                                } else {
                                                                    " · sem chave"
                                                                }}
                                                            </span>
                                                        </div>
                                                        <div class="provider-actions">
                                                            <button
                                                                class="icon-btn"
                                                                title="Editar"
                                                                on:click=move |_| editing.set(Some(Some(edit.clone())))
                                                            >
                                                                "✎"
                                                            </button>
                                                            <button
                                                                class="icon-btn danger"
                                                                title="Remover"
                                                                on:click=move |_| deleting.set(Some(del.clone()))
                                                            >
                                                                "🗑"
                                                            </button>
                                                        </div>
                                                    </div>
                                                }
                                            })
                                            .collect_view()
                                            .into_any()
                                    }
                                }}
                            </div>
                            <div class="modal-actions">
                                <button class="btn" on:click=move |_| on_close.run(())>
                                    "Fechar"
                                </button>
                                <button
                                    class="btn btn-primary"
                                    on:click=move |_| editing.set(Some(None))
                                >
                                    "+ Novo provedor"
                                </button>
                            </div>
                        }
                            .into_any()
                    }
                    Some(current) => {
                        view! {
                            <ProviderForm
                                editing=current
                                on_done=Callback::new(move |_| {
                                    editing.set(None);
                                    reload();
                                })
                                on_saved=Callback::new(move |p: AgentProvider| {
                                    // Continua no formulário para escolher o modelo.
                                    editing.set(Some(Some(p)));
                                    reload();
                                })
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>

        // Fora do backdrop: cliques no diálogo não podem borbulhar para o
        // on:click que fecha a tela de provedores.
        {move || {
            deleting
                .get()
                .map(|p| {
                    view! {
                        <ConfirmDialog
                            title="Remover provedor".to_string()
                            message=format!(
                                "Remover \"{}\"? A chave de API guardada no sistema também será removida; conexões que o usavam ficam sem provedor.",
                                p.label,
                            )
                            confirm_label="Remover".to_string()
                            on_confirm=confirm_delete
                            on_cancel=Callback::new(move |_| deleting.set(None))
                        />
                    }
                })
        }}
    }
}

/// Create/edit form. The API key is write-only and never prefilled.
#[component]
fn ProviderForm(
    editing: Option<AgentProvider>,
    /// Voltar à listagem (cancelar ou concluir).
    on_done: Callback<()>,
    /// Registro criado: permanece no formulário em modo edição.
    on_saved: Callback<AgentProvider>,
) -> impl IntoView {
    let is_edit = editing.is_some();
    let editing_id = editing.as_ref().map(|p| p.id.clone());
    let has_saved_key = editing.as_ref().is_some_and(|p| p.api_key_ref.is_some());

    let kind = RwSignal::new(
        editing
            .as_ref()
            .map(|p| p.kind)
            .unwrap_or(ProviderKind::Anthropic),
    );
    let label = RwSignal::new(editing.as_ref().map(|p| p.label.clone()).unwrap_or_default());
    let base_url = RwSignal::new(
        editing
            .as_ref()
            .and_then(|p| p.base_url.clone())
            .unwrap_or_default(),
    );
    let model = RwSignal::new(
        editing
            .as_ref()
            .and_then(|p| p.model.clone())
            .unwrap_or_default(),
    );
    let api_key = RwSignal::new(String::new());

    let models = RwSignal::new(Vec::<ModelCacheEntry>::new());
    let refreshing = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let field_errors = RwSignal::new(Vec::<FieldError>::new());
    let general_error = RwSignal::new(Option::<String>::None);
    let notice = RwSignal::new(Option::<String>::None);

    let stored_id = StoredValue::new(editing_id);

    // Cache persistido do provedor em edição, para o dropdown de modelos.
    if is_edit {
        let id = stored_id.get_value().unwrap();
        spawn_local(async move {
            if let Ok(list) = api::list_cached_models(&id).await {
                models.set(list);
            }
        });
    }

    let error_for = move |field: &'static str| {
        field_errors
            .get()
            .into_iter()
            .find(|e| e.field == field)
            .map(|e| e.message)
    };

    let refresh_models = move |_| {
        let Some(id) = stored_id.get_value() else { return };
        if refreshing.get() {
            return;
        }
        refreshing.set(true);
        general_error.set(None);
        spawn_local(async move {
            match api::refresh_models(&id).await {
                Ok(list) => {
                    notice.set(Some(format!("{} modelos disponíveis.", list.len())));
                    models.set(list);
                }
                Err(err) => general_error.set(Some(err.message())),
            }
            refreshing.set(false);
        });
    };

    let submit = move |_| {
        if saving.get() {
            return;
        }
        field_errors.set(Vec::new());
        general_error.set(None);
        saving.set(true);

        let input = ProviderInput {
            kind: kind.get(),
            label: label.get(),
            base_url: {
                let v = base_url.get();
                (!v.trim().is_empty()).then(|| v.trim().to_string())
            },
            model: {
                let v = model.get();
                (!v.trim().is_empty()).then(|| v.trim().to_string())
            },
        };
        let key = {
            let v = api_key.get();
            (!v.trim().is_empty()).then_some(v)
        };

        spawn_local(async move {
            let result = match stored_id.get_value() {
                Some(id) => api::update_provider(&id, &input, key.as_deref()).await,
                None => api::create_provider(&input, key.as_deref()).await,
            };
            saving.set(false);

            match result {
                Ok(provider) => {
                    api_key.set(String::new());
                    if stored_id.get_value().is_some() {
                        on_done.run(());
                    } else {
                        // Criou agora: fica no formulário para buscar os
                        // modelos e escolher o padrão.
                        on_saved.run(provider);
                    }
                }
                Err(AppError::Validation(errors)) => field_errors.set(errors),
                Err(err) => general_error.set(Some(err.message())),
            }
        });
    };

    view! {
        <h2>{if is_edit { "Editar provedor" } else { "Novo provedor" }}</h2>

        {move || {
            general_error
                .get()
                .map(|msg| view! { <div class="form-error-banner">{msg}</div> })
        }}
        {move || notice.get().map(|msg| view! { <div class="form-notice">{msg}</div> })}

        <div class="form-grid">
            <label class="form-field">
                <span>"Provedor *"</span>
                <select on:change=move |ev| {
                    kind.set(match event_target_value(&ev).as_str() {
                        "openai" => ProviderKind::Openai,
                        "openrouter" => ProviderKind::Openrouter,
                        _ => ProviderKind::Anthropic,
                    })
                }>
                    <option value="anthropic" selected=move || kind.get() == ProviderKind::Anthropic>
                        "Anthropic"
                    </option>
                    <option value="openai" selected=move || kind.get() == ProviderKind::Openai>
                        "OpenAI (ou compatível)"
                    </option>
                    <option
                        value="openrouter"
                        selected=move || kind.get() == ProviderKind::Openrouter
                    >
                        "OpenRouter"
                    </option>
                </select>
            </label>

            <label class="form-field">
                <span>"Nome"</span>
                <input
                    type="text"
                    placeholder="Se vazio: nome do provedor"
                    prop:value=move || label.get()
                    on:input=move |ev| label.set(event_target_value(&ev))
                />
            </label>

            <label class="form-field span-2">
                <span>"URL base (opcional)"</span>
                <input
                    type="text"
                    placeholder="ex.: https://api.x.ai/v1 para gateways OpenAI-compatíveis"
                    prop:value=move || base_url.get()
                    on:input=move |ev| base_url.set(event_target_value(&ev))
                />
                {move || {
                    error_for("base_url").map(|m| view! { <span class="field-error">{m}</span> })
                }}
            </label>

            <label class="form-field span-2">
                <span>{if has_saved_key {
                    "Chave de API (salva — preencha para substituir)"
                } else {
                    "Chave de API *"
                }}</span>
                <input
                    type="password"
                    autocomplete="off"
                    placeholder="Guardada no armazenamento seguro do sistema, nunca no banco"
                    prop:value=move || api_key.get()
                    on:input=move |ev| api_key.set(event_target_value(&ev))
                />
            </label>

            <div class="form-field span-2">
                <span>"Modelo padrão"</span>
                <div class="model-row">
                    {move || {
                        let list = models.get();
                        if list.is_empty() {
                            view! {
                                <input
                                    type="text"
                                    placeholder="ex.: claude-sonnet-5 (ou busque a lista ao lado)"
                                    prop:value=move || model.get()
                                    on:input=move |ev| model.set(event_target_value(&ev))
                                />
                            }
                                .into_any()
                        } else {
                            let current = model.get();
                            let known = list.iter().any(|m| m.model_id == current);
                            view! {
                                <select on:change=move |ev| model.set(event_target_value(&ev))>
                                    <option value="" selected=current.is_empty()>
                                        "— escolher na hora de usar —"
                                    </option>
                                    {(!known && !current.is_empty())
                                        .then(|| {
                                            view! {
                                                <option value=current.clone() selected=true>
                                                    {current.clone()}
                                                </option>
                                            }
                                        })}
                                    {list
                                        .into_iter()
                                        .map(|m| {
                                            let is_selected = m.model_id == current;
                                            let price = m
                                                .price_per_mtok()
                                                .map(|p| format!(" — {}", format_price(p)))
                                                .unwrap_or_default();
                                            let text = format!("{}{}", m.display(), price);
                                            view! {
                                                <option value=m.model_id.clone() selected=is_selected>
                                                    {text}
                                                </option>
                                            }
                                        })
                                        .collect_view()}
                                </select>
                            }
                                .into_any()
                        }
                    }}
                    {is_edit
                        .then(|| {
                            view! {
                                <button
                                    class="btn btn-sm"
                                    disabled=move || refreshing.get()
                                    on:click=refresh_models
                                >
                                    {move || {
                                        if refreshing.get() { "Buscando…" } else { "Atualizar modelos" }
                                    }}
                                </button>
                            }
                        })}
                </div>
                {move || {
                    let current = model.get();
                    models
                        .get()
                        .iter()
                        .find(|m| m.model_id == current)
                        .and_then(|m| m.price_per_mtok())
                        .map(|p| {
                            view! {
                                <span class="model-price">
                                    {format!("Custo: {} (entrada / saída)", format_price(p))}
                                </span>
                            }
                        })
                }}
                {(!is_edit)
                    .then(|| {
                        view! {
                            <span class="hint">
                                "Salve o provedor para buscar a lista de modelos e os preços."
                            </span>
                        }
                    })}
            </div>
        </div>

        <div class="modal-actions">
            <button class="btn" on:click=move |_| on_done.run(())>
                {if is_edit { "Voltar" } else { "Cancelar" }}
            </button>
            <button class="btn btn-primary" disabled=move || saving.get() on:click=submit>
                {move || if saving.get() { "Salvando…" } else { "Salvar" }}
            </button>
        </div>
    }
}
