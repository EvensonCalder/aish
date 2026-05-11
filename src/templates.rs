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
    let mut rest = body;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            break;
        };
        let candidate = &rest[..end];
        if is_placeholder_name(candidate) && !placeholders.iter().any(|item| item == candidate) {
            placeholders.push(candidate.to_string());
        }
        rest = &rest[end + 1..];
    }
    placeholders
}

fn is_placeholder_name(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
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
}
