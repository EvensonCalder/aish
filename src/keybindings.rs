#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: &'static str,
    pub action: &'static str,
}

pub const DEFAULT_KEYBINDINGS: &[KeyBinding] = &[
    KeyBinding {
        key: "Ctrl-C",
        action: "clear or cancel draft",
    },
    KeyBinding {
        key: "Ctrl-D",
        action: "exit on empty draft or delete character",
    },
    KeyBinding {
        key: "Ctrl-L",
        action: "clear screen",
    },
    KeyBinding {
        key: "Ctrl-A",
        action: "move to start",
    },
    KeyBinding {
        key: "Ctrl-E",
        action: "move to end",
    },
    KeyBinding {
        key: "Ctrl-U",
        action: "delete to start",
    },
    KeyBinding {
        key: "Ctrl-K",
        action: "delete to end",
    },
    KeyBinding {
        key: "Ctrl-W",
        action: "delete previous word",
    },
    KeyBinding {
        key: "Alt-B / Alt-Left",
        action: "move previous word",
    },
    KeyBinding {
        key: "Alt-F / Alt-Right",
        action: "move next word",
    },
    KeyBinding {
        key: "Tab",
        action: "cycle mode on empty draft",
    },
    KeyBinding {
        key: "Enter",
        action: "submit selected command or draft",
    },
    KeyBinding {
        key: "Up / Down",
        action: "browse history or AI selections",
    },
    KeyBinding {
        key: "Ctrl-R",
        action: "history search reserved",
    },
    KeyBinding {
        key: "Esc",
        action: "cancel temporary mode reserved",
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-E",
        action: "external editor reserved",
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-F",
        action: "file picker reserved",
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-T",
        action: "template picker reserved",
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-B",
        action: "git branch picker reserved",
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-V",
        action: "environment variable picker reserved",
    },
];

pub fn default_keybindings() -> &'static [KeyBinding] {
    DEFAULT_KEYBINDINGS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keybindings_include_common_and_advanced_bindings() {
        let keys: Vec<_> = default_keybindings()
            .iter()
            .map(|binding| binding.key)
            .collect();

        for expected in [
            "Ctrl-C",
            "Ctrl-D",
            "Ctrl-L",
            "Ctrl-A",
            "Ctrl-E",
            "Ctrl-U",
            "Ctrl-K",
            "Ctrl-W",
            "Alt-B / Alt-Left",
            "Alt-F / Alt-Right",
            "Tab",
            "Esc",
            "Up / Down",
            "Ctrl-R",
            "Ctrl-X Ctrl-E",
            "Ctrl-X Ctrl-F",
            "Ctrl-X Ctrl-T",
            "Ctrl-X Ctrl-B",
            "Ctrl-X Ctrl-V",
        ] {
            assert!(keys.contains(&expected));
        }
    }
}
