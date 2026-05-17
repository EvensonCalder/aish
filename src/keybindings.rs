use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::de;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeybindingConfig {
    pub clear_or_cancel: Vec<KeySequenceConfig>,
    pub exit_or_delete: Vec<KeySequenceConfig>,
    pub clear_screen: Vec<KeySequenceConfig>,
    pub move_start: Vec<KeySequenceConfig>,
    pub move_end: Vec<KeySequenceConfig>,
    pub delete_to_start: Vec<KeySequenceConfig>,
    pub delete_to_end: Vec<KeySequenceConfig>,
    pub delete_previous_word: Vec<KeySequenceConfig>,
    pub delete_next_word: Vec<KeySequenceConfig>,
    pub move_previous_word: Vec<KeySequenceConfig>,
    pub move_next_word: Vec<KeySequenceConfig>,
    pub move_left: Vec<KeySequenceConfig>,
    pub move_right_or_accept_completion: Vec<KeySequenceConfig>,
    pub previous_item: Vec<KeySequenceConfig>,
    pub next_item: Vec<KeySequenceConfig>,
    pub delete_previous_char: Vec<KeySequenceConfig>,
    pub delete_next_char: Vec<KeySequenceConfig>,
    pub cancel: Vec<KeySequenceConfig>,
    pub complete_or_cycle: Vec<KeySequenceConfig>,
    pub submit: Vec<KeySequenceConfig>,
    pub history_search: Vec<KeySequenceConfig>,
    pub external_editor: Vec<KeySequenceConfig>,
    pub file_picker: Vec<KeySequenceConfig>,
    pub template_picker: Vec<KeySequenceConfig>,
    pub git_branch_picker: Vec<KeySequenceConfig>,
    pub env_var_picker: Vec<KeySequenceConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct KeybindingConfigPatch {
    clear_or_cancel: Option<Vec<KeySequenceConfig>>,
    exit_or_delete: Option<Vec<KeySequenceConfig>>,
    clear_screen: Option<Vec<KeySequenceConfig>>,
    move_start: Option<Vec<KeySequenceConfig>>,
    move_end: Option<Vec<KeySequenceConfig>>,
    delete_to_start: Option<Vec<KeySequenceConfig>>,
    delete_to_end: Option<Vec<KeySequenceConfig>>,
    delete_previous_word: Option<Vec<KeySequenceConfig>>,
    delete_next_word: Option<Vec<KeySequenceConfig>>,
    move_previous_word: Option<Vec<KeySequenceConfig>>,
    move_next_word: Option<Vec<KeySequenceConfig>>,
    move_left: Option<Vec<KeySequenceConfig>>,
    move_right_or_accept_completion: Option<Vec<KeySequenceConfig>>,
    previous_item: Option<Vec<KeySequenceConfig>>,
    next_item: Option<Vec<KeySequenceConfig>>,
    delete_previous_char: Option<Vec<KeySequenceConfig>>,
    delete_next_char: Option<Vec<KeySequenceConfig>>,
    cancel: Option<Vec<KeySequenceConfig>>,
    complete_or_cycle: Option<Vec<KeySequenceConfig>>,
    submit: Option<Vec<KeySequenceConfig>>,
    history_search: Option<Vec<KeySequenceConfig>>,
    external_editor: Option<Vec<KeySequenceConfig>>,
    file_picker: Option<Vec<KeySequenceConfig>>,
    template_picker: Option<Vec<KeySequenceConfig>>,
    git_branch_picker: Option<Vec<KeySequenceConfig>>,
    env_var_picker: Option<Vec<KeySequenceConfig>>,
}

impl<'de> Deserialize<'de> for KeybindingConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let patch = KeybindingConfigPatch::deserialize(deserializer)?;
        let mut config = Self::default();
        if let Some(value) = patch.clear_or_cancel {
            config.clear_or_cancel = value;
        }
        if let Some(value) = patch.exit_or_delete {
            config.exit_or_delete = value;
        }
        if let Some(value) = patch.clear_screen {
            config.clear_screen = value;
        }
        if let Some(value) = patch.move_start {
            config.move_start = value;
        }
        if let Some(value) = patch.move_end {
            config.move_end = value;
        }
        if let Some(value) = patch.delete_to_start {
            config.delete_to_start = value;
        }
        if let Some(value) = patch.delete_to_end {
            config.delete_to_end = value;
        }
        if let Some(value) = patch.delete_previous_word {
            config.delete_previous_word = value;
        }
        if let Some(value) = patch.delete_next_word {
            config.delete_next_word = value;
        }
        if let Some(value) = patch.move_previous_word {
            config.move_previous_word = value;
        }
        if let Some(value) = patch.move_next_word {
            config.move_next_word = value;
        }
        if let Some(value) = patch.move_left {
            config.move_left = value;
        }
        if let Some(value) = patch.move_right_or_accept_completion {
            config.move_right_or_accept_completion = value;
        }
        if let Some(value) = patch.previous_item {
            config.previous_item = value;
        }
        if let Some(value) = patch.next_item {
            config.next_item = value;
        }
        if let Some(value) = patch.delete_previous_char {
            config.delete_previous_char = value;
        }
        if let Some(value) = patch.delete_next_char {
            config.delete_next_char = value;
        }
        if let Some(value) = patch.cancel {
            config.cancel = value;
        }
        if let Some(value) = patch.complete_or_cycle {
            config.complete_or_cycle = value;
        }
        if let Some(value) = patch.submit {
            config.submit = value;
        }
        if let Some(value) = patch.history_search {
            config.history_search = value;
        }
        if let Some(value) = patch.external_editor {
            config.external_editor = value;
        }
        if let Some(value) = patch.file_picker {
            config.file_picker = value;
        }
        if let Some(value) = patch.template_picker {
            config.template_picker = value;
        }
        if let Some(value) = patch.git_branch_picker {
            config.git_branch_picker = value;
        }
        if let Some(value) = patch.env_var_picker {
            config.env_var_picker = value;
        }
        Ok(config)
    }
}

impl Default for KeybindingConfig {
    fn default() -> Self {
        Self {
            clear_or_cancel: keys(&["Ctrl-C"]),
            exit_or_delete: keys(&["Ctrl-D"]),
            clear_screen: keys(&["Ctrl-L"]),
            move_start: keys(&["Ctrl-A"]),
            move_end: keys(&["Ctrl-E"]),
            delete_to_start: keys(&["Ctrl-U"]),
            delete_to_end: keys(&["Ctrl-K"]),
            delete_previous_word: keys(&["Ctrl-W", "Alt-Backspace"]),
            delete_next_word: keys(&["Alt-D", "Alt-Delete"]),
            move_previous_word: keys(&["Alt-B", "Alt-Left"]),
            move_next_word: keys(&["Alt-F", "Alt-Right"]),
            move_left: keys(&["Left"]),
            move_right_or_accept_completion: keys(&["Right"]),
            previous_item: keys(&["Up"]),
            next_item: keys(&["Down"]),
            delete_previous_char: keys(&["Backspace"]),
            delete_next_char: keys(&["Delete"]),
            cancel: keys(&["Esc"]),
            complete_or_cycle: keys(&["Tab"]),
            submit: keys(&["Enter"]),
            history_search: keys(&["Ctrl-R"]),
            external_editor: keys(&["Ctrl-X Ctrl-E"]),
            file_picker: keys(&["Ctrl-X Ctrl-F"]),
            template_picker: keys(&["Ctrl-X Ctrl-T"]),
            git_branch_picker: keys(&["Ctrl-X Ctrl-B"]),
            env_var_picker: keys(&["Ctrl-X Ctrl-V"]),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct KeySequenceConfig(String);

impl KeySequenceConfig {
    pub fn new(raw: impl Into<String>) -> Result<Self, String> {
        let raw = raw.into();
        parse_key_sequence(&raw)?;
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn sequence(&self) -> KeySequence {
        parse_key_sequence(&self.0)
            .expect("KeySequenceConfig validates syntax during construction/deserialization")
    }
}

impl<'de> Deserialize<'de> for KeySequenceConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    modifiers: KeyModifiers,
    code: KeyCodeSpec,
}

impl KeyPress {
    pub fn is_ctrl_x(&self) -> bool {
        self.modifiers == KeyModifiers::CONTROL && self.code == KeyCodeSpec::Char('x')
    }

    fn matches_event(&self, event: KeyEvent) -> bool {
        let event_modifiers =
            event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        if event_modifiers != self.modifiers {
            return false;
        }
        self.code.matches(event.code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyCodeSpec {
    Char(char),
    Enter,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Backspace,
    Delete,
}

impl KeyCodeSpec {
    fn matches(self, code: KeyCode) -> bool {
        match (self, code) {
            (Self::Char(expected), KeyCode::Char(actual)) => actual.eq_ignore_ascii_case(&expected),
            (Self::Enter, KeyCode::Enter)
            | (Self::Tab, KeyCode::Tab)
            | (Self::Esc, KeyCode::Esc)
            | (Self::Up, KeyCode::Up)
            | (Self::Down, KeyCode::Down)
            | (Self::Left, KeyCode::Left)
            | (Self::Right, KeyCode::Right)
            | (Self::Backspace, KeyCode::Backspace)
            | (Self::Delete, KeyCode::Delete) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeySequence {
    presses: Vec<KeyPress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyBindingAction {
    ClearOrCancel,
    ExitOrDelete,
    ClearScreen,
    MoveStart,
    MoveEnd,
    DeleteToStart,
    DeleteToEnd,
    DeletePreviousWord,
    DeleteNextWord,
    MovePreviousWord,
    MoveNextWord,
    MoveLeft,
    MoveRightOrAcceptCompletion,
    PreviousItem,
    NextItem,
    DeletePreviousChar,
    DeleteNextChar,
    Cancel,
    CompleteOrCycle,
    Submit,
    HistorySearch,
    ExternalEditor,
    FilePicker,
    TemplatePicker,
    GitBranchPicker,
    EnvVarPicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyBindingMatch {
    Action(KeyBindingAction),
    Prefix(KeyPress),
    UnmatchedPending,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub keys: String,
    pub action: &'static str,
    pub implemented: bool,
}

pub fn default_keybindings() -> Vec<KeyBinding> {
    configured_keybindings(&KeybindingConfig::default())
}

pub fn configured_keybindings(config: &KeybindingConfig) -> Vec<KeyBinding> {
    binding_entries(config)
        .into_iter()
        .filter_map(|entry| {
            let keys = entry
                .sequences
                .iter()
                .map(|sequence| sequence.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            (!keys.is_empty()).then_some(KeyBinding {
                keys,
                action: entry.description,
                implemented: true,
            })
        })
        .collect()
}

pub fn match_keybinding(
    config: &KeybindingConfig,
    pending_prefix: Option<&KeyPress>,
    event: KeyEvent,
) -> KeyBindingMatch {
    let entries = binding_entries(config);
    if let Some(prefix) = pending_prefix {
        for entry in &entries {
            for sequence_config in entry.sequences {
                let sequence = sequence_config.sequence();
                if sequence.presses.len() == 2
                    && &sequence.presses[0] == prefix
                    && sequence.presses[1].matches_event(event)
                {
                    return KeyBindingMatch::Action(entry.action);
                }
            }
        }
        return KeyBindingMatch::UnmatchedPending;
    }

    for entry in &entries {
        for sequence_config in entry.sequences {
            let sequence = sequence_config.sequence();
            if sequence.presses.len() == 1 && sequence.presses[0].matches_event(event) {
                return KeyBindingMatch::Action(entry.action);
            }
        }
    }
    for entry in &entries {
        for sequence_config in entry.sequences {
            let sequence = sequence_config.sequence();
            if sequence.presses.len() == 2 && sequence.presses[0].matches_event(event) {
                return KeyBindingMatch::Prefix(sequence.presses[0].clone());
            }
        }
    }
    KeyBindingMatch::None
}

fn keys(raw: &[&str]) -> Vec<KeySequenceConfig> {
    raw.iter()
        .map(|key| KeySequenceConfig::new(*key).expect("default keybinding is valid"))
        .collect()
}

struct BindingEntry<'a> {
    action: KeyBindingAction,
    description: &'static str,
    sequences: &'a [KeySequenceConfig],
}

fn binding_entries(config: &KeybindingConfig) -> Vec<BindingEntry<'_>> {
    vec![
        BindingEntry {
            action: KeyBindingAction::ClearOrCancel,
            description: "clear the draft, cancel continuation, or reject context confirmation",
            sequences: &config.clear_or_cancel,
        },
        BindingEntry {
            action: KeyBindingAction::ExitOrDelete,
            description: "exit on empty draft or delete character",
            sequences: &config.exit_or_delete,
        },
        BindingEntry {
            action: KeyBindingAction::ClearScreen,
            description: "clear screen",
            sequences: &config.clear_screen,
        },
        BindingEntry {
            action: KeyBindingAction::MoveStart,
            description: "move to start",
            sequences: &config.move_start,
        },
        BindingEntry {
            action: KeyBindingAction::MoveEnd,
            description: "move to end",
            sequences: &config.move_end,
        },
        BindingEntry {
            action: KeyBindingAction::DeleteToStart,
            description: "delete to start",
            sequences: &config.delete_to_start,
        },
        BindingEntry {
            action: KeyBindingAction::DeleteToEnd,
            description: "delete to end",
            sequences: &config.delete_to_end,
        },
        BindingEntry {
            action: KeyBindingAction::DeletePreviousWord,
            description: "delete previous word",
            sequences: &config.delete_previous_word,
        },
        BindingEntry {
            action: KeyBindingAction::DeleteNextWord,
            description: "delete next word",
            sequences: &config.delete_next_word,
        },
        BindingEntry {
            action: KeyBindingAction::MovePreviousWord,
            description: "move previous word",
            sequences: &config.move_previous_word,
        },
        BindingEntry {
            action: KeyBindingAction::MoveNextWord,
            description: "move next word",
            sequences: &config.move_next_word,
        },
        BindingEntry {
            action: KeyBindingAction::MoveLeft,
            description: "move left",
            sequences: &config.move_left,
        },
        BindingEntry {
            action: KeyBindingAction::MoveRightOrAcceptCompletion,
            description: "move right, or accept completion at end of draft",
            sequences: &config.move_right_or_accept_completion,
        },
        BindingEntry {
            action: KeyBindingAction::PreviousItem,
            description: "browse previous draft, history item, or AI item",
            sequences: &config.previous_item,
        },
        BindingEntry {
            action: KeyBindingAction::NextItem,
            description: "browse next draft, history item, or AI item",
            sequences: &config.next_item,
        },
        BindingEntry {
            action: KeyBindingAction::DeletePreviousChar,
            description: "delete previous character",
            sequences: &config.delete_previous_char,
        },
        BindingEntry {
            action: KeyBindingAction::DeleteNextChar,
            description: "delete next character",
            sequences: &config.delete_next_char,
        },
        BindingEntry {
            action: KeyBindingAction::Cancel,
            description: "clear draft and return to draft mode",
            sequences: &config.cancel,
        },
        BindingEntry {
            action: KeyBindingAction::CompleteOrCycle,
            description: "empty draft cycles modes; non-empty draft shows or accepts completion",
            sequences: &config.complete_or_cycle,
        },
        BindingEntry {
            action: KeyBindingAction::Submit,
            description: "submit the draft or selected read-only item",
            sequences: &config.submit,
        },
        BindingEntry {
            action: KeyBindingAction::HistorySearch,
            description: "search history with the configured picker",
            sequences: &config.history_search,
        },
        BindingEntry {
            action: KeyBindingAction::ExternalEditor,
            description: "open the configured external editor",
            sequences: &config.external_editor,
        },
        BindingEntry {
            action: KeyBindingAction::FilePicker,
            description: "open the file picker",
            sequences: &config.file_picker,
        },
        BindingEntry {
            action: KeyBindingAction::TemplatePicker,
            description: "open the template picker",
            sequences: &config.template_picker,
        },
        BindingEntry {
            action: KeyBindingAction::GitBranchPicker,
            description: "open the git branch picker",
            sequences: &config.git_branch_picker,
        },
        BindingEntry {
            action: KeyBindingAction::EnvVarPicker,
            description: "open the environment variable picker",
            sequences: &config.env_var_picker,
        },
    ]
}

fn parse_key_sequence(raw: &str) -> Result<KeySequence, String> {
    let parts = raw.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return Err("key sequence cannot be empty".to_string());
    }
    if parts.len() > 2 {
        return Err(format!("key sequence has too many keys: {raw}"));
    }
    let mut presses = Vec::with_capacity(parts.len());
    for part in parts {
        presses.push(parse_key_press(part)?);
    }
    Ok(KeySequence { presses })
}

fn parse_key_press(raw: &str) -> Result<KeyPress, String> {
    let mut modifiers = KeyModifiers::empty();
    let mut key_name = None;
    for part in raw.split('-') {
        if part.is_empty() {
            return Err(format!("invalid key binding: {raw}"));
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "meta" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ if key_name.is_none() => key_name = Some(part),
            _ => return Err(format!("invalid key binding: {raw}")),
        }
    }
    let Some(key_name) = key_name else {
        return Err(format!("key binding is missing a key: {raw}"));
    };
    let code = parse_key_code(key_name)?;
    Ok(KeyPress { modifiers, code })
}

fn parse_key_code(raw: &str) -> Result<KeyCodeSpec, String> {
    match raw.to_ascii_lowercase().as_str() {
        "enter" | "return" => Ok(KeyCodeSpec::Enter),
        "tab" => Ok(KeyCodeSpec::Tab),
        "esc" | "escape" => Ok(KeyCodeSpec::Esc),
        "up" => Ok(KeyCodeSpec::Up),
        "down" => Ok(KeyCodeSpec::Down),
        "left" => Ok(KeyCodeSpec::Left),
        "right" => Ok(KeyCodeSpec::Right),
        "backspace" | "bs" => Ok(KeyCodeSpec::Backspace),
        "delete" | "del" => Ok(KeyCodeSpec::Delete),
        "space" => Ok(KeyCodeSpec::Char(' ')),
        _ => {
            let mut chars = raw.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) if ch.is_ascii() => Ok(KeyCodeSpec::Char(ch.to_ascii_lowercase())),
                _ => Err(format!("unknown key name: {raw}")),
            }
        }
    }
}

impl fmt::Display for KeySequenceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn alt(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
    }

    #[test]
    fn default_keybindings_include_common_and_advanced_bindings() {
        let bindings = default_keybindings();
        let keys: Vec<_> = bindings
            .iter()
            .map(|binding| binding.keys.as_str())
            .collect();

        for expected in [
            "Ctrl-C",
            "Ctrl-D",
            "Ctrl-L",
            "Ctrl-A",
            "Ctrl-E",
            "Ctrl-U",
            "Ctrl-K",
            "Ctrl-W / Alt-Backspace",
            "Alt-D / Alt-Delete",
            "Alt-B / Alt-Left",
            "Alt-F / Alt-Right",
            "Tab",
            "Esc",
            "Up",
            "Down",
            "Ctrl-R",
            "Ctrl-X Ctrl-E",
            "Ctrl-X Ctrl-F",
            "Ctrl-X Ctrl-T",
            "Ctrl-X Ctrl-B",
            "Ctrl-X Ctrl-V",
        ] {
            assert!(keys.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn key_sequence_config_rejects_invalid_keys() {
        let err = KeySequenceConfig::new("Ctrl-").unwrap_err();

        assert!(err.contains("invalid key binding"));
    }

    #[test]
    fn keybinding_match_resolves_single_key_actions() {
        let config = KeybindingConfig::default();

        assert_eq!(
            match_keybinding(&config, None, ctrl('r')),
            KeyBindingMatch::Action(KeyBindingAction::HistorySearch)
        );
        assert_eq!(
            match_keybinding(&config, None, alt('b')),
            KeyBindingMatch::Action(KeyBindingAction::MovePreviousWord)
        );
    }

    #[test]
    fn keybinding_match_resolves_two_key_prefixes() {
        let config = KeybindingConfig::default();
        let KeyBindingMatch::Prefix(prefix) = match_keybinding(&config, None, ctrl('x')) else {
            panic!("missing prefix match");
        };

        assert_eq!(
            match_keybinding(&config, Some(&prefix), ctrl('f')),
            KeyBindingMatch::Action(KeyBindingAction::FilePicker)
        );
        assert_eq!(
            match_keybinding(&config, Some(&prefix), ctrl('z')),
            KeyBindingMatch::UnmatchedPending
        );
    }

    #[test]
    fn configured_keybindings_skip_disabled_actions() {
        let config = KeybindingConfig {
            history_search: Vec::new(),
            ..KeybindingConfig::default()
        };

        let keys: Vec<_> = configured_keybindings(&config)
            .into_iter()
            .map(|binding| binding.keys)
            .collect();

        assert!(!keys.iter().any(|key| key == "Ctrl-R"));
    }
}
