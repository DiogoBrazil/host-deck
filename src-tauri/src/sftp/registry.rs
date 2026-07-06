use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tauri::ipc::Channel;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::sftp::events::SftpEvent;
use crate::ssh::client::TofuHandler;

/// An established SFTP session plus the resources that must outlive it.
pub struct SftpSessionHandle {
    /// Keeps the SSH connection alive while the SFTP session exists.
    pub _ssh: Handle<TofuHandler>,
    pub sftp: Arc<SftpSession>,
    /// Event channel opened by `sftp_connect`; transfers stream progress here.
    pub events: Channel<SftpEvent>,
}

/// Active SFTP sessions and in-flight transfer cancellation tokens.
///
/// SFTP is request/response rather than streaming, so it keeps its own registry
/// separate from the terminal's `SessionRegistry`.
#[derive(Default, Clone)]
pub struct SftpRegistry {
    sessions: Arc<Mutex<HashMap<String, SftpSessionHandle>>>,
    /// Pending TOFU confirmation senders, awaiting a `confirm_host_key` decision.
    host_key: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    /// Cancellation tokens per `transfer_id`.
    transfers: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl SftpRegistry {
    pub fn has(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(session_id)
            || self.host_key.lock().unwrap().contains_key(session_id)
    }

    pub fn insert(&self, session_id: String, handle: SftpSessionHandle) {
        // The session is now established; the pending prompt slot is no longer needed.
        self.host_key.lock().unwrap().remove(&session_id);
        self.sessions.lock().unwrap().insert(session_id, handle);
    }

    pub fn remove(&self, session_id: &str) -> Option<SftpSessionHandle> {
        self.host_key.lock().unwrap().remove(session_id);
        self.sessions.lock().unwrap().remove(session_id)
    }

    pub fn session(&self, session_id: &str) -> Option<Arc<SftpSession>> {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.sftp.clone())
    }

    pub fn events(&self, session_id: &str) -> Option<Channel<SftpEvent>> {
        self.sessions
            .lock()
            .unwrap()
            .get(session_id)
            .map(|h| h.events.clone())
    }

    pub fn register_host_key(&self, session_id: String, tx: oneshot::Sender<bool>) {
        self.host_key.lock().unwrap().insert(session_id, tx);
    }

    /// Takes the pending TOFU confirmation sender, if any.
    pub fn take_host_key_tx(&self, session_id: &str) -> Option<oneshot::Sender<bool>> {
        self.host_key.lock().unwrap().remove(session_id)
    }

    pub fn register_transfer(&self, transfer_id: &str, token: CancellationToken) {
        self.transfers
            .lock()
            .unwrap()
            .insert(transfer_id.to_string(), token);
    }

    pub fn cancel_transfer(&self, transfer_id: &str) {
        if let Some(token) = self.transfers.lock().unwrap().get(transfer_id) {
            token.cancel();
        }
    }

    pub fn finish_transfer(&self, transfer_id: &str) {
        self.transfers.lock().unwrap().remove(transfer_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_register_and_take() {
        let reg = SftpRegistry::default();
        let (tx, rx) = oneshot::channel::<bool>();
        reg.register_host_key("s1".into(), tx);

        assert!(reg.has("s1"));
        let taken = reg.take_host_key_tx("s1").expect("sender present");
        taken.send(true).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), true);

        assert!(reg.take_host_key_tx("s1").is_none());
        assert!(!reg.has("s1"));
    }

    #[test]
    fn transfer_token_lifecycle() {
        let reg = SftpRegistry::default();
        let token = CancellationToken::new();
        reg.register_transfer("t1", token.clone());

        assert!(!token.is_cancelled());
        reg.cancel_transfer("t1");
        assert!(token.is_cancelled());

        reg.finish_transfer("t1");
        // Cancelling an unknown/finished transfer is a no-op.
        reg.cancel_transfer("t1");
    }
}
