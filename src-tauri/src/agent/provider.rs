//! Trait de provedor: a fronteira entre o laço agêntico e o mundo HTTP.

use async_trait::async_trait;

use super::domain::{ModelInfo, StreamDelta, TurnOutcome, TurnRequest};
use crate::error::AppError;

/// Callback que recebe os fragmentos de streaming durante um turno.
///
/// Fase 1 usa um closure para manter os adapters testáveis sem Tauri; o
/// laço da Fase 2 adapta isto para o `Channel<AgentEvent>` da UI.
pub type DeltaSink<'a> = &'a (dyn Fn(StreamDelta) + Send + Sync);

/// Erros na fronteira com o provedor. As mensagens podem ir para a UI e
/// nunca devem conter a chave de API.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("falha de rede ao falar com o provedor: {0}")]
    Network(String),

    /// O provedor respondeu com erro (4xx/5xx). 401/403 indicam chave inválida.
    #[error("provedor respondeu {status}: {message}")]
    Api { status: u16, message: String },

    #[error("resposta inesperada do provedor: {0}")]
    Protocol(String),
}

impl From<reqwest::Error> for ProviderError {
    fn from(e: reqwest::Error) -> Self {
        // `without_url` evita vazar query strings; a chave vai em header,
        // mas o hábito custa pouco.
        ProviderError::Network(e.without_url().to_string())
    }
}

impl From<ProviderError> for AppError {
    fn from(e: ProviderError) -> Self {
        AppError::Agent(e.to_string())
    }
}

/// Um provedor de modelos configurado (registro + chave + endpoint).
///
/// A abstração fica acima do laço agêntico: `turn` executa uma rodada
/// completa de streaming e devolve o desfecho já normalizado.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    /// Id do registro em `agent_providers` que originou este adapter.
    fn id(&self) -> &str;

    /// Lista os modelos disponíveis para alimentar o cache e a UI.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;

    /// Executa um turno, emitindo fragmentos em `sink` conforme chegam.
    async fn turn(
        &self,
        request: &TurnRequest,
        sink: DeltaSink<'_>,
    ) -> Result<TurnOutcome, ProviderError>;
}

#[cfg(test)]
pub mod mock {
    //! Provedor roteirizado para testes, no molde do `MockStore` de
    //! `credential_store`.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    pub struct ScriptedTurn {
        pub deltas: Vec<StreamDelta>,
        pub outcome: TurnOutcome,
    }

    #[derive(Default)]
    pub struct MockProvider {
        pub provider_id: String,
        pub models: Vec<ModelInfo>,
        pub turns: Mutex<VecDeque<ScriptedTurn>>,
        /// Requisições recebidas, para asserções.
        pub seen: Mutex<Vec<TurnRequest>>,
    }

    impl MockProvider {
        pub fn new(provider_id: &str) -> Self {
            Self {
                provider_id: provider_id.into(),
                ..Default::default()
            }
        }

        pub fn push_turn(&self, turn: ScriptedTurn) {
            self.turns.lock().unwrap().push_back(turn);
        }
    }

    #[async_trait]
    impl AgentProvider for MockProvider {
        fn id(&self) -> &str {
            &self.provider_id
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(self.models.clone())
        }

        async fn turn(
            &self,
            request: &TurnRequest,
            sink: DeltaSink<'_>,
        ) -> Result<TurnOutcome, ProviderError> {
            self.seen.lock().unwrap().push(request.clone());
            let scripted = self
                .turns
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| ProviderError::Protocol("mock sem turnos roteirizados".into()))?;
            for delta in scripted.deltas {
                sink(delta);
            }
            Ok(scripted.outcome)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{MockProvider, ScriptedTurn};
    use super::*;
    use crate::agent::domain::AgentTurn;
    use std::sync::Mutex;

    #[tokio::test]
    async fn mock_replays_scripted_turns_and_records_requests() {
        let provider = MockProvider::new("prov-1");
        provider.push_turn(ScriptedTurn {
            deltas: vec![StreamDelta::Text("olá".into())],
            outcome: TurnOutcome::Text(AgentTurn {
                text: "olá".into(),
                tool_calls: vec![],
            }),
        });

        let request = TurnRequest {
            model: "mock-model".into(),
            system: None,
            messages: vec![],
            tools: vec![],
            max_tokens: 1024,
        };

        let received = Mutex::new(Vec::new());
        let outcome = provider
            .turn(&request, &|d| received.lock().unwrap().push(d))
            .await
            .unwrap();

        assert_eq!(outcome.turn().text, "olá");
        assert_eq!(
            *received.lock().unwrap(),
            vec![StreamDelta::Text("olá".into())]
        );
        assert_eq!(provider.seen.lock().unwrap().len(), 1);

        // Sem roteiro restante, o mock acusa o teste mal montado.
        assert!(provider.turn(&request, &|_| {}).await.is_err());
    }
}
