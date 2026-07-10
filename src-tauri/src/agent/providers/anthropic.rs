//! Adapter Anthropic — HTTP puro (não há SDK oficial em Rust).
//!
//! `POST /v1/messages` com streaming SSE (`content_block_delta` etc.) e
//! `GET /v1/models` para a listagem. A tradução do dialeto interno é feita
//! por funções puras (`build_body`, `TurnAccumulator`) para ser testável
//! sem rede.

use std::collections::BTreeMap;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use super::{api_error_message, http_client};
use crate::agent::domain::{
    AgentMessage, AgentTurn, ModelInfo, StreamDelta, ToolCall, TurnOutcome, TurnRequest,
};
use crate::agent::provider::{AgentProvider, DeltaSink, ProviderError};
use crate::agent::sse::{SseEvent, SseParser};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    id: String,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(id: String, base_url: Option<String>, api_key: String) -> Self {
        Self {
            id,
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.into())
                .trim_end_matches('/')
                .to_string(),
            api_key,
            client: http_client(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
    }
}

/// Monta o corpo de `POST /v1/messages` a partir do dialeto interno.
fn build_body(req: &TurnRequest) -> Value {
    let messages: Vec<Value> = req.messages.iter().map(message_to_wire).collect();

    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": messages,
        "stream": true,
    });
    if let Some(system) = &req.system {
        body["system"] = json!(system);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect(),
        );
    }
    body
}

fn message_to_wire(msg: &AgentMessage) -> Value {
    match msg {
        AgentMessage::User(text) => json!({"role": "user", "content": text}),
        AgentMessage::Assistant(turn) => {
            let mut content = Vec::new();
            if !turn.text.is_empty() {
                content.push(json!({"type": "text", "text": turn.text}));
            }
            for call in &turn.tool_calls {
                content.push(json!({
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.name,
                    "input": call.arguments,
                }));
            }
            json!({"role": "assistant", "content": content})
        }
        AgentMessage::ToolResults(results) => {
            let content: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "type": "tool_result",
                        "tool_use_id": r.call_id,
                        "content": r.content,
                        "is_error": r.is_error,
                    })
                })
                .collect();
            json!({"role": "user", "content": content})
        }
    }
}

/// Reduz a sequência de eventos SSE a um `AgentTurn`.
///
/// `tool_use` chega fatiado: o bloco abre com id/nome, o JSON dos argumentos
/// vem em `input_json_delta` e só fecha em `content_block_stop`.
#[derive(Default)]
struct TurnAccumulator {
    text: String,
    tool_calls: Vec<ToolCall>,
    /// Blocos `tool_use` abertos, por índice: (id, nome, JSON parcial).
    open_tools: BTreeMap<u64, (String, String, String)>,
    done: bool,
}

impl TurnAccumulator {
    fn apply(&mut self, event: &SseEvent, sink: DeltaSink<'_>) -> Result<(), ProviderError> {
        if event.data.is_empty() {
            return Ok(());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| ProviderError::Protocol(format!("SSE com JSON inválido: {e}")))?;
        let kind = event
            .event
            .as_deref()
            .or_else(|| data.get("type").and_then(|t| t.as_str()))
            .unwrap_or_default();

        match kind {
            "content_block_start" => {
                let index = data["index"].as_u64().unwrap_or(0);
                let block = &data["content_block"];
                if block["type"] == "tool_use" {
                    self.open_tools.insert(
                        index,
                        (
                            block["id"].as_str().unwrap_or_default().to_string(),
                            block["name"].as_str().unwrap_or_default().to_string(),
                            String::new(),
                        ),
                    );
                }
            }
            "content_block_delta" => {
                let index = data["index"].as_u64().unwrap_or(0);
                let delta = &data["delta"];
                match delta["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        if let Some(text) = delta["text"].as_str() {
                            self.text.push_str(text);
                            sink(StreamDelta::Text(text.to_string()));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta["thinking"].as_str() {
                            sink(StreamDelta::Thinking(text.to_string()));
                        }
                    }
                    "input_json_delta" => {
                        if let (Some(open), Some(part)) = (
                            self.open_tools.get_mut(&index),
                            delta["partial_json"].as_str(),
                        ) {
                            open.2.push_str(part);
                        }
                    }
                    // signature_delta e afins não interessam ao laço.
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = data["index"].as_u64().unwrap_or(0);
                if let Some((id, name, buf)) = self.open_tools.remove(&index) {
                    let arguments = if buf.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&buf).map_err(|e| {
                            ProviderError::Protocol(format!(
                                "argumentos de tool_use inválidos: {e}"
                            ))
                        })?
                    };
                    self.tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
            }
            "message_stop" => self.done = true,
            "error" => {
                let message = data
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("erro não especificado")
                    .to_string();
                return Err(ProviderError::Api {
                    status: 200,
                    message,
                });
            }
            // message_start, message_delta, ping: nada a acumular por ora.
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<TurnOutcome, ProviderError> {
        if !self.done {
            return Err(ProviderError::Protocol(
                "stream terminou sem message_stop".into(),
            ));
        }
        Ok(TurnOutcome::from_turn(AgentTurn {
            text: self.text,
            tool_calls: self.tool_calls,
        }))
    }
}

/// Converte uma entrada de `GET /v1/models` em `ModelInfo`.
fn model_from_wire(m: &Value) -> Option<ModelInfo> {
    Some(ModelInfo {
        id: m["id"].as_str()?.to_string(),
        display_name: m["display_name"].as_str().map(String::from),
        max_input_tokens: m["max_input_tokens"].as_i64(),
        // Na Models API da Anthropic, `max_tokens` é o teto de saída.
        max_output_tokens: m["max_tokens"].as_i64(),
        capabilities: m.get("capabilities").cloned().unwrap_or_else(|| json!({})),
    })
}

#[async_trait]
impl AgentProvider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = Vec::new();
        let mut after_id: Option<String> = None;

        loop {
            let mut request = self
                .request(reqwest::Method::GET, "/v1/models")
                .query(&[("limit", "100")]);
            if let Some(after) = &after_id {
                request = request.query(&[("after_id", after.as_str())]);
            }
            let response = request.send().await?;
            let status = response.status();
            let body = response.text().await?;
            if !status.is_success() {
                return Err(ProviderError::Api {
                    status: status.as_u16(),
                    message: api_error_message(&body),
                });
            }

            let page: Value = serde_json::from_str(&body)
                .map_err(|e| ProviderError::Protocol(format!("listagem inválida: {e}")))?;
            let data = page["data"]
                .as_array()
                .ok_or_else(|| ProviderError::Protocol("listagem sem campo data".into()))?;
            models.extend(data.iter().filter_map(model_from_wire));

            if page["has_more"].as_bool() == Some(true) {
                after_id = page["last_id"].as_str().map(String::from);
                if after_id.is_none() {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(models)
    }

    async fn turn(
        &self,
        request: &TurnRequest,
        sink: DeltaSink<'_>,
    ) -> Result<TurnOutcome, ProviderError> {
        let response = self
            .request(reqwest::Method::POST, "/v1/messages")
            .json(&build_body(request))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status.as_u16(),
                message: api_error_message(&body),
            });
        }

        let mut parser = SseParser::default();
        let mut acc = TurnAccumulator::default();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            for event in parser.feed(&chunk?) {
                acc.apply(&event, sink)?;
            }
        }
        acc.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::domain::{ToolResult, ToolSpec};
    use std::sync::Mutex;

    fn sample_request() -> TurnRequest {
        TurnRequest {
            model: "claude-sonnet-5".into(),
            system: Some("Você acompanha uma sessão SSH.".into()),
            messages: vec![
                AgentMessage::User("qual o uptime?".into()),
                AgentMessage::Assistant(AgentTurn {
                    text: "Vou verificar.".into(),
                    tool_calls: vec![ToolCall {
                        id: "toolu_1".into(),
                        name: "run_command".into(),
                        arguments: json!({"command": "uptime"}),
                    }],
                }),
                AgentMessage::ToolResults(vec![ToolResult {
                    call_id: "toolu_1".into(),
                    content: "up 3 days".into(),
                    is_error: false,
                }]),
            ],
            tools: vec![ToolSpec {
                name: "run_command".into(),
                description: "Executa um comando no servidor.".into(),
                input_schema: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            }],
            max_tokens: 4096,
        }
    }

    #[test]
    fn builds_wire_body_with_tool_use_and_tool_result_blocks() {
        let body = build_body(&sample_request());

        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "Você acompanha uma sessão SSH.");
        // Ferramentas usam input_schema (dialeto Anthropic).
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["content"], "qual o uptime?");
        // Turno de assistente vira blocos text + tool_use.
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][1]["type"], "tool_use");
        assert_eq!(messages[1]["content"][1]["id"], "toolu_1");
        // Resultado vira bloco tool_result num turno de usuário.
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
    }

    fn apply_all(events: &[(&str, &str)]) -> (TurnAccumulator, Vec<StreamDelta>) {
        let deltas = Mutex::new(Vec::new());
        let mut acc = TurnAccumulator::default();
        for (event, data) in events {
            acc.apply(
                &SseEvent {
                    event: Some(event.to_string()),
                    data: data.to_string(),
                },
                &|d| deltas.lock().unwrap().push(d),
            )
            .unwrap();
        }
        (acc, deltas.into_inner().unwrap())
    }

    #[test]
    fn accumulates_text_stream_into_text_outcome() {
        let (acc, deltas) = apply_all(&[
            ("message_start", r#"{"type":"message_start","message":{}}"#),
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Servidor "}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok."}}"#,
            ),
            ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);

        let outcome = acc.finish().unwrap();
        assert_eq!(outcome, TurnOutcome::Text(AgentTurn {
            text: "Servidor ok.".into(),
            tool_calls: vec![],
        }));
        assert_eq!(deltas.len(), 2);
    }

    #[test]
    fn reassembles_tool_use_from_partial_json() {
        let (acc, _) = apply_all(&[
            (
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"run_command","input":{}}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"comm"}}"#,
            ),
            (
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"and\": \"df -h\"}"}}"#,
            ),
            ("content_block_stop", r#"{"type":"content_block_stop","index":0}"#),
            ("message_stop", r#"{"type":"message_stop"}"#),
        ]);

        match acc.finish().unwrap() {
            TurnOutcome::ToolCalls(turn) => {
                assert_eq!(turn.tool_calls[0].id, "toolu_9");
                assert_eq!(turn.tool_calls[0].name, "run_command");
                assert_eq!(turn.tool_calls[0].arguments, json!({"command": "df -h"}));
            }
            other => panic!("esperava ToolCalls, veio {other:?}"),
        }
    }

    #[test]
    fn surfaces_stream_error_event() {
        let deltas = Mutex::new(Vec::new());
        let mut acc = TurnAccumulator::default();
        let err = acc
            .apply(
                &SseEvent {
                    event: Some("error".into()),
                    data: r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#.into(),
                },
                &|d| deltas.lock().unwrap().push(d),
            )
            .unwrap_err();
        assert!(err.to_string().contains("Overloaded"));
    }

    #[test]
    fn truncated_stream_is_a_protocol_error() {
        let (acc, _) = apply_all(&[(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"parcial"}}"#,
        )]);
        assert!(matches!(acc.finish(), Err(ProviderError::Protocol(_))));
    }

    #[test]
    fn maps_models_listing_with_capabilities() {
        let wire = json!({
            "id": "claude-sonnet-5",
            "display_name": "Claude Sonnet 5",
            "max_input_tokens": 1_000_000,
            "max_tokens": 128_000,
            "capabilities": {"thinking": {"supported": true}},
        });
        let info = model_from_wire(&wire).unwrap();
        assert_eq!(info.id, "claude-sonnet-5");
        assert_eq!(info.max_input_tokens, Some(1_000_000));
        assert_eq!(info.max_output_tokens, Some(128_000));
        assert_eq!(info.capabilities["thinking"]["supported"], true);

        // Campos de capacidade ausentes não derrubam a listagem.
        let minimal = model_from_wire(&json!({"id": "claude-haiku-4-5"})).unwrap();
        assert_eq!(minimal.capabilities, json!({}));
    }
}
