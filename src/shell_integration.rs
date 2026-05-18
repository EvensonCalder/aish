use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn passthrough_key_bytes(key: KeyEvent) -> Option<String> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char(ch)) if ch.is_ascii_alphabetic() => {
            let code = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
            Some(char::from(code).to_string())
        }
        (KeyModifiers::ALT, KeyCode::Char(ch)) => Some(format!("\x1b{ch}")),
        (_, KeyCode::Char(ch)) => Some(ch.to_string()),
        (_, KeyCode::Enter) => Some("\r".to_string()),
        (_, KeyCode::Tab) => Some("\t".to_string()),
        (_, KeyCode::Backspace) => Some("\x7f".to_string()),
        (_, KeyCode::Esc) => Some("\x1b".to_string()),
        (_, KeyCode::Up) => Some("\x1b[A".to_string()),
        (_, KeyCode::Down) => Some("\x1b[B".to_string()),
        (_, KeyCode::Right) => Some("\x1b[C".to_string()),
        (_, KeyCode::Left) => Some("\x1b[D".to_string()),
        (_, KeyCode::Delete) => Some("\x1b[3~".to_string()),
        _ => None,
    }
}
