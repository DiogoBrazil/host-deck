//! Conversas ativas do agente, indexadas por `session_id` do terminal.
//!
//! Mesmo desenho do `SftpRegistry`: estado compartilhado atrás de
//! `Arc<Mutex<..>>`, `CancellationToken` por trabalho em andamento e
//! `oneshot::Sender<bool>` para decisões pendentes do usuário.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::agent::domain::AgentMessage;
use crate::error::{AppError, AppResult};

#[derive(Default)]
struct Conversation {
    history: Vec<AgentMessage>,
    /// Token do turno em andamento; `None` quando o agente está ocioso.
    turn: Option<CancellationToken>,
    /// Confirmações de comando aguardando `confirm_agent_command`,
    /// indexadas pelo id da `ToolCall`.
    pending: HashMap<String, oneshot::Sender<bool>>,
}

/// Conversas do agente por sessão SSH.
#[derive(Default, Clone)]
pub struct AgentRegistry(Arc<Mutex<HashMap<String, Conversation>>>);

impl AgentRegistry {
    /// Inicia um turno; recusa se já houver um em andamento na sessão.
    pub fn begin_turn(&self, session_id: &str) -> AppResult<CancellationToken> {
        let mut guard = self.0.lock().unwrap();
        let conv = guard.entry(session_id.to_string()).or_default();
        if conv.turn.is_some() {
            return Err(AppError::Agent(
                "Já existe um turno do agente em andamento nesta sessão.".into(),
            ));
        }
        let token = CancellationToken::new();
        conv.turn = Some(token.clone());
        Ok(token)
    }

    /// Encerra o turno e descarta confirmações pendentes (recusadas por queda
    /// do sender).
    pub fn finish_turn(&self, session_id: &str) {
        if let Some(conv) = self.0.lock().unwrap().get_mut(session_id) {
            conv.turn = None;
            conv.pending.clear();
        }
    }

    /// Cancela o turno em andamento; confirmações pendentes são recusadas.
    pub fn cancel_turn(&self, session_id: &str) {
        if let Some(conv) = self.0.lock().unwrap().get_mut(session_id) {
            if let Some(token) = &conv.turn {
                token.cancel();
            }
            conv.pending.clear();
        }
    }

    /// Remove a conversa quando a sessão SSH fecha, cancelando o que houver.
    pub fn remove(&self, session_id: &str) {
        if let Some(conv) = self.0.lock().unwrap().remove(session_id) {
            if let Some(token) = conv.turn {
                token.cancel();
            }
        }
    }

    pub fn history_snapshot(&self, session_id: &str) -> Vec<AgentMessage> {
        self.0
            .lock()
            .unwrap()
            .get(session_id)
            .map(|c| c.history.clone())
            .unwrap_or_default()
    }

    /// Grava o histórico produzido por um turno (inclusive parcial, se
    /// cancelado no meio).
    pub fn replace_history(&self, session_id: &str, history: Vec<AgentMessage>) {
        let mut guard = self.0.lock().unwrap();
        guard.entry(session_id.to_string()).or_default().history = history;
    }

    pub fn register_confirmation(
        &self,
        session_id: &str,
        call_id: &str,
        tx: oneshot::Sender<bool>,
    ) {
        let mut guard = self.0.lock().unwrap();
        guard
            .entry(session_id.to_string())
            .or_default()
            .pending
            .insert(call_id.to_string(), tx);
    }

    /// Retira o sender da confirmação pendente, se houver.
    pub fn take_confirmation(
        &self,
        session_id: &str,
        call_id: &str,
    ) -> Option<oneshot::Sender<bool>> {
        self.0
            .lock()
            .unwrap()
            .get_mut(session_id)
            .and_then(|c| c.pending.remove(call_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_lifecycle_rejects_concurrent_turns() {
        let reg = AgentRegistry::default();

        let token = reg.begin_turn("s1").unwrap();
        assert!(!token.is_cancelled());
        assert!(reg.begin_turn("s1").is_err(), "turno concorrente na mesma sessão");
        // Outra sessão não é afetada.
        reg.begin_turn("s2").unwrap();

        reg.finish_turn("s1");
        reg.begin_turn("s1").unwrap();
    }

    #[test]
    fn cancel_cancels_token_and_drops_pending_confirmations() {
        let reg = AgentRegistry::default();
        let token = reg.begin_turn("s1").unwrap();

        let (tx, mut rx) = oneshot::channel::<bool>();
        reg.register_confirmation("s1", "c1", tx);

        reg.cancel_turn("s1");
        assert!(token.is_cancelled());
        // Sender caiu: quem espera a confirmação lê como recusa.
        assert!(rx.try_recv().is_err());
        assert!(reg.take_confirmation("s1", "c1").is_none());
    }

    #[test]
    fn confirmation_register_and_take() {
        let reg = AgentRegistry::default();
        let (tx, rx) = oneshot::channel::<bool>();
        reg.register_confirmation("s1", "c1", tx);

        let taken = reg.take_confirmation("s1", "c1").expect("sender presente");
        taken.send(true).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), true);
        assert!(reg.take_confirmation("s1", "c1").is_none());
    }

    #[test]
    fn history_snapshot_and_replace() {
        let reg = AgentRegistry::default();
        assert!(reg.history_snapshot("s1").is_empty());

        reg.replace_history("s1", vec![AgentMessage::User("oi".into())]);
        let history = reg.history_snapshot("s1");
        assert_eq!(history.len(), 1);
        assert!(matches!(&history[0], AgentMessage::User(m) if m == "oi"));
    }

    #[test]
    fn remove_cancels_running_turn() {
        let reg = AgentRegistry::default();
        let token = reg.begin_turn("s1").unwrap();
        reg.remove("s1");
        assert!(token.is_cancelled());
        assert!(reg.history_snapshot("s1").is_empty());
    }
}
