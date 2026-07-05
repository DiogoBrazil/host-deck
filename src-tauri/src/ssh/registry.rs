use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

/// Input sent from the frontend to the SSH session task.
pub enum SessionInput {
    /// Bytes typed by the user.
    Data(Vec<u8>),
    /// New terminal size in columns and rows.
    Resize { cols: u32, rows: u32 },
    /// Close the session at the user's request.
    Close,
}

pub struct SessionHandle {
    pub input_tx: mpsc::Sender<SessionInput>,
    /// Present while TOFU host-key confirmation is pending.
    pub host_key_tx: Option<oneshot::Sender<bool>>,
}

/// Active SSH sessions indexed by frontend-generated session id.
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

    /// Takes the pending TOFU confirmation sender, if any.
    pub fn take_host_key_tx(&self, session_id: &str) -> Option<oneshot::Sender<bool>> {
        self.0
            .lock()
            .unwrap()
            .get_mut(session_id)
            .and_then(|h| h.host_key_tx.take())
    }
}
