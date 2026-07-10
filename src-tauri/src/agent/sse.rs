//! Parser incremental de Server-Sent Events.
//!
//! Os dois provedores usam SSE com framing distinto (Anthropic nomeia
//! `event:`; OpenAI/OpenRouter só mandam `data:` e comentários de
//! keep-alive). Este parser aceita chunks parciais de rede e devolve
//! apenas eventos completos.

/// Um evento SSE completo.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Acumula bytes até fechar um bloco (linha em branco) e o converte.
///
/// O buffer é mantido em bytes porque um chunk pode cortar um caractere
/// UTF-8 no meio; a conversão só acontece em blocos completos.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    /// Alimenta um chunk e devolve os eventos completados por ele.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some((block_end, sep_len)) = find_block_end(&self.buf) {
            let block: Vec<u8> = self.buf.drain(..block_end + sep_len).collect();
            let text = String::from_utf8_lossy(&block[..block_end]);
            if let Some(event) = parse_block(&text) {
                events.push(event);
            }
        }

        events
    }
}

/// Encontra o fim do primeiro bloco: `\n\n` ou `\r\n\r\n`, o que vier antes.
fn find_block_end(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (one, other) => one.or(other),
    }
}

fn parse_block(text: &str) -> Option<SseEvent> {
    let mut event = None;
    let mut data_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim_start_matches(' ').to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.strip_prefix(' ').unwrap_or(value));
        }
        // Comentários (": keep-alive") e campos como `id:`/`retry:` são ignorados.
    }

    if event.is_none() && data_lines.is_empty() {
        return None;
    }
    Some(SseEvent {
        event,
        data: data_lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_events_across_chunk_boundaries() {
        let mut p = SseParser::default();
        assert!(p.feed(b"event: content_block_delta\ndata: {\"a\"").is_empty());
        let events = p.feed(b": 1}\n\nevent: message_stop\ndata: {}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(events[0].data, r#"{"a": 1}"#);
        assert_eq!(events[1].event.as_deref(), Some("message_stop"));
    }

    #[test]
    fn handles_crlf_and_ignores_comments() {
        let mut p = SseParser::default();
        let events = p.feed(b": OPENROUTER PROCESSING\r\n\r\ndata: {\"x\":1}\r\n\r\ndata: [DONE]\r\n\r\n");
        assert_eq!(events.len(), 2);
        assert!(events[0].event.is_none());
        assert_eq!(events[0].data, r#"{"x":1}"#);
        assert_eq!(events[1].data, "[DONE]");
    }

    #[test]
    fn joins_multiline_data() {
        let mut p = SseParser::default();
        let events = p.feed(b"data: linha1\ndata: linha2\n\n");
        assert_eq!(events[0].data, "linha1\nlinha2");
    }

    #[test]
    fn does_not_split_utf8_between_chunks() {
        let mut p = SseParser::default();
        let full = "data: coração\n\n".as_bytes();
        // Corta no meio do "ç" (multi-byte).
        let cut = full.len() - 6;
        assert!(p.feed(&full[..cut]).is_empty());
        let events = p.feed(&full[cut..]);
        assert_eq!(events[0].data, "coração");
    }
}
