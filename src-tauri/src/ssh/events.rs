use serde::Serialize;

/// Events streamed to the frontend over the session `tauri::ipc::Channel`.
///
/// Terminal output is base64 encoded to preserve raw bytes across chunk boundaries.
#[derive(Clone, Serialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEvent {
    /// Session established; the terminal is ready.
    Connected { session_id: String },
    /// Server output encoded as base64 raw bytes.
    Output { data: String },
    /// First connection to this host; fingerprint confirmation is required.
    HostKeyPrompt { fingerprint: String, key_type: String },
    /// Session closed by the server or the user.
    Closed { reason: String },
    /// Asynchronous error after the session has started.
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_variant_and_fields_as_camel_case() {
        // The frontend depends on these exact serialized names.
        let ev = TerminalEvent::HostKeyPrompt {
            fingerprint: "SHA256:abc".into(),
            key_type: "ssh-ed25519".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"event\":\"hostKeyPrompt\""), "json={json}");
        assert!(json.contains("\"fingerprint\":\"SHA256:abc\""), "json={json}");
        assert!(json.contains("\"keyType\":\"ssh-ed25519\""), "json={json}");

        let connected = TerminalEvent::Connected {
            session_id: "s1".into(),
        };
        let json = serde_json::to_string(&connected).unwrap();
        assert!(json.contains("\"sessionId\":\"s1\""), "json={json}");
    }
}
