use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::bindings::agent::agent_send;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::models::{
    AgentEvent, AgentProvider, ModelCacheEntry, ProviderKind, SshConnection, StreamDelta,
};

/// Options of the temperature select; empty string leaves the parameter out.
const TEMPERATURE_CHOICES: [&str; 6] = ["", "0.0", "0.2", "0.5", "0.7", "1.0"];

/// One rendered entry of the conversation.
#[derive(Clone, PartialEq)]
enum ChatItem {
    User(String),
    Assistant { text: String, thinking: String },
    Tool { label: String },
    Error(String),
}

/// Pending `CommandPrompt` waiting for the user's decision.
#[derive(Clone, PartialEq)]
struct PendingCommand {
    call_id: String,
    tool: String,
    command: String,
}

/// Short human-readable description of a tool call for the transcript.
fn tool_label(name: &str, arguments: &serde_json::Value) -> String {
    let arg = |key: &str| arguments.get(key).and_then(|v| v.as_str()).unwrap_or("?");
    match name {
        "run_command" => format!("$ {}", arg("command")),
        "read_remote_file" => format!("lendo {}", arg("path")),
        "type_into_terminal" => format!("digitando no terminal: {}", arg("text")),
        other => other.to_string(),
    }
}

/// Chat do agente de IA, ancorado à sessão SSH do terminal ao lado.
#[component]
pub fn AgentPanel(
    connection: SshConnection,
    session_id: Signal<Option<String>>,
    on_close: Callback<()>,
) -> impl IntoView {
    let providers = RwSignal::new(Vec::<AgentProvider>::new());
    let models = RwSignal::new(Vec::<ModelCacheEntry>::new());
    let selected_provider = RwSignal::new(Option::<String>::None);
    // Empty string means "use the provider's default model".
    let selected_model = RwSignal::new(String::new());
    let messages_ref = NodeRef::<leptos::html::Div>::new();
    let items = RwSignal::new(Vec::<ChatItem>::new());
    let input = RwSignal::new(String::new());
    let running = RwSignal::new(false);
    let pending = RwSignal::new(Option::<PendingCommand>::None);
    // Consentimento de envio do terminal; Some(preview) = modal aberto.
    let consent = RwSignal::new(false);
    let consent_preview = RwSignal::new(Option::<String>::None);
    // Controles por capacidade; string vazia = temperatura padrão do modelo.
    let temperature = RwSignal::new(String::new());
    let thinking = RwSignal::new(false);

    let connection_id = StoredValue::new(connection.id.clone());
    let initial_provider = StoredValue::new(connection.provider_id.clone());

    // Provedores cadastrados; a seleção inicial vem do vínculo da conexão.
    spawn_local(async move {
        match api::list_providers().await {
            Ok(list) => {
                let seed = initial_provider
                    .get_value()
                    .filter(|id| list.iter().any(|p| p.id == *id))
                    .or_else(|| list.first().map(|p| p.id.clone()));
                providers.set(list);
                selected_provider.set(seed);
            }
            Err(err) => items.update(|it| it.push(ChatItem::Error(err.message()))),
        }
    });

    // O consentimento é dado uma única vez e vale entre sessões.
    spawn_local(async move {
        if let Ok(granted) = api::get_agent_consent().await {
            consent.set(granted);
        }
    });

    // Modelos em cache do provedor selecionado (dropdown; renovação fica na
    // tela de provedores).
    Effect::new(move |_| {
        let Some(provider_id) = selected_provider.get() else {
            models.set(Vec::new());
            return;
        };
        selected_model.set(String::new());
        temperature.set(String::new());
        thinking.set(false);
        spawn_local(async move {
            if let Ok(list) = api::list_cached_models(&provider_id).await {
                models.set(list);
            }
        });
    });

    let apply_event = move |event: AgentEvent| match event {
        AgentEvent::Delta(StreamDelta::Text(chunk)) => items.update(|it| {
            if let Some(ChatItem::Assistant { text, .. }) = it.last_mut() {
                text.push_str(&chunk);
            }
        }),
        AgentEvent::Delta(StreamDelta::Thinking(chunk)) => items.update(|it| {
            if let Some(ChatItem::Assistant { thinking, .. }) = it.last_mut() {
                thinking.push_str(&chunk);
            }
        }),
        AgentEvent::ToolUse {
            name, arguments, ..
        } => items.update(|it| {
            let label = tool_label(&name, &arguments);
            // O texto até aqui pertence ao turno que pediu a ferramenta; o
            // que vier depois começa um balão novo.
            it.push(ChatItem::Tool { label });
            it.push(ChatItem::Assistant {
                text: String::new(),
                thinking: String::new(),
            });
        }),
        AgentEvent::CommandPrompt {
            call_id,
            tool,
            command,
        } => pending.set(Some(PendingCommand {
            call_id,
            tool,
            command,
        })),
        AgentEvent::Done { .. } => {
            running.set(false);
            pending.set(None);
            // Remove balões vazios deixados por ferramentas no fim do turno.
            items.update(|it| {
                it.retain(|item| {
                    !matches!(
                        item,
                        ChatItem::Assistant { text, thinking }
                            if text.is_empty() && thinking.is_empty()
                    )
                });
            });
        }
        AgentEvent::Error { message } => {
            running.set(false);
            pending.set(None);
            items.update(|it| {
                it.retain(|item| {
                    !matches!(
                        item,
                        ChatItem::Assistant { text, thinking }
                            if text.is_empty() && thinking.is_empty()
                    )
                });
                it.push(ChatItem::Error(message));
            });
        }
    };

    // Modelo efetivo (seleção ou o padrão do provedor) e o que ele anuncia
    // suportar — os controles vêm das capacidades, nunca de lista fixa.
    let provider_kind = move || -> Option<ProviderKind> {
        let id = selected_provider.get()?;
        providers.get().iter().find(|p| p.id == id).map(|p| p.kind)
    };
    let effective_model = move || -> Option<String> {
        let chosen = selected_model.get();
        if !chosen.is_empty() {
            return Some(chosen);
        }
        let id = selected_provider.get()?;
        providers
            .get()
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.model.clone())
    };
    let model_entry = move || -> Option<ModelCacheEntry> {
        let id = effective_model()?;
        models.get().into_iter().find(|m| m.model_id == id)
    };
    let supports_temperature = move || {
        matches!((model_entry(), provider_kind()),
            (Some(entry), Some(kind)) if entry.supports_temperature(kind))
    };
    let supports_thinking = move || {
        matches!((model_entry(), provider_kind()),
            (Some(entry), Some(kind)) if entry.supports_thinking(kind))
    };

    // Envio de fato; só roda depois das validações e do consentimento.
    let do_send = move |message: String| {
        let Some(session) = session_id.get() else { return };
        let Some(provider_id) = selected_provider.get() else {
            return;
        };

        input.set(String::new());
        running.set(true);
        items.update(|it| {
            it.push(ChatItem::User(message.clone()));
            it.push(ChatItem::Assistant {
                text: String::new(),
                thinking: String::new(),
            });
        });

        let model = {
            let m = selected_model.get();
            (!m.is_empty()).then_some(m)
        };
        // Recheca o suporte na hora do envio: valor escolhido para um modelo
        // não vale para outro que não anuncia o parâmetro.
        let temperature = supports_temperature()
            .then(|| temperature.get().parse::<f64>().ok())
            .flatten();
        let thinking = thinking.get() && supports_thinking();

        let on_event = Closure::<dyn FnMut(String)>::new(move |json: String| {
            match serde_json::from_str::<AgentEvent>(&json) {
                Ok(event) => apply_event(event),
                Err(err) => leptos::logging::warn!("evento do agente ilegível: {err}: {json}"),
            }
        });
        // O Channel do lado JS retém o callback pela duração do turno.
        let on_event = on_event.into_js_value();

        spawn_local(async move {
            if let Err(err) = agent_send(
                &session,
                &provider_id,
                model,
                &message,
                temperature,
                thinking,
                &on_event,
            )
            .await
            {
                running.set(false);
                let message = crate::models::AppError::from_js(err).message();
                items.update(|it| it.push(ChatItem::Error(message)));
            }
        });
    };

    let send = move || {
        if running.get() {
            return;
        }
        let message = input.get().trim().to_string();
        if message.is_empty() {
            return;
        }
        let Some(session) = session_id.get() else {
            items.update(|it| {
                it.push(ChatItem::Error(
                    "A sessão SSH ainda não está pronta.".into(),
                ))
            });
            return;
        };
        if selected_provider.get().is_none() {
            items.update(|it| {
                it.push(ChatItem::Error(
                    "Cadastre um provedor de IA em “Provedores de IA” na barra lateral.".into(),
                ))
            });
            return;
        }

        // Primeira saída de dados da máquina: mostra exatamente o que vai ao
        // provedor e espera a autorização. A mensagem permanece no input.
        if !consent.get() {
            spawn_local(async move {
                match api::agent_context_preview(&session).await {
                    Ok(preview) => consent_preview.set(Some(preview)),
                    Err(err) => items.update(|it| it.push(ChatItem::Error(err.message()))),
                }
            });
            return;
        }

        do_send(message);
    };
    let send_click = send;

    let grant_consent = move |_| {
        spawn_local(async move {
            // Persiste antes de enviar: o backend recusa turnos sem o
            // consentimento gravado.
            if let Err(err) = api::set_agent_consent(true).await {
                items.update(|it| it.push(ChatItem::Error(err.message())));
                return;
            }
            consent.set(true);
            consent_preview.set(None);
            let message = input.get().trim().to_string();
            if !message.is_empty() {
                do_send(message);
            }
        });
    };
    let refuse_consent = move |_| consent_preview.set(None);

    let cancel = move |_| {
        if let Some(session) = session_id.get() {
            spawn_local(async move {
                let _ = api::agent_cancel(&session).await;
            });
        }
    };

    let change_provider = move |ev: leptos::ev::Event| {
        let value = event_target_value(&ev);
        let value = (!value.is_empty()).then_some(value);
        selected_provider.set(value.clone());
        // Persiste o vínculo para a próxima sessão deste servidor.
        let conn_id = connection_id.get_value();
        spawn_local(async move {
            let _ = api::set_connection_provider(&conn_id, value.as_deref()).await;
        });
    };

    let decide = move |accept: bool| {
        let Some(cmd) = pending.get() else { return };
        pending.set(None);
        if let Some(session) = session_id.get() {
            spawn_local(async move {
                let _ = api::confirm_agent_command(&session, &cmd.call_id, accept).await;
            });
        }
    };
    let decide_yes = decide;
    let decide_no = decide;

    // Acompanha o streaming: qualquer mudança na conversa rola para o fim.
    Effect::new(move |_| {
        items.track();
        if let Some(el) = messages_ref.get() {
            el.set_scroll_top(el.scroll_height());
        }
    });

    view! {
        <div class="agent-panel">
            <div class="agent-header">
                <span class="agent-title">"Agente"</span>
                <select
                    class="agent-select"
                    on:change=change_provider
                    disabled=move || running.get()
                >
                    {move || {
                        let selected = selected_provider.get();
                        providers
                            .get()
                            .into_iter()
                            .map(|p| {
                                let is_selected = selected.as_deref() == Some(p.id.as_str());
                                view! {
                                    <option value=p.id.clone() selected=is_selected>
                                        {p.label.clone()}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
                <select
                    class="agent-select"
                    on:change=move |ev| {
                        selected_model.set(event_target_value(&ev));
                        // Controles valem por modelo; troca zera a escolha.
                        temperature.set(String::new());
                        thinking.set(false);
                    }
                    disabled=move || running.get()
                >
                    <option value="" selected=move || selected_model.get().is_empty()>
                        "Modelo padrão"
                    </option>
                    {move || {
                        let current = selected_model.get();
                        models
                            .get()
                            .into_iter()
                            .map(|m| {
                                let is_selected = current == m.model_id;
                                view! {
                                    <option value=m.model_id.clone() selected=is_selected>
                                        {m.display().to_string()}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
                <button class="icon-btn" title="Fechar painel" on:click=move |_| on_close.run(())>
                    "✕"
                </button>
            </div>

            {move || {
                let show_temperature = supports_temperature();
                let show_thinking = supports_thinking();
                (show_temperature || show_thinking).then(|| {
                    view! {
                        <div class="agent-controls">
                            {show_temperature
                                .then(|| {
                                    view! {
                                        <label class="agent-control">
                                            "Temperatura"
                                            <select
                                                class="agent-select"
                                                on:change=move |ev| temperature.set(event_target_value(&ev))
                                                disabled=move || running.get()
                                            >
                                                {TEMPERATURE_CHOICES
                                                    .iter()
                                                    .map(|choice| {
                                                        let value = choice.to_string();
                                                        let label = if choice.is_empty() {
                                                            "Padrão".to_string()
                                                        } else {
                                                            value.clone()
                                                        };
                                                        let is_selected = {
                                                            let value = value.clone();
                                                            move || temperature.get() == value
                                                        };
                                                        view! {
                                                            <option value=value selected=is_selected>
                                                                {label}
                                                            </option>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </select>
                                        </label>
                                    }
                                })}
                            {show_thinking
                                .then(|| {
                                    view! {
                                        <label class="agent-control">
                                            <input
                                                type="checkbox"
                                                prop:checked=move || thinking.get()
                                                on:change=move |ev| thinking.set(event_target_checked(&ev))
                                                disabled=move || running.get()
                                            />
                                            "Raciocínio estendido"
                                        </label>
                                    }
                                })}
                        </div>
                    }
                })
            }}

            <div class="agent-messages" node_ref=messages_ref>
                {move || {
                    let list = items.get();
                    if list.is_empty() {
                        view! {
                            <div class="agent-empty">
                                <p>"Pergunte sobre o servidor desta sessão."</p>
                                <p class="hint">
                                    "O agente lê o terminal, investiga com comandos de leitura e pede confirmação antes de mudar qualquer coisa."
                                </p>
                            </div>
                        }
                            .into_any()
                    } else {
                        list.into_iter()
                            .map(|item| match item {
                                ChatItem::User(text) => {
                                    view! { <div class="agent-msg user">{text}</div> }.into_any()
                                }
                                ChatItem::Assistant { text, thinking } => {
                                    let show_thinking = !thinking.is_empty();
                                    let show_text = !text.is_empty();
                                    let hidden = !show_thinking && !show_text;
                                    view! {
                                        <div class="agent-msg assistant" class:hidden=hidden>
                                            {show_thinking
                                                .then(|| {
                                                    view! { <div class="agent-thinking">{thinking}</div> }
                                                })}
                                            {show_text.then(|| view! { <div>{text}</div> })}
                                        </div>
                                    }
                                        .into_any()
                                }
                                ChatItem::Tool { label } => {
                                    view! { <div class="agent-tool">{label}</div> }.into_any()
                                }
                                ChatItem::Error(message) => {
                                    view! { <div class="agent-msg error">{message}</div> }
                                        .into_any()
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
                {move || {
                    (running.get() && pending.get().is_none())
                        .then(|| view! { <div class="agent-working">"Pensando…"</div> })
                }}
            </div>

            <div class="agent-input-row">
                <textarea
                    class="agent-input"
                    rows="2"
                    placeholder="Pergunte ao agente… (Enter envia, Shift+Enter quebra linha)"
                    prop:value=move || input.get()
                    on:input=move |ev| input.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" && !ev.shift_key() {
                            ev.prevent_default();
                            send();
                        }
                    }
                ></textarea>
                {move || {
                    if running.get() {
                        view! {
                            <button class="btn btn-sm" on:click=cancel>
                                "Cancelar"
                            </button>
                        }
                            .into_any()
                    } else {
                        view! {
                            <button class="btn btn-primary btn-sm" on:click=move |_| send_click()>
                                "Enviar"
                            </button>
                        }
                            .into_any()
                    }
                }}
            </div>

            {move || {
                pending
                    .get()
                    .map(|cmd| {
                        view! {
                            <ConfirmDialog
                                title="O agente quer executar um comando".to_string()
                                message=format!(
                                    "Ferramenta {}:\n\n{}\n\nExecutar no servidor?",
                                    cmd.tool, cmd.command,
                                )
                                confirm_label="Executar".to_string()
                                on_confirm=Callback::new(move |_| decide_yes(true))
                                on_cancel=Callback::new(move |_| decide_no(false))
                            />
                        }
                    })
            }}

            {move || {
                consent_preview
                    .get()
                    .map(|preview| {
                        let preview = if preview.is_empty() {
                            "(terminal vazio — só a sua mensagem seria enviada)".to_string()
                        } else {
                            preview
                        };
                        view! {
                            <div class="modal-backdrop" on:click=refuse_consent>
                                <div class="modal agent-consent" on:click=|ev| ev.stop_propagation()>
                                    <h2>"Enviar o conteúdo do terminal?"</h2>
                                    <p class="confirm-message">
                                        "Para ter contexto, o agente envia a parte recente do terminal ao provedor de IA junto com a conversa. Segredos reconhecidos já foram substituídos por [REDACTED]. Abaixo está exatamente o que será enviado; a autorização fica registrada para as próximas conversas."
                                    </p>
                                    <pre class="agent-consent-preview">{preview}</pre>
                                    <div class="modal-actions">
                                        <button class="btn" on:click=refuse_consent>
                                            "Agora não"
                                        </button>
                                        <button class="btn btn-primary" on:click=grant_consent>
                                            "Autorizar e enviar"
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
