//! Decoding of raw SSH channel bytes into high-level key events, plus a
//! minimal single-line text input. No terminal/russh dependency — pure logic,
//! unit-testable on its own.

/// A decoded key event. Horizontal arrows and unrecognized escapes are dropped
/// during decoding, so they never reach the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Up,
    Down,
    PageUp,
    PageDown,
    CtrlC,
}

/// Decode a burst of channel bytes (keystrokes or a paste) into key events.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            0x03 => {
                keys.push(Key::CtrlC);
                i += 1;
            }
            0x0d | 0x0a => {
                keys.push(Key::Enter);
                i += 1;
            }
            0x09 => {
                keys.push(Key::Tab);
                i += 1;
            }
            0x7f | 0x08 => {
                keys.push(Key::Backspace);
                i += 1;
            }
            0x1b => {
                if i + 1 < bytes.len() && (bytes[i + 1] == b'[' || bytes[i + 1] == b'O') {
                    let (key, consumed) = parse_escape(&bytes[i..]);
                    if let Some(key) = key {
                        keys.push(key);
                    }
                    i += consumed;
                } else {
                    keys.push(Key::Esc);
                    i += 1;
                }
            }
            0x20..=0x7e => {
                keys.push(Key::Char(b as char));
                i += 1;
            }
            _ if b >= 0x80 => {
                let start = i;
                while i < bytes.len() && bytes[i] >= 0x80 {
                    i += 1;
                }
                for ch in String::from_utf8_lossy(&bytes[start..i]).chars() {
                    keys.push(Key::Char(ch));
                }
            }
            _ => {
                // Other control bytes are ignored.
                i += 1;
            }
        }
    }
    keys
}

/// Parse a CSI (`ESC [`) or SS3 (`ESC O`) sequence beginning at `seq[0] == 0x1b`.
/// Returns the decoded key (if recognized) and the number of bytes consumed.
fn parse_escape(seq: &[u8]) -> (Option<Key>, usize) {
    if seq.len() < 2 {
        return (Some(Key::Esc), 1);
    }
    if seq[1] == b'O' {
        // SS3: ESC O <final>
        if seq.len() < 3 {
            return (None, seq.len());
        }
        let key = match seq[2] {
            b'A' => Some(Key::Up),
            b'B' => Some(Key::Down),
            _ => None,
        };
        return (key, 3);
    }
    // CSI: ESC [ <params> <final>, final byte in 0x40..=0x7e.
    let mut j = 2;
    while j < seq.len() {
        let c = seq[j];
        if (0x40..=0x7e).contains(&c) {
            let params = &seq[2..j];
            let key = match c {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'~' => match params {
                    b"5" => Some(Key::PageUp),
                    b"6" => Some(Key::PageDown),
                    _ => None,
                },
                _ => None,
            };
            return (key, j + 1);
        }
        j += 1;
    }
    (None, seq.len())
}

/// A single-line text buffer. Append/backspace at the end only; Enter/Esc are
/// handled by the owning pane, never here.
#[derive(Default)]
pub struct TextInput {
    pub value: String,
}

impl TextInput {
    pub fn handle(&mut self, key: &Key) {
        match key {
            Key::Char(ch) => self.value.push(*ch),
            Key::Backspace => {
                self.value.pop();
            }
            _ => {}
        }
    }

    pub fn clear(&mut self) {
        self.value.clear();
    }
}
