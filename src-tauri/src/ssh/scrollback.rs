use std::collections::VecDeque;

/// Default scrollback capacity in bytes (~200 KB).
pub const SCROLLBACK_CAP: usize = 200 * 1024;

/// Bounded ring buffer with the raw PTY output of a session.
///
/// The terminal history used to live only inside xterm.js, in the WebView; the
/// backend keeps this copy so the agent can read the session context without
/// asking the frontend. When full, the oldest bytes are discarded first.
pub struct Scrollback {
    buf: VecDeque<u8>,
    cap: usize,
}

impl Scrollback {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap.min(SCROLLBACK_CAP)),
            cap,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.cap {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.cap);
        if overflow > 0 {
            self.buf.drain(..overflow);
        }
        self.buf.extend(bytes);
    }

    /// Copies the current contents, oldest bytes first.
    pub fn snapshot(&self) -> Vec<u8> {
        let (a, b) = self.buf.as_slices();
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Default for Scrollback {
    fn default() -> Self {
        Self::new(SCROLLBACK_CAP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_everything_under_capacity() {
        let mut sb = Scrollback::new(16);
        sb.push(b"hello ");
        sb.push(b"world");
        assert_eq!(sb.snapshot(), b"hello world");
    }

    #[test]
    fn discards_oldest_bytes_when_full() {
        let mut sb = Scrollback::new(8);
        sb.push(b"12345678");
        sb.push(b"AB");
        assert_eq!(sb.snapshot(), b"345678AB");
        assert_eq!(sb.len(), 8);
    }

    #[test]
    fn push_larger_than_capacity_keeps_the_tail() {
        let mut sb = Scrollback::new(4);
        sb.push(b"abcdefgh");
        assert_eq!(sb.snapshot(), b"efgh");
    }

    #[test]
    fn snapshot_of_empty_buffer() {
        let sb = Scrollback::default();
        assert!(sb.is_empty());
        assert!(sb.snapshot().is_empty());
    }
}
