use kohiro::tui::input::{decode, Key};

#[test]
fn decodes_control_keys_arrows_and_text() {
    assert_eq!(decode(b"\x1b[A"), vec![Key::Up]);
    assert_eq!(decode(b"\x1b[B"), vec![Key::Down]);
    assert_eq!(decode(b"\x1b[5~"), vec![Key::PageUp]);
    assert_eq!(decode(b"\x1b[6~"), vec![Key::PageDown]);
    assert_eq!(decode(b"\r"), vec![Key::Enter]);
    assert_eq!(decode(b"\n"), vec![Key::Enter]);
    assert_eq!(decode(b"\x7f"), vec![Key::Backspace]);
    assert_eq!(decode(b"\x1b"), vec![Key::Esc]);
    assert_eq!(decode(b"\x03"), vec![Key::CtrlC]);
    assert_eq!(decode(b"hi"), vec![Key::Char('h'), Key::Char('i')]);
    assert_eq!(decode(b"\t"), vec![Key::Tab]);
}

#[test]
fn ignores_horizontal_arrows_and_decodes_application_mode() {
    // Left/Right are dropped; SS3 up/down are decoded.
    assert_eq!(decode(b"\x1b[C"), vec![]);
    assert_eq!(decode(b"\x1b[D"), vec![]);
    assert_eq!(decode(b"\x1bOA"), vec![Key::Up]);
    assert_eq!(decode(b"\x1bOB"), vec![Key::Down]);
}

#[test]
fn decodes_mixed_burst() {
    assert_eq!(
        decode(b"a\x1b[Bx"),
        vec![Key::Char('a'), Key::Down, Key::Char('x')]
    );
}
