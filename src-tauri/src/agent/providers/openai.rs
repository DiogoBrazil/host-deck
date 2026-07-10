//! Adapter para o dialeto Chat Completions, usado por OpenAI e OpenRouter.
//!
//! OpenRouter (https://openrouter.ai/docs/quickstart) é intencionalmente
//! compatível com a API da OpenAI: mesma autenticação Bearer, mesmo
//! `POST /chat/completions` com `tools`/`tool_calls` e mesmo framing SSE
//! com terminador `data: [DONE]`. As diferenças ficam no `Flavor`:
//! base URL, campo de teto de tokens e a riqueza da listagem de modelos.

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

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flavor {
    OpenAi,
    OpenRouter,
}

pub struct OpenAiCompatProvider {
    id: String,
    flavor: Flavor,
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiCompatProvider {
    pub fn openai(id: String, base_url: Option<String>, api_key: String) -> Self {
        Self::new(id, Flavor::OpenAi, base_url, api_key)
    }

    pub fn openrouter(id: String, base_url: Option<String>, api_key: String) -> Self {
        Self::new(id, Flavor::OpenRouter, base_url, api_key)
    }

    fn new(id: String, flavor: Flavor, base_url: Option<String>, api_key: String) -> Self {
        let default = match flavor {
            Flavor::OpenAi => OPENAI_BASE_URL,
            Flavor::OpenRouter => OPENROUTER_BASE_URL,
        };
        Self {
            id,
            flavor,
            base_url: base_url
                .unwrap_or_else(|| default.into())
                .trim_end_matches('/')
                .to_string(),
            api_key,
            client: http_client(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.api_key);
        if self.flavor == Flavor::OpenRouter {
            // Atribuição opcional no ranking do OpenRouter.
            builder = builder.header("X-Title", "HostDeck");
        }
        builder
    }
}

/// Monta o corpo de `POST /chat/completions` a partir do dialeto interno.
fn build_body(flavor: Flavor, req: &TurnRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for msg in &req.messages {
        match msg {
            AgentMessage::User(text) => {
                messages.push(json!({"role": "user", "content": text}));
            }
            AgentMessage::Assistant(turn) => {
                let mut wire = json!({
                    "role": "assistant",
                    "content": if turn.text.is_empty() { Value::Null } else { json!(turn.text) },
                });
                if !turn.tool_calls.is_empty() {
                    wire["tool_calls"] = Value::Array(
                        turn.tool_calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": {
                                        "name": c.name,
                                        // Neste dialeto os argumentos vão serializados.
                                        "arguments": c.arguments.to_string(),
                                    },
                                })
                            })
                            .collect(),
                    );
                }
                messages.push(wire);
            }
            AgentMessage::ToolResults(results) => {
                for r in results {
                    // Não há campo is_error; o prefixo mantém o sinal.
                    let content = if r.is_error {
                        format!("ERRO: {}", r.content)
                    } else {
                        r.content.clone()
                    };
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": r.call_id,
                        "content": content,
                    }));
                }
            }
        }
    }

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
    });
    // A OpenAI aposentou `max_tokens` em favor de `max_completion_tokens`;
    // o OpenRouter normaliza `max_tokens` para todos os modelos.
    match flavor {
        Flavor::OpenAi => body["max_completion_tokens"] = json!(req.max_tokens),
        Flavor::OpenRouter => body["max_tokens"] = json!(req.max_tokens),
    }
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    if req.thinking {
        match flavor {
            // https://openrouter.ai/docs/use-cases/reasoning-tokens; os
            // deltas voltam em `delta.reasoning` → StreamDelta::Thinking.
            Flavor::OpenRouter => body["reasoning"] = json!({"enabled": true}),
            // A listagem da OpenAI não anuncia capacidades, então a UI nunca
            // liga a flag neste flavor; nada a enviar.
            Flavor::OpenAi => {}
        }
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        },
                    })
                })
                .collect(),
        );
    }
    body
}

/// Reduz os chunks SSE do Chat Completions a um `AgentTurn`.
///
/// `tool_calls` chega fatiado por índice: o primeiro fragmento traz id e
/// nome, os seguintes concatenam `function.arguments`.
#[derive(Default)]
struct TurnAccumulator {
    text: String,
    open_tools: BTreeMap<u64, (String, String, String)>,
    done: bool,
}

impl TurnAccumulator {
    fn apply(&mut self, event: &SseEvent, sink: DeltaSink<'_>) -> Result<(), ProviderError> {
        if event.data.is_empty() {
            return Ok(());
        }
        if event.data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let data: Value = serde_json::from_str(&event.data)
            .map_err(|e| ProviderError::Protocol(format!("SSE com JSON inválido: {e}")))?;

        // Erros mid-stream chegam como um objeto {"error": {...}}.
        if let Some(message) = data.pointer("/error/message").and_then(|m| m.as_str()) {
            return Err(ProviderError::Api {
                status: 200,
                message: message.to_string(),
            });
        }

        let Some(choice) = data.pointer("/choices/0") else {
            return Ok(());
        };
        let delta = &choice["delta"];

        if let Some(text) = delta["content"].as_str() {
            if !text.is_empty() {
                self.text.push_str(text);
                sink(StreamDelta::Text(text.to_string()));
            }
        }
        // OpenRouter expõe raciocínio de modelos que o suportam em `reasoning`.
        if let Some(reasoning) = delta["reasoning"].as_str() {
            if !reasoning.is_empty() {
                sink(StreamDelta::Thinking(reasoning.to_string()));
            }
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let index = call["index"].as_u64().unwrap_or(0);
                let entry = self
                    .open_tools
                    .entry(index)
                    .or_insert_with(|| (String::new(), String::new(), String::new()));
                if let Some(id) = call["id"].as_str() {
                    entry.0 = id.to_string();
                }
                if let Some(name) = call.pointer("/function/name").and_then(|n| n.as_str()) {
                    entry.1 = name.to_string();
                }
                if let Some(args) = call.pointer("/function/arguments").and_then(|a| a.as_str()) {
                    entry.2.push_str(args);
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<TurnOutcome, ProviderError> {
        if !self.done {
            return Err(ProviderError::Protocol(
                "stream terminou sem [DONE]".into(),
            ));
        }
        let mut tool_calls = Vec::new();
        for (_, (id, name, buf)) in self.open_tools {
            let arguments = if buf.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&buf).map_err(|e| {
                    ProviderError::Protocol(format!("argumentos de tool_call inválidos: {e}"))
                })?
            };
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
        Ok(TurnOutcome::from_turn(AgentTurn {
            text: self.text,
            tool_calls,
        }))
    }
}

/// Converte uma entrada de `GET /models` em `ModelInfo`.
///
/// A OpenAI só devolve ids; o OpenRouter traz nome, janela de contexto,
/// preços e `supported_parameters` — tudo isso vira `capabilities`.
fn model_from_wire(flavor: Flavor, m: &Value) -> Option<ModelInfo> {
    let id = m["id"].as_str()?.to_string();
    match flavor {
        Flavor::OpenAi => Some(ModelInfo {
            id,
            display_name: None,
            max_input_tokens: None,
            max_output_tokens: None,
            capabilities: json!({}),
        }),
        Flavor::OpenRouter => {
            let mut capabilities = serde_json::Map::new();
            for key in ["pricing", "supported_parameters", "architecture"] {
                if let Some(v) = m.get(key) {
                    capabilities.insert(key.to_string(), v.clone());
                }
            }
            Some(ModelInfo {
                id,
                display_name: m["name"].as_str().map(String::from),
                max_input_tokens: m["context_length"].as_i64(),
                max_output_tokens: m
                    .pointer("/top_provider/max_completion_tokens")
                    .and_then(|v| v.as_i64()),
                capabilities: Value::Object(capabilities),
            })
        }
    }
}

#[async_trait]
impl AgentProvider for OpenAiCompatProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let response = self.request(reqwest::Method::GET, "/models").send().await?;
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
        Ok(data
            .iter()
            .filter_map(|m| model_from_wire(self.flavor, m))
            .collect())
    }

    async fn turn(
        &self,
        request: &TurnRequest,
        sink: DeltaSink<'_>,
    ) -> Result<TurnOutcome, ProviderError> {
        let response = self
            .request(reqwest::Method::POST, "/chat/completions")
            .json(&build_body(self.flavor, request))
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
            model: "anthropic/claude-sonnet-5".into(),
            system: Some("Você acompanha uma sessão SSH.".into()),
            messages: vec![
                AgentMessage::User("qual o uptime?".into()),
                AgentMessage::Assistant(AgentTurn {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "run_command".into(),
                        arguments: json!({"command": "uptime"}),
                    }],
                }),
                AgentMessage::ToolResults(vec![ToolResult {
                    call_id: "call_1".into(),
                    content: "denied".into(),
                    is_error: true,
                }]),
            ],
            tools: vec![ToolSpec {
                name: "run_command".into(),
                description: "Executa um comando no servidor.".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 4096,
            temperature: None,
            thinking: false,
        }
    }

    #[test]
    fn builds_wire_body_in_chat_completions_dialect() {
        let body = build_body(Flavor::OpenRouter, &sample_request());

        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 4096);
        // Ferramentas usam function.parameters (dialeto OpenAI).
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");

        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        // Assistant sem texto vira content null + tool_calls serializado.
        assert_eq!(messages[2]["content"], Value::Null);
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"],
            r#"{"command":"uptime"}"#
        );
        // Resultado vira mensagem role tool; erro é prefixado.
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(messages[3]["content"], "ERRO: denied");
    }

    #[test]
    fn openai_flavor_uses_max_completion_tokens() {
        let body = build_body(Flavor::OpenAi, &sample_request());
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn temperature_and_thinking_only_travel_when_set() {
        let bare = build_body(Flavor::OpenRouter, &sample_request());
        assert!(bare.get("temperature").is_none());
        assert!(bare.get("reasoning").is_none());

        let mut req = sample_request();
        req.temperature = Some(0.7);
        req.thinking = true;
        let body = build_body(Flavor::OpenRouter, &req);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["reasoning"]["enabled"], true);

        // No flavor OpenAI o raciocínio não tem tradução; a flag é ignorada.
        let body = build_body(Flavor::OpenAi, &req);
        assert_eq!(body["temperature"], 0.7);
        assert!(body.get("reasoning").is_none());
    }

    fn apply_all(datas: &[&str]) -> (TurnAccumulator, Vec<StreamDelta>) {
        let deltas = Mutex::new(Vec::new());
        let mut acc = TurnAccumulator::default();
        for data in datas {
            acc.apply(
                &SseEvent {
                    event: None,
                    data: data.to_string(),
                },
                &|d| deltas.lock().unwrap().push(d),
            )
            .unwrap();
        }
        (acc, deltas.into_inner().unwrap())
    }

    #[test]
    fn accumulates_text_chunks_until_done() {
        let (acc, deltas) = apply_all(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"Servidor "}}]}"#,
            r#"{"choices":[{"delta":{"content":"ok."}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        let outcome = acc.finish().unwrap();
        assert_eq!(outcome.turn().text, "Servidor ok.");
        assert_eq!(deltas.len(), 2);
    }

    #[test]
    fn reassembles_tool_calls_split_across_chunks() {
        let (acc, _) = apply_all(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_9","type":"function","function":{"name":"run_command","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":": \"df -h\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]);
        match acc.finish().unwrap() {
            TurnOutcome::ToolCalls(turn) => {
                assert_eq!(turn.tool_calls[0].id, "call_9");
                assert_eq!(turn.tool_calls[0].name, "run_command");
                assert_eq!(turn.tool_calls[0].arguments, json!({"command": "df -h"}));
            }
            other => panic!("esperava ToolCalls, veio {other:?}"),
        }
    }

    #[test]
    fn reasoning_deltas_map_to_thinking() {
        let (_, deltas) = apply_all(&[
            r#"{"choices":[{"delta":{"reasoning":"pensando..."}}]}"#,
            "[DONE]",
        ]);
        assert_eq!(deltas, vec![StreamDelta::Thinking("pensando...".into())]);
    }

    #[test]
    fn mid_stream_error_is_surfaced() {
        let mut acc = TurnAccumulator::default();
        let err = acc
            .apply(
                &SseEvent {
                    event: None,
                    data: r#"{"error":{"message":"Rate limit exceeded","code":429}}"#.into(),
                },
                &|_| {},
            )
            .unwrap_err();
        assert!(err.to_string().contains("Rate limit exceeded"));
    }

    #[test]
    fn truncated_stream_is_a_protocol_error() {
        let (acc, _) = apply_all(&[r#"{"choices":[{"delta":{"content":"parcial"}}]}"#]);
        assert!(matches!(acc.finish(), Err(ProviderError::Protocol(_))));
    }

    #[test]
    fn openrouter_listing_maps_metadata_into_capabilities() {
        let wire = json!({
            "id": "anthropic/claude-sonnet-5",
            "name": "Anthropic: Claude Sonnet 5",
            "context_length": 1_000_000,
            "top_provider": {"max_completion_tokens": 128_000},
            "pricing": {"prompt": "0.000003", "completion": "0.000015"},
            "supported_parameters": ["tools", "max_tokens"],
        });
        let info = model_from_wire(Flavor::OpenRouter, &wire).unwrap();
        assert_eq!(info.display_name.as_deref(), Some("Anthropic: Claude Sonnet 5"));
        assert_eq!(info.max_input_tokens, Some(1_000_000));
        assert_eq!(info.max_output_tokens, Some(128_000));
        assert_eq!(info.capabilities["pricing"]["prompt"], "0.000003");
        assert_eq!(info.capabilities["supported_parameters"][0], "tools");

        // OpenAI só devolve o id.
        let plain = model_from_wire(Flavor::OpenAi, &json!({"id": "gpt-5"})).unwrap();
        assert!(plain.display_name.is_none());
        assert_eq!(plain.capabilities, json!({}));
    }
}
