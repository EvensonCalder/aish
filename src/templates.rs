use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::history::{JsonlLineError, JsonlLoad, append_jsonl, load_jsonl, rewrite_jsonl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateEntry {
    pub body: String,
}

impl TemplateEntry {
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }

    pub fn id(&self) -> String {
        template_id(&self.body)
    }
}

pub fn template_id(body: &str) -> String {
    format!("tpl-{:016x}", fnv1a64(body.as_bytes()))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn append_template(path: &Path, entry: &TemplateEntry) -> Result<()> {
    append_jsonl(path, entry)
}

pub fn load_templates(path: &Path) -> Result<JsonlLoad<TemplateEntry>> {
    load_jsonl(path)
}

pub fn find_template_by_id(path: &Path, id: &str) -> Result<JsonlLoad<TemplateEntry>> {
    let mut loaded = load_templates(path)?;
    loaded.items = loaded
        .items
        .into_iter()
        .rev()
        .find(|template| template.id() == id)
        .into_iter()
        .collect();
    Ok(loaded)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRemoval {
    pub removed: usize,
    pub remaining: Vec<TemplateEntry>,
    pub errors: Vec<JsonlLineError>,
}

pub fn remove_templates_by_id(path: &Path, id: &str) -> Result<TemplateRemoval> {
    let loaded = load_templates(path)?;
    let before = loaded.items.len();
    let remaining: Vec<_> = loaded
        .items
        .into_iter()
        .filter(|template| template.id() != id)
        .collect();
    let removed = before - remaining.len();
    rewrite_jsonl(path, &remaining)?;

    Ok(TemplateRemoval {
        removed,
        remaining,
        errors: loaded.errors,
    })
}

pub fn replace_template_by_id(
    path: &Path,
    existing_id: &str,
    entry: TemplateEntry,
) -> Result<TemplateRemoval> {
    let loaded = load_templates(path)?;
    let before = loaded.items.len();
    let mut remaining: Vec<_> = loaded
        .items
        .into_iter()
        .filter(|template| template.id() != existing_id)
        .collect();
    let removed = before - remaining.len();
    remaining.push(entry);
    rewrite_jsonl(path, &remaining)?;

    Ok(TemplateRemoval {
        removed,
        remaining,
        errors: loaded.errors,
    })
}

pub fn template_placeholders(body: &str) -> Vec<String> {
    let mut placeholders = Vec::new();
    for placeholder in parse_template_placeholders(body) {
        if !placeholders.iter().any(|item| item == &placeholder.name) {
            placeholders.push(placeholder.name);
        }
    }
    placeholders
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPlaceholder {
    raw: String,
    name: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceholderSpan {
    pub start: usize,
    pub end: usize,
}

pub fn template_placeholder_spans(body: &str) -> Vec<PlaceholderSpan> {
    parse_template_placeholders(body)
        .into_iter()
        .map(|placeholder| PlaceholderSpan {
            start: placeholder.start,
            end: placeholder.end,
        })
        .collect()
}

fn parse_template_placeholders(body: &str) -> Vec<ParsedPlaceholder> {
    let mut placeholders = Vec::new();
    let mut offset = 0;
    while let Some(relative_start) = body[offset..].find('{') {
        let start = offset + relative_start;
        let content_start = start + 1;
        let Some(relative_end) = body[content_start..].find('}') else {
            break;
        };
        let end = content_start + relative_end;
        let candidate = &body[content_start..end];
        if let Some(name) = placeholder_name(candidate) {
            placeholders.push(ParsedPlaceholder {
                raw: format!("{{{candidate}}}"),
                name: name.to_string(),
                start,
                end: end + 1,
            });
        }
        offset = end + 1;
    }
    placeholders
}

pub fn apply_template_values(body: &str, values: &HashMap<String, String>) -> String {
    apply_template_values_with_usage(body, values).0
}

pub fn apply_template_values_with_usage(
    body: &str,
    values: &HashMap<String, String>,
) -> (String, Vec<String>) {
    let mut rendered = body.to_string();
    let mut used = Vec::new();
    for placeholder in parse_template_placeholders(body) {
        if let Some(value) = values.get(&placeholder.name) {
            rendered = rendered.replace(&placeholder.raw, value);
            if !used.iter().any(|item| item == &placeholder.name) {
                used.push(placeholder.name);
            }
        }
    }
    (rendered, used)
}

fn is_placeholder_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn placeholder_name(candidate: &str) -> Option<&str> {
    let name = candidate
        .strip_suffix("...")
        .or_else(|| candidate.split_once(':').map(|(name, _)| name))
        .unwrap_or(candidate);
    is_placeholder_name(name).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_entry_roundtrips_through_jsonl() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        let entry = TemplateEntry::new("rsync -avz {from} {to}");

        append_template(&path, &entry).unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items, [entry]);
    }

    #[test]
    fn template_id_is_a_stable_body_hash() {
        assert_eq!(
            template_id("echo {something}"),
            template_id("echo {something}")
        );
        assert_ne!(template_id("echo {something}"), template_id("echo other"));
        assert!(template_id("echo {something}").starts_with("tpl-"));
    }

    #[test]
    fn old_named_template_records_load_as_body_only_templates() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        append_jsonl(
            &path,
            &serde_json::json!({
                "name": "old-name",
                "body": "echo old-body"
            }),
        )
        .unwrap();

        let loaded = load_templates(&path).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items, [TemplateEntry::new("echo old-body")]);
    }

    #[test]
    fn remove_templates_by_id_removes_all_matches_and_keeps_others() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for body in ["one", "tail -f {file}", "one"] {
            append_template(&path, &TemplateEntry::new(body)).unwrap();
        }

        let id = template_id("one");
        let removal = remove_templates_by_id(&path, &id).unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(removal.removed, 2);
        assert_eq!(removal.errors, []);
        assert_eq!(removal.remaining.len(), 1);
        assert_eq!(loaded.items, removal.remaining);
        assert_eq!(loaded.items[0].body, "tail -f {file}");
    }

    #[test]
    fn find_template_by_id_returns_newest_match() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for body in ["old", "tail", "old"] {
            append_template(&path, &TemplateEntry::new(body)).unwrap();
        }

        let loaded = find_template_by_id(&path, &template_id("old")).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].body, "old");
    }

    #[test]
    fn replace_template_by_id_removes_old_matches_and_appends_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for body in ["old", "tail", "old"] {
            append_template(&path, &TemplateEntry::new(body)).unwrap();
        }

        let replacement = TemplateEntry::new("new");
        let removal =
            replace_template_by_id(&path, &template_id("old"), replacement.clone()).unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(removal.removed, 2);
        assert_eq!(removal.errors, []);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].body, "tail");
        assert_eq!(loaded.items[1], replacement);
    }

    #[test]
    fn template_placeholders_returns_unique_simple_names_in_order() {
        assert_eq!(
            template_placeholders("rsync {from} {user}@{host}:{to} {from} {bad space} {}"),
            ["from", "user", "host", "to"]
        );
    }

    #[test]
    fn template_placeholders_support_descriptions_and_variadic_markers() {
        assert_eq!(
            template_placeholders(
                "git commit -m {message:commit message} -- {paths...} {message} {bad name:ignored}"
            ),
            ["message", "paths"]
        );
    }

    #[test]
    fn template_placeholder_spans_return_valid_byte_ranges() {
        assert_eq!(
            template_placeholder_spans("echo {name} {paths...}"),
            [
                PlaceholderSpan { start: 5, end: 11 },
                PlaceholderSpan { start: 12, end: 22 },
            ]
        );
    }

    #[test]
    fn apply_template_values_replaces_known_placeholders_and_leaves_unknown() {
        let values = HashMap::from([
            ("from".to_string(), "src".to_string()),
            ("to".to_string(), "dst".to_string()),
        ]);

        assert_eq!(
            apply_template_values("cp {from} {to} {mode}", &values),
            "cp src dst {mode}"
        );
    }

    #[test]
    fn apply_template_values_with_usage_reports_used_keys() {
        let values = HashMap::from([
            ("from".to_string(), "src".to_string()),
            ("to".to_string(), "dst".to_string()),
            ("extra".to_string(), "ignored".to_string()),
        ]);

        let (rendered, used) = apply_template_values_with_usage("cp {from} {to} {mode}", &values);

        assert_eq!(rendered, "cp src dst {mode}");
        assert_eq!(used, ["from", "to"]);
    }

    #[test]
    fn apply_template_values_replaces_described_and_variadic_placeholders_by_name() {
        let values = HashMap::from([
            ("message".to_string(), "ship it".to_string()),
            ("paths".to_string(), "src tests".to_string()),
        ]);

        let (rendered, used) = apply_template_values_with_usage(
            "git commit -m {message:commit message} -- {paths...} {message}",
            &values,
        );

        assert_eq!(rendered, "git commit -m ship it -- src tests ship it");
        assert_eq!(used, ["message", "paths"]);
    }
}
