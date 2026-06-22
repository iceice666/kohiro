use kohiro::tui::input::{Key, MultilineInput, decode};

#[test]
fn decodes_control_keys_arrows_and_text() {
    assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
    assert_eq!(decode(b"\x1b[B"), vec![Key::Down]);
    assert_eq!(decode(b"\x1b[C"), vec![Key::Right]);
    assert_eq!(decode(b"\x1b[D"), vec![Key::Left]);
    assert_eq!(decode(b"\x1b[5~"), vec![Key::PageUp]);
    assert_eq!(decode(b"\x1b[6~"), vec![Key::PageDown]);
    assert_eq!(decode(b"\r"), vec![Key::Enter]);
    assert_eq!(decode(b"\n"), vec![Key::Enter]);
    assert_eq!(decode(b"\x7f"), vec![Key::Backspace]);
    assert_eq!(decode(b"\x1b"), vec![Key::Esc]);
    assert_eq!(decode(b"\x03"), vec![Key::CtrlC]);
    assert_eq!(decode(b"hi"), vec![Key::Char('h'), Key::Char('i')]);
    assert_eq!(decode(b"\t"), vec![Key::Tab]);
    assert_eq!(decode(b"\x13"), vec![Key::CtrlS]);
}

#[test]
fn decodes_application_mode_arrows() {
    assert_eq!(decode(b"\x1bOA"), vec![Key::Up]);
    assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
    assert_eq!(decode(b"\x1bOC"), vec![Key::Right]);
    assert_eq!(decode(b"\x1bOD"), vec![Key::Left]);
}

#[test]
fn decodes_mixed_burst() {
    assert_eq!(
        decode(b"a\x1b[Bx"),
        vec![Key::Char('a'), Key::Down, Key::Char('x')]
    );
}

#[test]
fn multiline_input_accepts_newlines_and_backspace() {
    let mut input = MultilineInput::default();
    for key in [
        Key::Char('a'),
        Key::Enter,
        Key::Char('b'),
        Key::Backspace,
        Key::Char('c'),
    ] {
        input.handle(&key);
    }
    assert_eq!(input.value, "a\nc");
}

#[test]
fn multiline_input_moves_cursor_and_edits_at_cursor() {
    let mut input = MultilineInput::default();
    input.set("abc\ndef".into());

    input.handle(&Key::Up);
    input.handle(&Key::Left);
    input.handle(&Key::Char('X'));

    assert_eq!(input.value, "abXc\ndef");
    assert_eq!(input.display_lines(), vec!["abX▏c", "def"]);
}

#[test]
fn multiline_input_preserves_preferred_column_across_short_lines() {
    let mut input = MultilineInput::default();
    input.set("abcd\nx\nyz".into());

    input.handle(&Key::Up);
    input.handle(&Key::Up);

    assert_eq!(input.display_lines(), vec!["ab▏cd", "x", "yz"]);
}
