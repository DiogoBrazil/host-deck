//! Eventos do agente enviados à UI pelo `tauri::ipc::Channel`.

use serde::Serialize;

use super::domain::StreamDelta;

/// Events streamed to the frontend during an agent turn.
///
/// Mirrors `TerminalEvent`'s serialization convention (`event` tag, `data`
/// content, `camelCase`).
#[derive(Clone, Serialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    /// Fragmento incremental de texto/pensamento do modelo.
    Delta(StreamDelta),
    /// O modelo pediu a execução de uma ferramenta.
    ToolUse {
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Execução aguardando decisão do usuário; respondida por
    /// `confirm_agent_command` (espelha o fluxo do `HostKeyPrompt`).
    CommandPrompt {
        call_id: String,
        tool: String,
        command: String,
    },
    /// Turno concluído; `text` é a resposta final do modelo.
    Done { text: String },
    /// Falha ou cancelamento; encerra o turno.
    Error { message: String },
}

/// Callback emprestado que recebe os eventos de um turno; o laço e os testes
/// usam closures, o comando embrulha o `Channel<AgentEvent>`.
pub type EventSink<'a> = &'a (dyn Fn(AgentEvent) + Send + Sync);

/// Variante possuída do sink, para tasks e para o toolbox.
pub type SharedEventSink = std::sync::Arc<dyn Fn(AgentEvent) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_variant_and_fields_as_camel_case() {
        // The frontend depends on these exact serialized names.
        let delta = AgentEvent::Delta(StreamDelta::Text("oi".into()));
        let json = serde_json::to_string(&delta).unwrap();
        assert!(json.contains("\"event\":\"delta\""), "json={json}");
        assert!(json.contains("\"type\":\"text\""), "json={json}");

        let tool = AgentEvent::ToolUse {
            call_id: "c1".into(),
            name: "run_command".into(),
            arguments: serde_json::json!({"command": "uptime"}),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("\"event\":\"toolUse\""), "json={json}");
        assert!(json.contains("\"callId\":\"c1\""), "json={json}");
        assert!(json.contains("\"command\":\"uptime\""), "json={json}");

        let prompt = AgentEvent::CommandPrompt {
            call_id: "c1".into(),
            tool: "run_command".into(),
            command: "rm -r build".into(),
        };
        let json = serde_json::to_string(&prompt).unwrap();
        assert!(json.contains("\"event\":\"commandPrompt\""), "json={json}");
        assert!(json.contains("\"callId\":\"c1\""), "json={json}");

        let done = AgentEvent::Done { text: "fim".into() };
        let json = serde_json::to_string(&done).unwrap();
        assert!(json.contains("\"event\":\"done\""), "json={json}");
    }
}
