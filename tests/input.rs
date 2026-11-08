use crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};
use rustune::input::{InputEvent, Parser};
#[test]
fn oversized_or_unterminated_paste_remains_bounded_and_never_yields_a_prefix() {
    let mut parser = Parser::default();
    for b in b"\x1b[200~" {
        assert!(parser.feed(*b).is_none());
    }
    for _ in 0..1_000_000 {
        assert!(parser.feed(b'a').is_none());
        assert!(parser.retained_bytes() <= 16 * 1024 + 64);
    }
    let mut output = None;
    for b in b"\x1b[201~" {
        if let Some(e) = parser.feed(*b) {
            output = Some(e);
        }
    }
    assert!(matches!(output, Some(InputEvent::Overflow)));
    assert!(matches!(parser.feed(b'\r'),Some(InputEvent::Key(k)) if k.code==KeyCode::Enter));
}
#[test]
fn paste_unicode_keys_arrows_function_keys_and_mouse_are_parsed_incrementally() {
    let mut parser = Parser::default();
    let mut events = Vec::new();
    for b in "\x1b[200~example-token\x1b[201~é\x1b[A\x1bOQ\x1b[<0;12;20M\u{3}".bytes() {
        if let Some(e) = parser.feed(b) {
            events.push(e);
        }
    }
    assert!(matches!(&events[0],InputEvent::Paste(s) if s.as_str()=="example-token"));
    assert!(matches!(events[1],InputEvent::Key(k) if k.code==KeyCode::Char('é')));
    assert!(matches!(events[2],InputEvent::Key(k) if k.code==KeyCode::Up));
    assert!(matches!(events[3],InputEvent::Key(k) if k.code==KeyCode::F(2)));
    assert!(
        matches!(events[4],InputEvent::Mouse(m) if m.column==11 && m.row==19 && m.kind==MouseEventKind::Down(crossterm::event::MouseButton::Left))
    );
    assert!(
        matches!(events[5],InputEvent::Key(k) if k.code==KeyCode::Char('c') && k.modifiers==KeyModifiers::CONTROL)
    );
}

#[test]
fn control_c_cancels_an_unterminated_escape_or_paste_without_retaining_input() {
    for prefix in [
        b"\x1b[".as_slice(),
        b"\x1b[200~unfinished-private-input".as_slice(),
    ] {
        let mut parser = Parser::default();
        for b in prefix {
            parser.feed(*b);
        }
        assert!(
            matches!(parser.feed(3),Some(InputEvent::Key(k)) if k.code==KeyCode::Char('c') && k.modifiers==KeyModifiers::CONTROL)
        );
        assert_eq!(parser.retained_bytes(), 0);
    }
}
