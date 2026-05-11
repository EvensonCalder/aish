use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::history::{JsonlLoad, append_jsonl, load_jsonl};

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
}
