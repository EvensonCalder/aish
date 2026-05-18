use super::TokenContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletionMatcher {
    ignore_spaces: bool,
    match_threshold_percent: usize,
    typo_threshold_percent: usize,
}

impl CompletionMatcher {
    pub(crate) fn new(
        ignore_spaces: bool,
        match_threshold_percent: usize,
        typo_threshold_percent: usize,
    ) -> Self {
        Self {
            ignore_spaces,
            match_threshold_percent,
            typo_threshold_percent,
        }
    }

    pub(crate) fn prefix_matches(&self, candidate: &str, typed: &str) -> bool {
        matches_completion_prefix_with_threshold(
            candidate,
            typed,
            self.ignore_spaces,
            self.match_threshold_percent,
        )
    }

    pub(crate) fn word_prefix_matches(&self, candidate: &str, typed: &str) -> bool {
        word_prefix_matches(candidate, typed, self.ignore_spaces)
    }

    pub(crate) fn typo_similarity_percent(&self, candidate: &str, typed: &str) -> usize {
        typo_similarity_percent(candidate, typed, self.ignore_spaces)
    }

    pub(crate) fn typo_matches(&self, candidate: &str, typed: &str) -> bool {
        self.typo_similarity_percent(candidate, typed) >= self.typo_threshold_percent
    }

    pub(crate) fn words_match_threshold(
        &self,
        candidate_words: &[String],
        typed_words: &[String],
    ) -> bool {
        words_match_threshold_by(
            candidate_words,
            typed_words,
            self.match_threshold_percent,
            |candidate, typed| self.word_prefix_matches(candidate, typed),
        )
    }

    pub(crate) fn template_words_match_threshold(
        &self,
        template_words: &[String],
        typed_words: &[String],
    ) -> bool {
        words_match_threshold_by(
            template_words,
            typed_words,
            self.match_threshold_percent,
            |candidate, typed| {
                template_word_is_placeholder(candidate)
                    || self.word_prefix_matches(candidate, typed)
            },
        )
    }

    pub(crate) fn words_match_threshold_with_typos(
        &self,
        candidate_words: &[String],
        typed_words: &[String],
    ) -> bool {
        words_match_threshold_with_typo_usage_by(
            candidate_words,
            typed_words,
            self.match_threshold_percent,
            |candidate, typed| self.word_prefix_matches(candidate, typed),
            |candidate, typed| self.typo_matches(candidate, typed),
        )
    }

    pub(crate) fn template_words_match_threshold_with_typos(
        &self,
        template_words: &[String],
        typed_words: &[String],
    ) -> bool {
        words_match_threshold_with_typo_usage_by(
            template_words,
            typed_words,
            self.match_threshold_percent,
            |candidate, typed| {
                template_word_is_placeholder(candidate)
                    || self.word_prefix_matches(candidate, typed)
            },
            |candidate, typed| {
                !template_word_is_placeholder(candidate) && self.typo_matches(candidate, typed)
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TemplatePlaceholderWord {
    pub(super) raw: String,
    pub(super) name: String,
}

pub(super) fn template_placeholder_words(body: &str) -> Vec<TemplatePlaceholderWord> {
    super::parser::split_shell_like_words(body)
        .into_iter()
        .filter_map(|word| {
            let name = template_word_placeholder_name(&word)?.to_string();
            Some(TemplatePlaceholderWord { raw: word, name })
        })
        .collect()
}

fn words_match_threshold_by(
    candidate_words: &[String],
    typed_words: &[String],
    match_threshold_percent: usize,
    mut word_matches: impl FnMut(&str, &str) -> bool,
) -> bool {
    if typed_words.is_empty() || candidate_words.len() < typed_words.len() {
        return false;
    }
    let matched = typed_words
        .iter()
        .zip(candidate_words.iter())
        .filter(|(typed, candidate)| word_matches(candidate, typed))
        .count();
    percent(matched, typed_words.len()) >= match_threshold_percent.min(100)
}

fn words_match_threshold_with_typo_usage_by(
    candidate_words: &[String],
    typed_words: &[String],
    match_threshold_percent: usize,
    mut structural_matches: impl FnMut(&str, &str) -> bool,
    mut typo_matches: impl FnMut(&str, &str) -> bool,
) -> bool {
    if typed_words.is_empty() || candidate_words.len() < typed_words.len() {
        return false;
    }
    let mut matched = 0;
    let mut used_typo = false;
    for (typed, candidate) in typed_words.iter().zip(candidate_words.iter()) {
        if structural_matches(candidate, typed) {
            matched += 1;
        } else if typo_matches(candidate, typed) {
            matched += 1;
            used_typo = true;
        }
    }
    used_typo && percent(matched, typed_words.len()) >= match_threshold_percent.min(100)
}

pub(super) fn word_prefix_matches(candidate: &str, typed: &str, ignore_spaces: bool) -> bool {
    if typed.is_empty() {
        return false;
    }
    if ignore_spaces {
        return remove_spaces(candidate).starts_with(&remove_spaces(typed));
    }
    candidate.starts_with(typed)
}

pub(super) fn typo_similarity_percent(candidate: &str, typed: &str, ignore_spaces: bool) -> usize {
    let candidate = if ignore_spaces {
        remove_spaces(candidate)
    } else {
        candidate.to_string()
    };
    let typed = if ignore_spaces {
        remove_spaces(typed)
    } else {
        typed.to_string()
    };
    if candidate.is_empty() || typed.is_empty() {
        return 0;
    }
    let distance = edit_distance_chars(&candidate, &typed);
    let max_len = candidate.chars().count().max(typed.chars().count());
    percent(max_len.saturating_sub(distance), max_len)
}

pub(super) fn edit_distance_chars(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

pub(super) fn template_replacement_for_index(
    template_words: &[String],
    current_word_index: usize,
    token: &TokenContext,
    ignore_spaces: bool,
    match_threshold_percent: usize,
) -> String {
    let template_word = &template_words[current_word_index];
    let rest = &template_words[current_word_index + 1..];
    if token.text.is_empty() || !template_word_is_placeholder(template_word) {
        return join_words(&template_words[current_word_index..]);
    }

    let placeholder_name = template_word_placeholder_name(template_word).unwrap_or_default();
    if token.text.starts_with('{')
        || matches_completion_prefix_with_threshold(
            template_word,
            &token.text,
            ignore_spaces,
            match_threshold_percent,
        )
        || matches_completion_prefix_with_threshold(
            placeholder_name,
            &token.text,
            ignore_spaces,
            match_threshold_percent,
        )
    {
        return join_words(&template_words[current_word_index..]);
    }

    join_words_with_first(token.text.as_str(), rest)
}

pub(super) fn template_word_is_placeholder(word: &str) -> bool {
    template_word_placeholder_name(word).is_some()
}

fn template_word_placeholder_name(word: &str) -> Option<&str> {
    let candidate = word.strip_prefix('{')?.strip_suffix('}')?;
    let name = candidate
        .strip_suffix("...")
        .or_else(|| candidate.split_once(':').map(|(name, _)| name))
        .unwrap_or(candidate);
    is_placeholder_name(name).then_some(name)
}

fn is_placeholder_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn join_words_with_first(first: &str, rest: &[String]) -> String {
    if rest.is_empty() {
        return first.to_string();
    }
    format!("{} {}", first, join_words(rest))
}

pub(super) fn join_words(words: &[String]) -> String {
    words.join(" ")
}

pub fn matches_completion_prefix(candidate: &str, prefix: &str, ignore_spaces: bool) -> bool {
    matches_completion_prefix_with_threshold(candidate, prefix, ignore_spaces, 50)
}

pub fn matches_completion_prefix_with_threshold(
    candidate: &str,
    prefix: &str,
    ignore_spaces: bool,
    _match_threshold_percent: usize,
) -> bool {
    if prefix.is_empty() {
        return false;
    }
    if !ignore_spaces {
        return candidate.starts_with(prefix);
    }

    let compact_prefix = remove_spaces(prefix);
    let compact_candidate = remove_spaces(candidate);
    if compact_candidate.starts_with(&compact_prefix) {
        return true;
    }

    let mut candidate_words = candidate.split_whitespace();
    for prefix_part in prefix.split_whitespace() {
        let Some(candidate_word) = candidate_words.next() else {
            return false;
        };
        if !candidate_word.starts_with(prefix_part) {
            return false;
        }
    }
    true
}

fn percent(numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        return 0;
    }
    numerator * 100 / denominator
}

fn remove_spaces(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}
