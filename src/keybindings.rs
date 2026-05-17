#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub key: &'static str,
    pub action: &'static str,
    pub implemented: bool,
}

pub const DEFAULT_KEYBINDINGS: &[KeyBinding] = &[
    KeyBinding {
        key: "Ctrl-C",
        action: "clear the draft, cancel continuation, or reject context confirmation",
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
        key: "Alt-Backspace",
        action: "delete previous word",
        implemented: true,
    },
    KeyBinding {
        key: "Alt-D / Alt-Delete",
        action: "delete next word",
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
        action: "empty draft cycles modes; non-empty draft shows or accepts completion",
        implemented: true,
    },
    KeyBinding {
        key: "Enter",
        action: "submit the draft or selected read-only item",
        implemented: true,
    },
    KeyBinding {
        key: "Up / Down",
        action: "browse saved drafts, history, or AI selections",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-R",
        action: "search history with the configured picker",
        implemented: true,
    },
    KeyBinding {
        key: "Esc",
        action: "clear draft and return to draft mode",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-E",
        action: "open the configured external editor",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-F",
        action: "open the file picker",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-T",
        action: "open the template picker",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-B",
        action: "open the git branch picker",
        implemented: true,
    },
    KeyBinding {
        key: "Ctrl-X Ctrl-V",
        action: "open the environment variable picker",
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
            "Alt-Backspace",
            "Alt-D / Alt-Delete",
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
