//! Dialeto interno do agente.
//!
//! Anthropic e OpenAI divergem justamente em tool use (`input_schema` vs
//! `function.parameters`, blocos `tool_result` vs mensagens `role: "tool"`,
//! `stop_reason` vs `finish_reason`). Estes tipos são o formato único que o
//! laço agêntico consome; a tradução fica nos adapters.

use serde::{Deserialize, Serialize};

use crate::domain::ModelCacheEntry;

/// Ferramenta exposta ao modelo. `input_schema` é um JSON Schema; cada
/// adapter o coloca no campo esperado pelo provedor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Chamada de ferramenta pedida pelo modelo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Id atribuído pelo provedor; devolvido no `ToolResult` correspondente.
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Resultado da execução de uma `ToolCall`, enviado no turno seguinte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Turno de assistente completo: texto acumulado + chamadas de ferramenta.
///
/// É ecoado de volta ao provedor nas requisições seguintes, então precisa
/// preservar tudo que o modelo produziu.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// Uma entrada do histórico da conversa, no dialeto interno.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessage {
    User(String),
    Assistant(AgentTurn),
    /// Resultados das chamadas do turno de assistente imediatamente anterior.
    ToolResults(Vec<ToolResult>),
}

/// Fragmento incremental do streaming, repassado à UI token a token.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum StreamDelta {
    Text(String),
    Thinking(String),
}

/// Como o turno terminou: resposta final ou pedido de ferramentas.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnOutcome {
    Text(AgentTurn),
    ToolCalls(AgentTurn),
}

impl TurnOutcome {
    /// Classifica pelo conteúdo: qualquer `tool_call` pendente exige execução.
    pub fn from_turn(turn: AgentTurn) -> Self {
        if turn.tool_calls.is_empty() {
            TurnOutcome::Text(turn)
        } else {
            TurnOutcome::ToolCalls(turn)
        }
    }

    /// Acesso uniforme ao turno; hoje só os testes precisam dele.
    #[allow(dead_code)]
    pub fn turn(&self) -> &AgentTurn {
        match self {
            TurnOutcome::Text(t) | TurnOutcome::ToolCalls(t) => t,
        }
    }
}

/// Requisição de um turno do modelo.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<ToolSpec>,
    /// Obrigatório na Anthropic; teto de saída nos demais.
    pub max_tokens: u32,
}

/// Metadados de um modelo, como devolvidos pelo endpoint de listagem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub max_input_tokens: Option<i64>,
    pub max_output_tokens: Option<i64>,
    /// Árvore de capacidades específica do provedor; a UI deriva os
    /// controles daqui, nunca de uma lista fixa.
    pub capabilities: serde_json::Value,
}

impl ModelInfo {
    pub fn into_cache_entry(self, provider_id: &str, fetched_at: String) -> ModelCacheEntry {
        ModelCacheEntry {
            provider_id: provider_id.to_string(),
            model_id: self.id,
            display_name: self.display_name,
            max_input_tokens: self.max_input_tokens,
            max_output_tokens: self.max_output_tokens,
            capabilities: self.capabilities.to_string(),
            fetched_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_classifies_by_pending_tool_calls() {
        let plain = AgentTurn {
            text: "pronto".into(),
            tool_calls: vec![],
        };
        assert!(matches!(
            TurnOutcome::from_turn(plain),
            TurnOutcome::Text(_)
        ));

        let with_call = AgentTurn {
            text: String::new(),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({"command": "uptime"}),
            }],
        };
        assert!(matches!(
            TurnOutcome::from_turn(with_call),
            TurnOutcome::ToolCalls(_)
        ));
    }

    #[test]
    fn model_info_serializes_capabilities_as_json_string() {
        let info = ModelInfo {
            id: "claude-sonnet-5".into(),
            display_name: Some("Claude Sonnet 5".into()),
            max_input_tokens: Some(1_000_000),
            max_output_tokens: Some(128_000),
            capabilities: serde_json::json!({"thinking": {"supported": true}}),
        };
        let entry = info.into_cache_entry("prov-1", "2026-07-10T00:00:00Z".into());
        assert_eq!(entry.provider_id, "prov-1");
        assert_eq!(entry.model_id, "claude-sonnet-5");
        assert_eq!(entry.capabilities, r#"{"thinking":{"supported":true}}"#);
    }
}
