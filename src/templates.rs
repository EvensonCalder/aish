use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::history::{JsonlLineError, JsonlLoad, append_jsonl, load_jsonl, rewrite_jsonl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateEntry {
    pub name: String,
    pub body: String,
}

pub fn append_template(path: &Path, entry: &TemplateEntry) -> Result<()> {
    append_jsonl(path, entry)
}

pub fn load_templates(path: &Path) -> Result<JsonlLoad<TemplateEntry>> {
    load_jsonl(path)
}

pub fn find_template_by_name(path: &Path, name: &str) -> Result<JsonlLoad<TemplateEntry>> {
    let mut loaded = load_templates(path)?;
    loaded.items = loaded
        .items
        .into_iter()
        .rev()
        .find(|template| template.name == name)
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

pub fn remove_templates_by_name(path: &Path, name: &str) -> Result<TemplateRemoval> {
    let loaded = load_templates(path)?;
    let before = loaded.items.len();
    let remaining: Vec<_> = loaded
        .items
        .into_iter()
        .filter(|template| template.name != name)
        .collect();
    let removed = before - remaining.len();
    rewrite_jsonl(path, &remaining)?;

    Ok(TemplateRemoval {
        removed,
        remaining,
        errors: loaded.errors,
    })
}

pub fn replace_template(path: &Path, entry: TemplateEntry) -> Result<TemplateRemoval> {
    let loaded = load_templates(path)?;
    let before = loaded.items.len();
    let mut remaining: Vec<_> = loaded
        .items
        .into_iter()
        .filter(|template| template.name != entry.name)
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
        let entry = TemplateEntry {
            name: "deploy".to_string(),
            body: "rsync -avz {from} {to}".to_string(),
        };

        append_template(&path, &entry).unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items, [entry]);
    }

    #[test]
    fn remove_templates_by_name_removes_all_matches_and_keeps_others() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [
            ("deploy", "one"),
            ("logs", "tail -f {file}"),
            ("deploy", "two"),
        ] {
            append_template(
                &path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }

        let removal = remove_templates_by_name(&path, "deploy").unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(removal.removed, 2);
        assert_eq!(removal.errors, []);
        assert_eq!(removal.remaining.len(), 1);
        assert_eq!(loaded.items, removal.remaining);
        assert_eq!(loaded.items[0].name, "logs");
    }

    #[test]
    fn find_template_by_name_returns_newest_match() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [("deploy", "old"), ("logs", "tail"), ("deploy", "new")] {
            append_template(
                &path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }

        let loaded = find_template_by_name(&path, "deploy").unwrap();

        assert_eq!(loaded.errors, []);
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].body, "new");
    }

    #[test]
    fn replace_template_removes_old_matches_and_appends_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("templates/templates.jsonl");
        for (name, body) in [("deploy", "old"), ("logs", "tail"), ("deploy", "older")] {
            append_template(
                &path,
                &TemplateEntry {
                    name: name.to_string(),
                    body: body.to_string(),
                },
            )
            .unwrap();
        }

        let replacement = TemplateEntry {
            name: "deploy".to_string(),
            body: "new".to_string(),
        };
        let removal = replace_template(&path, replacement.clone()).unwrap();
        let loaded = load_templates(&path).unwrap();

        assert_eq!(removal.removed, 2);
        assert_eq!(removal.errors, []);
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].name, "logs");
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
