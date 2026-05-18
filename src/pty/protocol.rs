use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

const MARKER_PREFIX: &str = "__AISH_STATUS__";
pub(super) const READY_MARKER: &str = "__AISH_READY__";
pub(super) const START_MARKER: &str = "__AISH_START__";

static NEXT_MARKER_ID: AtomicU64 = AtomicU64::new(1);
static SESSION_MARKERS: OnceLock<SessionMarkers> = OnceLock::new();

#[derive(Debug)]
struct SessionMarkers {
    status_prefix: String,
    ready: String,
    start: String,
}

pub(super) fn status_marker_command(marker: &str) -> String {
    format!(
        " __aish_status=$?; command -v __aish_run_prompt_command >/dev/null 2>&1 && __aish_run_prompt_command >/dev/null 2>&1; printf '\\n%s%s\\t%s\\n' '{marker}' \"$__aish_status\" \"$PWD\"; __aish_preserve_status() {{ return \"$1\"; }}; __aish_preserve_status \"$__aish_status\"\n"
    )
}

pub(super) fn next_marker() -> String {
    let id = NEXT_MARKER_ID.fetch_add(1, Ordering::Relaxed);
    format!("{}{id}__", session_markers().status_prefix)
}

pub(super) fn ready_marker() -> &'static str {
    session_markers().ready.as_str()
}

pub(super) fn start_marker() -> &'static str {
    session_markers().start.as_str()
}

fn session_markers() -> &'static SessionMarkers {
    SESSION_MARKERS.get_or_init(|| {
        let token = marker_token();
        SessionMarkers {
            status_prefix: format!("{MARKER_PREFIX}{token}_"),
            ready: format!("{READY_MARKER}{token}__"),
            start: format!("{START_MARKER}{token}__"),
        }
    })
}

fn marker_token() -> String {
    let mut random = [0_u8; 16];
    if getrandom::fill(&mut random).is_ok() {
        return hex_bytes(&random);
    }

    let id = NEXT_MARKER_ID.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{pid:x}{nanos:x}{id:x}")
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
