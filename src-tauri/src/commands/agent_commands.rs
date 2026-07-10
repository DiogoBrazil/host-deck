use std::sync::Arc;

use tauri::State;
use tauri::ipc::Channel;

use crate::agent::build_provider;
use crate::agent::domain::AgentMessage;
use crate::agent::events::{AgentEvent, SharedEventSink};
use crate::agent::models::refresh_model_cache;
use crate::agent::r#loop::{LoopConfig, LoopEnd, MAX_TURNS, run_loop};
use crate::agent::registry::AgentRegistry;
use crate::agent::tools::SessionToolbox;
use crate::domain::{AgentProvider as ProviderRecord, ModelCacheEntry};
use crate::error::{AppError, AppResult};
use crate::infra::agent_repository;
use crate::infra::credential_store::CredentialStore;
use crate::infra::db::Db;
use crate::ssh::registry::SessionRegistry;
use crate::state::CredStore;

/// Saída pedida quando o modelo não anuncia `max_output_tokens`.
const MAX_TOKENS_DEFAULT: u32 = 4096;
/// Teto de saída mesmo quando o modelo suporta mais; segura o custo por turno.
const MAX_TOKENS_CAP: i64 = 8192;
/// Quanto do fim do scrollback vai como contexto ao modelo.
///
/// Fase 4 adiciona redação de segredos e consentimento antes deste envio.
const SCROLLBACK_CONTEXT_BYTES: usize = 24 * 1024;

/// Envia uma mensagem do usuário ao agente da sessão.
///
/// Retorna imediatamente; o turno roda em background e os eventos (tokens,
/// pedidos de ferramenta, confirmações, desfecho) chegam por `on_event`.
#[tauri::command]
pub async fn agent_send(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    sessions: State<'_, SessionRegistry>,
    agents: State<'_, AgentRegistry>,
    session_id: String,
    provider_id: String,
    model: Option<String>,
    message: String,
    on_event: Channel<AgentEvent>,
) -> AppResult<()> {
    let (record, cached_models) = {
        let conn = db.0.lock().unwrap();
        let record = agent_repository::get(&conn, &provider_id)?;
        let cached = agent_repository::list_model_cache(&conn, &provider_id)?;
        (record, cached)
    };

    let api_key = resolve_api_key(&record, store.0.as_ref())?;
    let model = model
        .or_else(|| record.model.clone())
        .ok_or_else(|| AppError::Agent("Nenhum modelo definido para este provedor.".into()))?;

    // A sessão precisa estar viva antes de gastar tokens.
    let scrollback = sessions
        .scrollback_snapshot(&session_id)
        .ok_or(AppError::NotFound)?;

    let token = agents.begin_turn(&session_id)?;
    let provider = build_provider(&record, api_key);

    let config = LoopConfig {
        max_tokens: max_tokens_for(&cached_models, &model),
        model,
        system: Some(build_system_prompt(&scrollback)),
        max_turns: MAX_TURNS,
    };

    let mut history = agents.history_snapshot(&session_id);
    history.push(AgentMessage::User(message));

    let emit: SharedEventSink = {
        let channel = on_event.clone();
        Arc::new(move |ev: AgentEvent| {
            let _ = channel.send(ev);
        })
    };
    let toolbox = SessionToolbox::new(
        sessions.inner().clone(),
        agents.inner().clone(),
        session_id.clone(),
        emit.clone(),
    );
    let agents = agents.inner().clone();

    tauri::async_runtime::spawn(async move {
        let end = run_loop(
            provider.as_ref(),
            &toolbox,
            &config,
            &mut history,
            emit.as_ref(),
            &token,
        )
        .await;

        // Persiste também históricos parciais (cancelamento, erro no meio).
        agents.replace_history(&session_id, history);
        agents.finish_turn(&session_id);

        match end {
            Ok(LoopEnd::Completed(text)) => emit(AgentEvent::Done { text }),
            Ok(LoopEnd::Cancelled) => emit(AgentEvent::Error {
                message: "Turno cancelado.".into(),
            }),
            Err(err) => {
                log::warn!("[{session_id}] turno do agente falhou: {err}");
                emit(AgentEvent::Error {
                    message: err.to_string(),
                });
            }
        }
    });

    Ok(())
}

/// Cancela o turno em andamento; confirmações pendentes são recusadas.
#[tauri::command]
pub async fn agent_cancel(
    agents: State<'_, AgentRegistry>,
    session_id: String,
) -> AppResult<()> {
    agents.cancel_turn(&session_id);
    Ok(())
}

/// Encaminha a decisão do usuário sobre um comando proposto pelo agente
/// (espelha `confirm_host_key`).
#[tauri::command]
pub async fn confirm_agent_command(
    agents: State<'_, AgentRegistry>,
    session_id: String,
    call_id: String,
    accept: bool,
) -> AppResult<()> {
    if let Some(tx) = agents.take_confirmation(&session_id, &call_id) {
        let _ = tx.send(accept);
    }
    Ok(())
}

/// Busca a listagem de modelos no provedor e substitui o cache persistido.
#[tauri::command]
pub async fn agent_refresh_models(
    db: State<'_, Db>,
    store: State<'_, CredStore>,
    provider_id: String,
) -> AppResult<Vec<ModelCacheEntry>> {
    let record = {
        let conn = db.0.lock().unwrap();
        agent_repository::get(&conn, &provider_id)?
    };
    let api_key = resolve_api_key(&record, store.0.as_ref())?;
    let provider = build_provider(&record, api_key);
    refresh_model_cache(&db.0, provider.as_ref()).await
}

/// Resolve a chave de API no keyring; nunca há fallback para o banco.
fn resolve_api_key(record: &ProviderRecord, store: &dyn CredentialStore) -> AppResult<String> {
    let key_ref = record.api_key_ref.as_deref().ok_or_else(|| {
        AppError::Agent("O provedor ainda não tem chave de API cadastrada.".into())
    })?;
    store.get(key_ref)?.ok_or_else(|| {
        AppError::Agent("Chave de API não encontrada no armazenamento seguro.".into())
    })
}

/// `max_tokens` do turno: o anunciado pelo modelo, limitado pelo teto local.
fn max_tokens_for(cache: &[ModelCacheEntry], model: &str) -> u32 {
    cache
        .iter()
        .find(|m| m.model_id == model)
        .and_then(|m| m.max_output_tokens)
        .map(|max| max.clamp(256, MAX_TOKENS_CAP) as u32)
        .unwrap_or(MAX_TOKENS_DEFAULT)
}

fn build_system_prompt(scrollback: &[u8]) -> String {
    const PREAMBLE: &str = "You are an assistant embedded in an SSH terminal client. You help \
        the user inspect and operate the remote server of the current session. Investigate \
        with the available tools before answering: prefer run_command for inspecting the \
        system, read_remote_file for file contents, and type_into_terminal only when the \
        user's interactive context (current directory, environment, sudo state) matters. \
        Commands that change state require the user's confirmation and may be declined. Be \
        concise and answer in the user's language.";

    let start = scrollback.len().saturating_sub(SCROLLBACK_CONTEXT_BYTES);
    let tail = strip_ansi(&String::from_utf8_lossy(&scrollback[start..]));
    let tail = tail.trim();
    if tail.is_empty() {
        PREAMBLE.to_string()
    } else {
        format!(
            "{PREAMBLE}\n\nCurrent contents of the user's terminal (most recent output, \
             possibly truncated):\n<terminal>\n{tail}\n</terminal>"
        )
    }
}

/// Remove sequências ANSI (CSI/OSC) e controles do scrollback antes de
/// mandá-lo ao modelo — cor e reposicionamento de cursor só gastam tokens.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parâmetros até um byte final em @..~.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC: até BEL ou ST (ESC \).
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                // Demais escapes de dois bytes são descartados junto.
                _ => {}
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model_id: &str, max_output_tokens: Option<i64>) -> ModelCacheEntry {
        ModelCacheEntry {
            provider_id: "prov-1".into(),
            model_id: model_id.into(),
            display_name: None,
            max_input_tokens: None,
            max_output_tokens,
            capabilities: "{}".into(),
            fetched_at: "2026-07-10T00:00:00Z".into(),
        }
    }

    #[test]
    fn strip_ansi_removes_csi_and_osc_but_keeps_text() {
        let raw = "\u{1b}]0;title\u{7}\u{1b}[1;32muser@host\u{1b}[0m:~$ ls\r\ntotal 0\n";
        assert_eq!(strip_ansi(raw), "user@host:~$ ls\ntotal 0\n");
    }

    #[test]
    fn system_prompt_embeds_scrollback_tail() {
        let prompt = build_system_prompt(b"$ uptime\r\n 10:00 up 3 days\r\n");
        assert!(prompt.contains("<terminal>"), "{prompt}");
        assert!(prompt.contains("up 3 days"), "{prompt}");
        assert!(!prompt.contains('\r'), "{prompt}");

        // Sem scrollback, sem bloco de terminal.
        let prompt = build_system_prompt(b"");
        assert!(!prompt.contains("<terminal>"), "{prompt}");
    }

    #[test]
    fn system_prompt_takes_only_the_tail_of_large_scrollbacks() {
        let mut scrollback = vec![b'a'; 100 * 1024];
        let marker = b"MARKER-AT-THE-END";
        scrollback.extend_from_slice(marker);
        let prompt = build_system_prompt(&scrollback);
        assert!(prompt.contains("MARKER-AT-THE-END"));
        assert!(prompt.len() < 30 * 1024, "len={}", prompt.len());
    }

    #[test]
    fn max_tokens_respects_model_announcement_and_cap() {
        let cache = vec![
            entry("small", Some(2048)),
            entry("big", Some(128_000)),
            entry("unknown-limit", None),
        ];
        assert_eq!(max_tokens_for(&cache, "small"), 2048);
        assert_eq!(max_tokens_for(&cache, "big"), MAX_TOKENS_CAP as u32);
        assert_eq!(max_tokens_for(&cache, "unknown-limit"), MAX_TOKENS_DEFAULT);
        assert_eq!(max_tokens_for(&cache, "absent"), MAX_TOKENS_DEFAULT);
    }
}
