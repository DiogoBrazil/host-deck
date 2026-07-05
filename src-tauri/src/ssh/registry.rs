use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

/// Entrada enviada do frontend para a task da sessão SSH.
pub enum SessionInput {
    /// Bytes digitados pelo usuário.
    Data(Vec<u8>),
    /// Novo tamanho do terminal (colunas, linhas).
    Resize { cols: u32, rows: u32 },
    /// Encerrar a sessão a pedido do usuário.
    Close,
}

pub struct SessionHandle {
    pub input_tx: mpsc::Sender<SessionInput>,
    /// Presente enquanto a confirmação TOFU do host key está pendente.
    pub host_key_tx: Option<oneshot::Sender<bool>>,
}

/// Sessões SSH ativas, indexadas por session_id (UUID). Clonável (Arc
/// interno) para que tasks de sessão possam se remover ao encerrar.
#[derive(Default, Clone)]
pub struct SessionRegistry(Arc<Mutex<HashMap<String, SessionHandle>>>);

impl SessionRegistry {
    pub fn insert(&self, session_id: String, handle: SessionHandle) {
        self.0.lock().unwrap().insert(session_id, handle);
    }

    pub fn remove(&self, session_id: &str) -> Option<SessionHandle> {
        self.0.lock().unwrap().remove(session_id)
    }

    pub fn input_sender(&self, session_id: &str) -> Option<mpsc::Sender<SessionInput>> {
        self.0
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.input_tx.clone())
    }

    /// Retira o sender de confirmação TOFU pendente, se houver.
    pub fn take_host_key_tx(&self, session_id: &str) -> Option<oneshot::Sender<bool>> {
        self.0
            .lock()
            .unwrap()
            .get_mut(session_id)
            .and_then(|h| h.host_key_tx.take())
    }
}
