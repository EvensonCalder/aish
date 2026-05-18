use anyhow::{Context, Result, bail};

use super::filter::{clean_fish_repaint_lines, strip_terminal_control_sequences};
use super::{ready_marker, start_marker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HookCommandResult {
    pub(super) output: String,
    pub(super) exit_code: i32,
    pub(super) cwd: String,
    pub(super) started_command: Option<String>,
}

pub(super) fn marker_status_is_complete(raw: &str, marker: &str) -> bool {
    find_complete_marker(raw, marker).is_some()
}

pub(super) fn find_complete_marker(raw: &str, marker: &str) -> Option<usize> {
    raw.match_indices(marker)
        .find_map(|(marker_pos, _)| marker_has_complete_status(raw, marker, marker_pos))
}

fn marker_has_complete_status(raw: &str, marker: &str, marker_pos: usize) -> Option<usize> {
    let status_start = marker_pos + marker.len();
    let mut chars = raw[status_start..].chars();
    let first = chars.next()?;
    if !first.is_ascii_digit() {
        return None;
    }
    chars
        .any(|ch| ch == '\n' || ch == '\r')
        .then_some(marker_pos)
}

pub(super) fn start_marker_command(command: &str) -> String {
    let display_command = command.trim_end_matches('\n').replace(['\r', '\n'], "\\n");
    let start_marker = start_marker();
    format!(
        " printf '\n{start_marker}\t%s\n' {}\n",
        shell_single_quote(&display_command)
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn parse_marker_output(
    raw: &str,
    marker: &str,
) -> Result<(String, i32, Option<String>, Option<String>)> {
    let marker_pos = find_complete_marker(raw, marker)
        .context("backend shell output did not contain prompt marker")?;
    let before_marker = normalize_pty_newlines(strip_marker_separator(&raw[..marker_pos]))
        .trim_start_matches('\n')
        .to_string();
    let started_command = parse_started_command(&before_marker);
    let output = clean_marker_echo(&before_marker, marker);
    let status_start = marker_pos + marker.len();
    let status: String = raw[status_start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if status.is_empty() {
        bail!("backend shell prompt marker did not include exit status");
    }
    let exit_code = status.parse::<i32>().context("invalid shell exit status")?;
    let cwd_start = status_start + status.len();
    let cwd = raw[cwd_start..]
        .strip_prefix('\t')
        .map(|rest| {
            normalize_pty_newlines(rest)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|cwd| !cwd.is_empty());
    Ok((output, exit_code, cwd, started_command))
}

fn parse_started_command(output: &str) -> Option<String> {
    let prefix = format!("{}\t", start_marker());
    output
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .next_back()
        .map(str::to_string)
        .filter(|command| !command.is_empty())
}

fn strip_marker_separator(output: &str) -> &str {
    output
        .strip_suffix("\r\n")
        .or_else(|| output.strip_suffix('\n'))
        .or_else(|| output.strip_suffix('\r'))
        .unwrap_or(output)
}

pub(super) fn parse_ready_cwd(raw: &str) -> Option<String> {
    let prefix = format!("{}\t", ready_marker());
    let normalized = normalize_pty_newlines(raw);
    complete_normalized_lines(&normalized)
        .into_iter()
        .find_map(|line| {
            let cleaned = strip_terminal_control_sequences(line);
            cleaned
                .trim_start()
                .strip_prefix(&prefix)
                .and_then(parse_ready_marker_fields)
        })
        .map(|fields| fields.cwd)
}

pub(super) fn parse_ready_status_output(
    raw: &str,
    strip_terminal_repaint: bool,
) -> Result<HookCommandResult> {
    parse_ready_status_output_inner(raw, strip_terminal_repaint, false)
}

pub(super) fn parse_ready_status_output_with_prompt_separator(
    raw: &str,
    strip_terminal_repaint: bool,
) -> Result<HookCommandResult> {
    parse_ready_status_output_inner(raw, strip_terminal_repaint, true)
}

fn parse_ready_status_output_inner(
    raw: &str,
    strip_terminal_repaint: bool,
    ready_leading_newline_is_separator: bool,
) -> Result<HookCommandResult> {
    let raw = normalize_pty_newlines(raw);
    let mut ready = None;
    let mut current_started_command = None;
    let mut saw_start_marker = false;
    let mut pre_start_output_lines = Vec::new();
    let mut command_output_lines = Vec::new();

    for line in complete_normalized_lines(&raw) {
        let cleaned_marker_line = strip_terminal_control_sequences(line);
        let marker_line = cleaned_marker_line.trim_start();
        if let Some(command) = marker_line.strip_prefix(&format!("{}\t", start_marker())) {
            current_started_command = Some(command.to_string());
            saw_start_marker = true;
            command_output_lines.clear();
            continue;
        }
        if let Some(rest) = marker_line.strip_prefix(&format!("{}\t", ready_marker())) {
            if let Some(fields) = parse_ready_marker_fields(rest)
                && let Some(status) = fields.status
            {
                let output_lines = if saw_start_marker {
                    command_output_lines.clone()
                } else {
                    pre_start_output_lines.clone()
                };
                ready = Some(ReadyStatusSnapshot {
                    status,
                    cwd: fields.cwd,
                    output_lines,
                    started_command: current_started_command.clone(),
                });
            }
            continue;
        }
        if saw_start_marker {
            command_output_lines.push(line.to_string());
        } else {
            pre_start_output_lines.push(line.to_string());
        }
    }

    let ready = ready.context("backend shell output did not contain ready marker")?;
    let mut output_lines = ready.output_lines;
    let exit_code = ready
        .status
        .trim()
        .parse::<i32>()
        .context("invalid shell exit status in ready marker")?;
    let started_command = ready.started_command;
    if strip_terminal_repaint {
        output_lines = clean_fish_repaint_lines(output_lines, started_command.as_deref());
    }
    let output_ended_with_newline = if ready_leading_newline_is_separator {
        output_lines.last().is_some_and(|line| line.is_empty())
    } else {
        !output_lines.is_empty()
    };
    while output_lines.first().is_some_and(|line| line.is_empty()) {
        output_lines.remove(0);
    }
    while output_lines.last().is_some_and(|line| line.is_empty()) {
        output_lines.pop();
    }
    let output = if output_lines.is_empty() {
        String::new()
    } else if output_ended_with_newline {
        format!("{}\n", output_lines.join("\n"))
    } else {
        output_lines.join("\n")
    };

    Ok(HookCommandResult {
        output,
        exit_code,
        cwd: ready.cwd,
        started_command,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyMarkerFields {
    status: Option<String>,
    cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadyStatusSnapshot {
    status: String,
    cwd: String,
    output_lines: Vec<String>,
    started_command: Option<String>,
}

fn parse_ready_marker_fields(rest: &str) -> Option<ReadyMarkerFields> {
    let mut parts = rest.splitn(2, '\t');
    let first = strip_terminal_control_sequences(parts.next()?);
    if let Some(cwd) = parts.next() {
        let status = first.trim().to_string();
        if status.parse::<i32>().is_err() {
            return None;
        }
        let cwd = strip_terminal_control_sequences(cwd).trim_end().to_string();
        return (!cwd.is_empty()).then_some(ReadyMarkerFields {
            status: Some(status),
            cwd,
        });
    }

    let cwd = first.trim_end().to_string();
    (!cwd.is_empty() && cwd.parse::<i32>().is_err())
        .then_some(ReadyMarkerFields { status: None, cwd })
}

pub(super) fn normalize_pty_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn complete_normalized_lines(normalized: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = normalized.lines().collect();
    if !normalized.ends_with('\n') {
        lines.pop();
    }
    lines
}

pub(super) fn clean_marker_echo(output: &str, marker: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in output.split_inclusive('\n') {
        if is_internal_marker_echo_line(line, marker) {
            if lines
                .last()
                .is_some_and(|previous| terminal_separator_only(previous))
            {
                lines.pop();
            }
            continue;
        }
        lines.push(line);
    }
    lines.concat()
}

fn is_internal_marker_echo_line(line: &str, marker: &str) -> bool {
    let text = line.trim_end_matches('\n');
    text.contains(ready_marker())
        || text.contains(start_marker())
        || text.contains("__aish_status=$?") && text.contains(marker)
}

fn terminal_separator_only(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|ch| matches!(ch, '\r' | '\n'))
}
