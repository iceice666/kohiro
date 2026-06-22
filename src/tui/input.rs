//! Decoding of raw SSH channel bytes into high-level key events, plus a
//! minimal single-line text input. No terminal/russh dependency — pure logic,
//! unit-testable on its own.

/// A decoded key event. Unrecognized escapes are dropped during decoding, so
/// they never reach the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    CtrlC,
    CtrlS,
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
            0x13 => {
                keys.push(Key::CtrlS);
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
            b'C' => Some(Key::Right),
            b'D' => Some(Key::Left),
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
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
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

#[derive(Default)]
pub struct MultilineInput {
    pub value: String,
    pub cursor: usize,
    preferred_col: Option<usize>,
}

impl MultilineInput {
    pub fn handle(&mut self, key: &Key) {
        match key {
            Key::Char(ch) => self.insert(*ch),
            Key::Enter => self.insert('\n'),
            Key::Backspace => self.backspace(),
            Key::Left => self.move_left(),
            Key::Right => self.move_right(),
            Key::Up => self.move_vertical(-1),
            Key::Down => self.move_vertical(1),
            _ => {}
        }
    }

    pub fn set(&mut self, value: String) {
        self.value = value;
        self.cursor = self.value.len();
        self.preferred_col = None;
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
        self.preferred_col = None;
    }

    pub fn display_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.value.split('\n').map(str::to_owned).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let (row, col) = self.cursor_position();
        if let Some(line) = lines.get_mut(row) {
            let idx = byte_index_for_col(line, col);
            line.insert(idx, '▏');
        }
        lines
    }

    fn insert(&mut self, ch: char) {
        self.normalize_cursor();
        self.value.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.preferred_col = None;
    }

    fn backspace(&mut self) {
        self.normalize_cursor();
        let Some(prev) = previous_char_boundary(&self.value, self.cursor) else {
            return;
        };
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
        self.preferred_col = None;
    }

    fn move_left(&mut self) {
        self.normalize_cursor();
        if let Some(prev) = previous_char_boundary(&self.value, self.cursor) {
            self.cursor = prev;
        }
        self.preferred_col = None;
    }

    fn move_right(&mut self) {
        self.normalize_cursor();
        if self.cursor < self.value.len() {
            let ch = self.value[self.cursor..]
                .chars()
                .next()
                .expect("cursor at char boundary");
            self.cursor += ch.len_utf8();
        }
        self.preferred_col = None;
    }

    fn move_vertical(&mut self, delta: isize) {
        self.normalize_cursor();
        let spans = line_spans(&self.value);
        let (row, col) = self.cursor_position();
        let target_row = row.saturating_add_signed(delta).min(spans.len() - 1);
        let preferred = self.preferred_col.unwrap_or(col);
        let (start, end) = spans[target_row];
        let line = &self.value[start..end];
        self.cursor = start + byte_index_for_col(line, preferred);
        self.preferred_col = Some(preferred);
    }

    fn cursor_position(&self) -> (usize, usize) {
        let cursor = self.cursor.min(self.value.len());
        let mut row = 0;
        let mut line_start = 0;
        for (idx, ch) in self.value.char_indices() {
            if idx >= cursor {
                break;
            }
            if ch == '\n' {
                row += 1;
                line_start = idx + ch.len_utf8();
            }
        }
        let col = self.value[line_start..cursor].chars().count();
        (row, col)
    }

    fn normalize_cursor(&mut self) {
        self.cursor = self.cursor.min(self.value.len());
        while self.cursor > 0 && !self.value.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }
}

fn previous_char_boundary(value: &str, cursor: usize) -> Option<usize> {
    value[..cursor].char_indices().last().map(|(idx, _)| idx)
}

fn byte_index_for_col(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map(|(idx, _)| idx)
        .unwrap_or(line.len())
}

fn line_spans(value: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (idx, ch) in value.char_indices() {
        if ch == '\n' {
            spans.push((start, idx));
            start = idx + ch.len_utf8();
        }
    }
    spans.push((start, value.len()));
    spans
}
