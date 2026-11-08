use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
pub const DEFAULTS: &[(&str, &str)] = &[
    ("quit", "q"),
    ("toggle", "Space"),
    ("next", "n"),
    ("previous", "p"),
    ("search", "/"),
    ("down", "j"),
    ("up", "k"),
    ("select", "Enter"),
    ("back", "Esc"),
    ("focus", "Tab"),
    ("volume_up", "+"),
    ("volume_down", "-"),
    ("shuffle", "s"),
    ("repeat", "r"),
    ("equalizer", "e"),
    ("visualizer", "v"),
    ("lyrics", "l"),
    ("settings", ","),
    ("help", "?"),
    ("command", ":"),
    ("enqueue", "a"),
    ("play_next", "A"),
    ("remove", "Delete"),
    ("move_up", "K"),
    ("move_down", "J"),
    ("page", "PageDown"),
    ("mute", "m"),
    ("seek_back", "Left"),
    ("seek_forward", "Right"),
];
pub fn parse_key(s: &str) -> Option<KeyEvent> {
    let (mods, name) = if let Some(k) = s.strip_prefix("ctrl-") {
        (KeyModifiers::CONTROL, k)
    } else if let Some(k) = s.strip_prefix("alt-") {
        (KeyModifiers::ALT, k)
    } else {
        (KeyModifiers::NONE, s)
    };
    let code = match name {
        "Space" => KeyCode::Char(' '),
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "BackTab" => KeyCode::BackTab,
        "Delete" => KeyCode::Delete,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "PageDown" => KeyCode::PageDown,
        "PageUp" => KeyCode::PageUp,
        _ => {
            let mut c = name.chars();
            let first = c.next()?;
            if c.next().is_some() {
                return None;
            }
            KeyCode::Char(first)
        }
    };
    Some(KeyEvent::new(code, mods))
}
pub fn validate_bindings(b: &BTreeMap<String, String>) -> anyhow::Result<()> {
    for (action, key) in b {
        anyhow::ensure!(
            DEFAULTS.iter().any(|(a, _)| a == action),
            "unknown keybinding action: {action}"
        );
        anyhow::ensure!(parse_key(key).is_some(), "invalid key name for {action}");
    }
    let mut seen = Vec::new();
    for (a, k) in DEFAULTS {
        let key = parse_key(b.get(*a).map_or(*k, String::as_str))
            .ok_or_else(|| anyhow::anyhow!("invalid keybinding"))?;
        anyhow::ensure!(!seen.contains(&key), "duplicate keybinding: {a}");
        seen.push(key);
    }
    Ok(())
}
pub fn action_for(mut event: KeyEvent, b: &BTreeMap<String, String>) -> Option<&'static str> {
    if matches!(event.code, KeyCode::Char(_)) {
        event.modifiers.remove(KeyModifiers::SHIFT);
    }
    DEFAULTS.iter().find_map(|(a, k)| {
        let key = parse_key(b.get(*a).map_or(*k, String::as_str))?;
        (event.code == key.code && event.modifiers == key.modifiers).then_some(*a)
    })
}
