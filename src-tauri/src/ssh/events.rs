use serde::Serialize;

/// Eventos enviados ao frontend pelo `tauri::ipc::Channel` da sessão.
/// A saída do terminal vai em base64 para preservar bytes brutos
/// (sequências ANSI podem quebrar UTF-8 em fronteiras de chunk).
#[derive(Clone, Serialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TerminalEvent {
    /// Sessão estabelecida; terminal pronto.
    Connected { session_id: String },
    /// Saída do servidor (base64 de bytes brutos).
    Output { data: String },
    /// Primeira conexão a este host: aguarda confirmação do fingerprint.
    HostKeyPrompt { fingerprint: String, key_type: String },
    /// Sessão encerrada (pelo servidor ou pelo usuário).
    Closed { reason: String },
    /// Erro assíncrono após a conexão estabelecida.
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_variant_and_fields_as_camel_case() {
        // O frontend depende EXATAMENTE destes nomes (event/data/campos).
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
