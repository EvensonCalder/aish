#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Draft,
    History,
    Ai,
    CommandRunning,
    Passthrough,
    ExternalEditor,
    PasteReviewEditor,
    Picker,
    UnlockPassthrough,
}

impl Mode {
    pub fn symbol(self) -> char {
        match self {
            Self::Draft => '>',
            Self::History => '$',
            Self::Ai => '%',
            Self::CommandRunning
            | Self::Passthrough
            | Self::ExternalEditor
            | Self::PasteReviewEditor
            | Self::Picker
            | Self::UnlockPassthrough => '>',
        }
    }

    pub fn next_primary(self) -> Self {
        match self {
            Self::Draft => Self::History,
            Self::History => Self::Ai,
            Self::Ai => Self::Draft,
            _ => Self::Draft,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_modes_cycle_deterministically() {
        let modes = [
            Mode::Draft,
            Mode::Draft.next_primary(),
            Mode::Draft.next_primary().next_primary(),
            Mode::Draft.next_primary().next_primary().next_primary(),
        ];
        assert_eq!(modes, [Mode::Draft, Mode::History, Mode::Ai, Mode::Draft]);
    }

    #[test]
    fn primary_mode_symbols_match_spec() {
        assert_eq!(Mode::Draft.symbol(), '>');
        assert_eq!(Mode::History.symbol(), '$');
        assert_eq!(Mode::Ai.symbol(), '%');
    }
}
