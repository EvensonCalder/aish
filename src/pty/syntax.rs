use anyhow::Result;

use super::ContinuationCheck;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellLexicalState {
    Normal,
    SingleQuoted,
    DoubleQuoted,
}

pub(super) fn input_needs_more_lines(
    _shell_program: &str,
    input: &str,
) -> Result<ContinuationCheck> {
    Ok(lexical_continuation_check(input))
}

pub(super) fn lexical_continuation_check(input: &str) -> ContinuationCheck {
    let state = lexical_state(input);
    match state.quote {
        ShellLexicalState::SingleQuoted => ContinuationCheck {
            needs_more: true,
            prompt: Some("quote> ".to_string()),
        },
        ShellLexicalState::DoubleQuoted => ContinuationCheck {
            needs_more: true,
            prompt: Some("dquote> ".to_string()),
        },
        ShellLexicalState::Normal if state.escaped => ContinuationCheck {
            needs_more: true,
            prompt: Some("> ".to_string()),
        },
        ShellLexicalState::Normal => ContinuationCheck {
            needs_more: false,
            prompt: None,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LexicalState {
    quote: ShellLexicalState,
    escaped: bool,
}

fn lexical_state(input: &str) -> LexicalState {
    let mut quote = ShellLexicalState::Normal;
    let mut escaped = false;

    for ch in input.chars() {
        if escaped {
            escaped = false;
            continue;
        }

        match quote {
            ShellLexicalState::Normal => match ch {
                '\\' => escaped = true,
                '\'' => quote = ShellLexicalState::SingleQuoted,
                '"' => quote = ShellLexicalState::DoubleQuoted,
                _ => {}
            },
            ShellLexicalState::SingleQuoted => {
                if ch == '\'' {
                    quote = ShellLexicalState::Normal;
                }
            }
            ShellLexicalState::DoubleQuoted => match ch {
                '\\' => escaped = true,
                '"' => quote = ShellLexicalState::Normal,
                _ => {}
            },
        }
    }

    LexicalState { quote, escaped }
}
