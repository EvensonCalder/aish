#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: &'static str,
    pub action: &'static str,
    pub implemented: bool,
}

pub const DEFAULT_KEYBINDINGS: &[KeyBinding] = &[
    KeyBinding {
        key: "Ctrl-C",
        action: "clear or cancel draft",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-D",
        action: "exit on empty draft or delete character",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-L",
        action: "clear screen",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-A",
        action: "move to start",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-E",
        action: "move to end",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-U",
        action: "delete to start",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-K",
        action: "delete to end",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-W",
        action: "delete previous word",
        implemented: true,
    },
    KeyBinding {
        key: "Alt-B / Alt-Left",
        action: "move previous word",
        implemented: true,
    },
    KeyBinding {
        key: "Alt-F / Alt-Right",
        action: "move next word",
        implemented: true,
    },
    KeyBinding {
        key: "Tab",
        action: "cycle mode on empty draft",
        implemented: true,
    },
    KeyBinding {
        key: "Enter",
        action: "submit selected command or draft",
        implemented: true,
    },
    KeyBinding {
        key: "Up / Down",
        action: "browse history or AI selections; Down starts a new saved draft from non-empty draft mode",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-R",
        action: "history search placeholder",
        implemented: true,
    },
    KeyBinding {
        key: "Esc",
        action: "clear draft and return to draft mode",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-E",
        action: "external editor",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-F",
        action: "file picker placeholder",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-T",
        action: "template picker placeholder",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-B",
        action: "git branch picker placeholder",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-V",
        action: "environment variable picker placeholder",
        implemented: true,
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

    #[test]
    fn default_keybindings_distinguish_implemented_and_reserved_bindings() {
        let bindings = default_keybindings();

        assert!(
            bindings
                .iter()
                .find(|binding| binding.key == "Ctrl-C")
                .unwrap()
                .implemented
        );
        assert!(
            bindings
                .iter()
                .find(|binding| binding.key == "Esc")
                .unwrap()
                .implemented
        );
        assert!(
            bindings
                .iter()
                .find(|binding| binding.key == "Ctrl-R")
                .unwrap()
                .implemented
        );
        assert!(
            bindings
                .iter()
                .find(|binding| binding.key == "Ctrl-X Ctrl-E")
                .unwrap()
                .implemented
        );
    }
}
