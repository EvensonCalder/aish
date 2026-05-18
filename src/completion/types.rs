#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionOptions {
    pub max_results: usize,
    pub ignore_spaces: bool,
    pub fuzzy_enabled: bool,
    pub match_threshold_percent: usize,
    pub typo_threshold_percent: usize,
}

impl Default for CompletionOptions {
    fn default() -> Self {
        Self {
            max_results: 5,
            ignore_spaces: true,
            fuzzy_enabled: true,
            match_threshold_percent: 50,
            typo_threshold_percent: 80,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenContext {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub is_first_token: bool,
    pub quote: Option<char>,
    pub path_like: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub display: String,
    pub replacement: String,
    pub is_dir: bool,
    pub source: CompletionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCompletion {
    pub line: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionSource {
    Path,
    Template,
    TemplateTypo,
    History,
    HistoryTypo,
    Executable,
    TemplatePlaceholder,
    PrivateCommand,
}
