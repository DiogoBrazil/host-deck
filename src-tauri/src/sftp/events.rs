use serde::Serialize;
use tauri::ipc::Channel;

use crate::error::{AppError, AppResult};
use crate::ssh::client::HostKeyPrompter;

/// Events streamed to the frontend over the SFTP session `tauri::ipc::Channel`.
///
/// Mirrors `TerminalEvent`'s serialization convention (`event` tag, `data`
/// content, `camelCase`). File contents never cross this channel — only
/// directory metadata and transfer progress.
#[derive(Clone, Serialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SftpEvent {
    /// SFTP session established.
    Connected { session_id: String },
    /// First connection to this host; fingerprint confirmation is required (TOFU).
    HostKeyPrompt { fingerprint: String, key_type: String },
    /// Upload/download progress.
    Progress {
        transfer_id: String,
        transferred: u64,
        total: u64,
    },
    /// Transfer finished successfully.
    TransferDone { transfer_id: String, path: String },
    /// Asynchronous error after the session has started.
    Error { message: String },
    /// Session closed by the server or the user.
    Closed { reason: String },
}

/// `HostKeyPrompter` backed by an SFTP event channel, so `ssh::client::connect`
/// can drive the same TOFU prompt for SFTP sessions.
pub struct SftpPrompter(pub Channel<SftpEvent>);

impl HostKeyPrompter for SftpPrompter {
    fn prompt(&self, fingerprint: String, key_type: String) -> AppResult<()> {
        self.0
            .send(SftpEvent::HostKeyPrompt {
                fingerprint,
                key_type,
            })
            .map_err(|e| AppError::Internal(format!("enviando evento: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_variant_and_fields_as_camel_case() {
        // The frontend depends on these exact serialized names.
        let ev = SftpEvent::HostKeyPrompt {
            fingerprint: "SHA256:abc".into(),
            key_type: "ssh-ed25519".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"hostKeyPrompt\""), "json={json}");
        assert!(json.contains("\"keyType\":\"ssh-ed25519\""), "json={json}");

        let progress = SftpEvent::Progress {
            transfer_id: "t1".into(),
            transferred: 10,
            total: 100,
        };
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"event\":\"progress\""), "json={json}");
        assert!(json.contains("\"transferId\":\"t1\""), "json={json}");
        assert!(json.contains("\"transferred\":10"), "json={json}");

        let done = SftpEvent::TransferDone {
            transfer_id: "t1".into(),
            path: "/tmp/f".into(),
        };
        let json = serde_json::to_string(&done).unwrap();
        assert!(json.contains("\"event\":\"transferDone\""), "json={json}");

        let connected = SftpEvent::Connected {
            session_id: "s1".into(),
        };
        let json = serde_json::to_string(&connected).unwrap();
        assert!(json.contains("\"sessionId\":\"s1\""), "json={json}");
    }
}
