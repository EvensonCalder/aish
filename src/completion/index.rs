use crate::history::HistoryEntry;
use crate::templates::TemplateEntry;

use super::matching::template_placeholder_words;
use super::parser::{ShellWord, shell_like_words, split_shell_like_words};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedHistoryEntry {
    pub(crate) entry: HistoryEntry,
    pub(crate) words: Vec<String>,
    pub(crate) raw_words: Vec<String>,
    pub(crate) arguments: Vec<ShellWord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedTemplateEntry {
    pub(crate) entry: TemplateEntry,
    pub(crate) id: String,
    pub(crate) words: Vec<String>,
    pub(crate) placeholders: Vec<IndexedTemplatePlaceholder>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedTemplatePlaceholder {
    pub(crate) raw: String,
    pub(crate) name: String,
}

pub(crate) fn index_history_entries(
    history_newest_first: &[HistoryEntry],
) -> Vec<IndexedHistoryEntry> {
    history_newest_first
        .iter()
        .cloned()
        .map(|entry| {
            let words = shell_like_words(&entry.command);
            IndexedHistoryEntry {
                raw_words: words.iter().map(|word| word.raw.clone()).collect(),
                arguments: words.iter().skip(1).cloned().collect(),
                words: words.into_iter().map(|word| word.value).collect(),
                entry,
            }
        })
        .collect()
}

pub(crate) fn index_template_entries(templates: &[TemplateEntry]) -> Vec<IndexedTemplateEntry> {
    templates
        .iter()
        .cloned()
        .map(|entry| {
            let placeholders = template_placeholder_words(&entry.body)
                .into_iter()
                .map(|placeholder| IndexedTemplatePlaceholder {
                    raw: placeholder.raw,
                    name: placeholder.name,
                })
                .collect();
            IndexedTemplateEntry {
                id: entry.id(),
                words: split_shell_like_words(&entry.body),
                placeholders,
                entry,
            }
        })
        .collect()
}
