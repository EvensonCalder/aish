use std::io::Write;

use anyhow::Result;

use crate::log::{format_recent_events, load_events};

use super::AppState;

pub(super) fn show_event_log(state: &AppState, out: &mut impl Write, args: &str) -> Result<()> {
    let count = args.parse::<usize>();
    match (count, &state.events_path) {
        (Ok(count), Some(path)) => {
            let loaded = load_events(path)?;
            for line in format_recent_events(&loaded.items, count) {
                writeln!(out, "{line}")?;
            }
            if loaded.items.is_empty() {
                writeln!(out, "no events logged")?;
            }
            if !loaded.errors.is_empty() {
                writeln!(out, "skipped {} bad event line(s)", loaded.errors.len())?;
            }
        }
        (Ok(_), None) => writeln!(out, "event log storage is not configured")?,
        (Err(_), _) => writeln!(out, "usage: #log <count>")?,
    }
    Ok(())
}
