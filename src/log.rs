use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::history::{JsonlLoad, append_jsonl, load_jsonl, rewrite_jsonl};

pub const DEFAULT_MAX_EVENTS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry {
    pub t: i64,
    pub level: EventLevel,
    pub msg: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

pub fn append_event(
    path: &Path,
    t: i64,
    level: EventLevel,
    msg: &str,
    max_events: usize,
) -> Result<()> {
    append_jsonl(
        path,
        &EventLogEntry {
            t,
            level,
            msg: redact_secrets(msg),
        },
    )?;
    trim_events(path, max_events)?;
    Ok(())
}

pub fn load_events(path: &Path) -> Result<JsonlLoad<EventLogEntry>> {
    load_jsonl(path)
}

pub fn trim_events(path: &Path, max_events: usize) -> Result<JsonlLoad<EventLogEntry>> {
    let loaded = load_events(path)?;
    let keep_from = loaded.items.len().saturating_sub(max_events);
    rewrite_jsonl(path, &loaded.items[keep_from..])?;
    Ok(loaded)
}

pub fn format_recent_events(events: &[EventLogEntry], count: usize) -> Vec<String> {
    let keep_from = events.len().saturating_sub(count);
    events[keep_from..]
        .iter()
        .map(|event| format!("{}\t{:?}\t{}", event.t, event.level, event.msg))
        .collect()
}

pub fn redact_secrets(msg: &str) -> String {
    msg.split_whitespace()
        .map(|part| {
            if looks_secret_like(part) {
                "[redacted]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_secret_like(value: &str) -> bool {
    let trimmed =
        value.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    trimmed.starts_with("sk-")
        || trimmed.starts_with("sk_")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("github_pat_")
        || trimmed.starts_with("Bearer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_event_writes_jsonl_and_trims_old_events() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("logs/events.jsonl");

        append_event(&path, 1, EventLevel::Info, "one", 2).unwrap();
        append_event(&path, 2, EventLevel::Warn, "two", 2).unwrap();
        append_event(&path, 3, EventLevel::Error, "three", 2).unwrap();

        let loaded = load_events(&path).unwrap();
        assert_eq!(loaded.items.len(), 2);
        assert_eq!(loaded.items[0].msg, "two");
        assert_eq!(loaded.items[1].msg, "three");
    }

    #[test]
    fn format_recent_events_keeps_latest_count() {
        let events = vec![
            EventLogEntry {
                t: 1,
                level: EventLevel::Info,
                msg: "one".to_string(),
            },
            EventLogEntry {
                t: 2,
                level: EventLevel::Warn,
                msg: "two".to_string(),
            },
        ];

        assert_eq!(format_recent_events(&events, 1), vec!["2\tWarn\ttwo"]);
    }

    #[test]
    fn redact_secrets_masks_common_token_shapes() {
        assert_eq!(
            redact_secrets("failed with sk-test ghp_abc github_pat_xyz safe"),
            "failed with [redacted] [redacted] [redacted] safe"
        );
    }
}
