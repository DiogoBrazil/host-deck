//! Laço agêntico: alterna turnos do provedor com execução de ferramentas
//! até o modelo responder sem pedir ferramenta (ou estourar o budget).

use tokio_util::sync::CancellationToken;

use super::domain::{AgentMessage, ToolCall, ToolResult, TurnOutcome, TurnRequest};
use super::events::{AgentEvent, EventSink};
use super::provider::AgentProvider;
use super::tools::ToolExecutor;
use crate::error::{AppError, AppResult};

/// Idas ao provedor permitidas num único `agent_send`; evita que um modelo
/// preso num ciclo de ferramentas queime tokens indefinidamente.
pub const MAX_TURNS: usize = 16;

/// Parâmetros fixos ao longo de um `agent_send`.
pub struct LoopConfig {
    pub model: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub max_turns: usize,
}

/// Como o laço terminou. Cancelamento não é erro: o histórico fica
/// consistente (todo turno de assistente com `tool_calls` recebe os
/// `ToolResults` correspondentes, ainda que sintéticos).
#[derive(Debug)]
pub enum LoopEnd {
    /// Resposta final do modelo.
    Completed(String),
    /// Cancelado pelo usuário via `agent_cancel`.
    Cancelled,
}

/// Executa o laço, mutando `history` conforme avança — quem chama persiste o
/// que sobrou no registry mesmo em cancelamento ou erro.
pub async fn run_loop(
    provider: &dyn AgentProvider,
    tools: &dyn ToolExecutor,
    config: &LoopConfig,
    history: &mut Vec<AgentMessage>,
    events: EventSink<'_>,
    token: &CancellationToken,
) -> AppResult<LoopEnd> {
    for _ in 0..config.max_turns {
        if token.is_cancelled() {
            return Ok(LoopEnd::Cancelled);
        }

        let request = TurnRequest {
            model: config.model.clone(),
            system: config.system.clone(),
            messages: history.clone(),
            tools: tools.specs(),
            max_tokens: config.max_tokens,
        };

        let sink = |delta| events(AgentEvent::Delta(delta));
        let outcome = tokio::select! {
            _ = token.cancelled() => return Ok(LoopEnd::Cancelled),
            result = provider.turn(&request, &sink) => {
                result.map_err(AppError::from)?
            }
        };

        match outcome {
            TurnOutcome::Text(turn) => {
                let text = turn.text.clone();
                history.push(AgentMessage::Assistant(turn));
                return Ok(LoopEnd::Completed(text));
            }
            TurnOutcome::ToolCalls(turn) => {
                let calls = turn.tool_calls.clone();
                history.push(AgentMessage::Assistant(turn));

                let mut results = Vec::with_capacity(calls.len());
                let mut cancelled = false;
                for call in &calls {
                    if cancelled || token.is_cancelled() {
                        // Preenche o restante para o histórico ecoável ao
                        // provedor continuar válido (tool_use sem tool_result
                        // é rejeitado no turno seguinte).
                        cancelled = true;
                        results.push(cancelled_result(call));
                        continue;
                    }
                    events(AgentEvent::ToolUse {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    });
                    let result = tokio::select! {
                        _ = token.cancelled() => {
                            cancelled = true;
                            cancelled_result(call)
                        }
                        result = tools.execute(call) => result,
                    };
                    results.push(result);
                }
                history.push(AgentMessage::ToolResults(results));
                if cancelled {
                    return Ok(LoopEnd::Cancelled);
                }
            }
        }
    }

    Err(AppError::Agent(format!(
        "O agente atingiu o limite de {} turnos sem concluir.",
        config.max_turns
    )))
}

fn cancelled_result(call: &ToolCall) -> ToolResult {
    ToolResult {
        call_id: call.id.clone(),
        content: "Cancelled by the user before execution.".into(),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::agent::domain::{AgentTurn, StreamDelta, ToolSpec};
    use crate::agent::provider::mock::{MockProvider, ScriptedTurn};

    /// Executor roteirizado: devolve `ran <command>` (ou recusa, se `decline`).
    struct FakeTools {
        decline: bool,
        executed: Mutex<Vec<String>>,
    }

    impl FakeTools {
        fn new(decline: bool) -> Self {
            Self {
                decline,
                executed: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ToolExecutor for FakeTools {
        fn specs(&self) -> Vec<ToolSpec> {
            vec![ToolSpec {
                name: "run_command".into(),
                description: "test".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }]
        }

        async fn execute(&self, call: &ToolCall) -> ToolResult {
            let command = call.arguments["command"].as_str().unwrap_or("?").to_string();
            self.executed.lock().unwrap().push(command.clone());
            if self.decline {
                ToolResult {
                    call_id: call.id.clone(),
                    content: "The user declined to run this command.".into(),
                    is_error: true,
                }
            } else {
                ToolResult {
                    call_id: call.id.clone(),
                    content: format!("ran {command}"),
                    is_error: false,
                }
            }
        }
    }

    fn config() -> LoopConfig {
        LoopConfig {
            model: "mock-model".into(),
            system: Some("system".into()),
            max_tokens: 1024,
            max_turns: MAX_TURNS,
        }
    }

    fn text_turn(text: &str) -> ScriptedTurn {
        ScriptedTurn {
            deltas: vec![StreamDelta::Text(text.into())],
            outcome: TurnOutcome::Text(AgentTurn {
                text: text.into(),
                tool_calls: vec![],
            }),
        }
    }

    fn tool_turn(command: &str) -> ScriptedTurn {
        ScriptedTurn {
            deltas: vec![],
            outcome: TurnOutcome::ToolCalls(AgentTurn {
                text: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"command": command}),
                }],
            }),
        }
    }

    #[tokio::test]
    async fn completes_on_text_only_turn() {
        let provider = MockProvider::new("prov-1");
        provider.push_turn(text_turn("pronto"));
        let tools = FakeTools::new(false);
        let mut history = vec![AgentMessage::User("oi".into())];
        let events: Mutex<Vec<AgentEvent>> = Mutex::new(Vec::new());
        let sink = |ev: AgentEvent| events.lock().unwrap().push(ev);

        let end = run_loop(
            &provider,
            &tools,
            &config(),
            &mut history,
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(matches!(end, LoopEnd::Completed(t) if t == "pronto"));
        assert_eq!(history.len(), 2);
        assert!(matches!(&history[1], AgentMessage::Assistant(t) if t.text == "pronto"));
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [AgentEvent::Delta(StreamDelta::Text(t))] if t == "pronto"
        ));
    }

    #[tokio::test]
    async fn executes_tools_and_feeds_results_back() {
        let provider = MockProvider::new("prov-1");
        provider.push_turn(tool_turn("uptime"));
        provider.push_turn(text_turn("carga ok"));
        let tools = FakeTools::new(false);
        let mut history = vec![AgentMessage::User("como está a carga?".into())];
        let events: Mutex<Vec<AgentEvent>> = Mutex::new(Vec::new());
        let sink = |ev: AgentEvent| events.lock().unwrap().push(ev);

        let end = run_loop(
            &provider,
            &tools,
            &config(),
            &mut history,
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(matches!(end, LoopEnd::Completed(t) if t == "carga ok"));
        assert_eq!(*tools.executed.lock().unwrap(), vec!["uptime".to_string()]);

        // User, Assistant(tool_calls), ToolResults, Assistant(texto).
        assert_eq!(history.len(), 4);
        match &history[2] {
            AgentMessage::ToolResults(results) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].call_id, "c1");
                assert_eq!(results[0].content, "ran uptime");
                assert!(!results[0].is_error);
            }
            other => panic!("esperava ToolResults, veio {other:?}"),
        }

        // O segundo request ao provedor deve ecoar o histórico completo.
        let seen = provider.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].messages.len(), 3);
        assert_eq!(seen[1].tools.len(), 1);

        // ToolUse foi emitido antes da execução.
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, AgentEvent::ToolUse { call_id, .. } if call_id == "c1")));
    }

    #[tokio::test]
    async fn declined_tool_still_returns_to_the_model() {
        let provider = MockProvider::new("prov-1");
        provider.push_turn(tool_turn("rm -rf build"));
        provider.push_turn(text_turn("sem problemas, não executei"));
        let tools = FakeTools::new(true);
        let mut history = vec![AgentMessage::User("limpa o build".into())];
        let sink = |_ev: AgentEvent| {};

        let end = run_loop(
            &provider,
            &tools,
            &config(),
            &mut history,
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        // A recusa vira ToolResult de erro e o modelo responde na sequência.
        assert!(matches!(end, LoopEnd::Completed(_)));
        match &history[2] {
            AgentMessage::ToolResults(results) => assert!(results[0].is_error),
            other => panic!("esperava ToolResults, veio {other:?}"),
        }
    }

    #[tokio::test]
    async fn stops_at_turn_budget() {
        let provider = MockProvider::new("prov-1");
        provider.push_turn(tool_turn("uptime"));
        provider.push_turn(tool_turn("df -h"));
        let tools = FakeTools::new(false);
        let mut history = vec![AgentMessage::User("investiga".into())];
        let sink = |_ev: AgentEvent| {};

        let mut cfg = config();
        cfg.max_turns = 2;
        let err = run_loop(
            &provider,
            &tools,
            &cfg,
            &mut history,
            &sink,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Agent(m) if m.contains("limite de 2 turnos")));
        // O histórico permanece consistente: cada tool_call tem seu resultado.
        assert_eq!(history.len(), 5);
    }

    #[tokio::test]
    async fn cancelled_before_start_calls_no_provider() {
        let provider = MockProvider::new("prov-1");
        let tools = FakeTools::new(false);
        let mut history = vec![AgentMessage::User("oi".into())];
        let sink = |_ev: AgentEvent| {};
        let token = CancellationToken::new();
        token.cancel();

        let end = run_loop(&provider, &tools, &config(), &mut history, &sink, &token)
            .await
            .unwrap();

        assert!(matches!(end, LoopEnd::Cancelled));
        assert!(provider.seen.lock().unwrap().is_empty());
        assert_eq!(history.len(), 1);
    }
}
