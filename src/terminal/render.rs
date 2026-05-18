use std::io::Write;

use anyhow::Result;
use crossterm::cursor::{
    MoveDown, MoveTo, MoveToColumn, MoveToPreviousLine, RestorePosition, SavePosition,
};
use crossterm::execute;
use crossterm::terminal::{Clear, ClearType, size};

use crate::app::AppState;
use crate::display_width::{visual_line_count, visual_position};

use super::completion_ui::{render_inline_completion_suffix, write_inline_completion_suffix};

pub fn redraw(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    let (width, height) = terminal_display_size();
    redraw_for_size(state, out, width, height)
}

#[cfg(test)]
pub(super) fn redraw_for_width(
    state: &mut AppState,
    out: &mut impl Write,
    width: usize,
) -> Result<()> {
    redraw_for_size(state, out, width, terminal_display_height())
}

pub(super) fn redraw_for_size(
    state: &mut AppState,
    out: &mut impl Write,
    width: usize,
    height: usize,
) -> Result<()> {
    let rendered = state.rendered_text();
    let inline_suffix = render_inline_completion_suffix(state, width);
    let rendered_with_inline =
        full_rendered_text_for_width(&rendered, inline_suffix.as_deref(), &[]);
    let prompt_lines = visual_line_count(&rendered_with_inline, width);
    let visible_panel_len = state
        .completion_panel
        .len()
        .min(height.saturating_sub(prompt_lines));
    let visible_panel = &state.completion_panel[..visible_panel_len];
    let full_render =
        full_rendered_text_for_width(&rendered, inline_suffix.as_deref(), visible_panel);
    let render_lines = visual_line_count(&full_render, width).max(1);

    move_to_rendered_start(state, out)?;
    reserve_render_area(out, render_lines, height)?;
    execute!(
        out,
        MoveToColumn(0),
        Clear(ClearType::FromCursorDown),
        SavePosition
    )?;
    write!(out, "{}", rendered.replace('\n', "\r\n"))?;
    if let Some(suffix) = &inline_suffix {
        write_inline_completion_suffix(out, suffix)?;
    }
    if !visible_panel.is_empty() {
        for line in visible_panel {
            write!(out, "\r\n{line}")?;
        }
    }
    let final_row = visual_line_count(&full_render, width).saturating_sub(1);
    let (cursor_row, cursor_col) = terminal_cursor_position_for_width(state, width);
    move_to_rendered_position(out, cursor_row, cursor_col)?;
    state.last_rendered_lines = final_row + 1;
    state.last_rendered_cursor_row = cursor_row;
    state.render_anchor_saved = true;
    out.flush()?;
    Ok(())
}

pub(super) fn clear_screen_for_redraw(state: &mut AppState, out: &mut impl Write) -> Result<()> {
    invalidate_render_anchor(state);
    execute!(
        out,
        MoveTo(0, 0),
        Clear(ClearType::All),
        Clear(ClearType::Purge),
        MoveTo(0, 0)
    )?;
    Ok(())
}

pub(super) fn terminal_display_width() -> usize {
    terminal_display_size().0
}

#[cfg(test)]
pub(super) fn terminal_display_height() -> usize {
    terminal_display_size().1
}

fn terminal_display_size() -> (usize, usize) {
    match size() {
        Ok((columns, rows)) => (
            if columns > 0 { columns as usize } else { 80 },
            if rows > 0 { rows as usize } else { 24 },
        ),
        _ => (80, 24),
    }
}

pub(super) fn completion_panel_content_start_col(state: &AppState, width: usize) -> usize {
    let prefix = if state.draft.as_str()[..state.draft.cursor()].contains('\n') {
        state
            .continuation_prompt
            .as_deref()
            .unwrap_or(".. ")
            .to_string()
    } else {
        state.prompt_prefix()
    };
    visual_position(&prefix, width).1 as usize
}

fn move_to_rendered_start(state: &AppState, out: &mut impl Write) -> Result<()> {
    if state.last_rendered_cursor_row > 0 {
        execute!(
            out,
            MoveToPreviousLine(state.last_rendered_cursor_row as u16)
        )?;
    }
    execute!(out, MoveToColumn(0))?;
    Ok(())
}

fn reserve_render_area(out: &mut impl Write, render_lines: usize, height: usize) -> Result<()> {
    let reserve_rows = render_lines
        .saturating_sub(1)
        .min(height.saturating_sub(1))
        .min(u16::MAX as usize);
    if reserve_rows == 0 {
        return Ok(());
    }
    for _ in 0..reserve_rows {
        write!(out, "\r\n")?;
    }
    execute!(
        out,
        MoveToPreviousLine(reserve_rows as u16),
        MoveToColumn(0)
    )?;
    Ok(())
}

fn move_to_rendered_position(out: &mut impl Write, row: usize, col: u16) -> Result<()> {
    execute!(out, RestorePosition)?;
    if row > 0 {
        execute!(out, MoveDown(row.min(u16::MAX as usize) as u16))?;
    }
    execute!(out, MoveToColumn(col))?;
    Ok(())
}

pub(super) fn move_to_rendered_end(
    state: &AppState,
    out: &mut impl Write,
    width: usize,
) -> Result<()> {
    move_to_rendered_start(state, out)?;
    let rendered = state.rendered_text();
    let (end_row, end_col) = visual_position(&rendered, width);
    if end_row > 0 {
        execute!(out, MoveDown(end_row as u16))?;
    }
    execute!(out, MoveToColumn(end_col))?;
    Ok(())
}

pub(super) fn invalidate_render_anchor(state: &mut AppState) {
    state.last_rendered_lines = 0;
    state.last_rendered_cursor_row = 0;
    state.render_anchor_saved = false;
}

fn full_rendered_text_for_width(
    rendered: &str,
    inline_suffix: Option<&str>,
    panel: &[String],
) -> String {
    let mut full = String::from(rendered);
    if let Some(suffix) = inline_suffix {
        full.push_str(suffix);
    }
    for line in panel {
        full.push('\n');
        full.push_str(line);
    }
    full
}

pub(super) fn terminal_cursor_position_for_width(state: &AppState, width: usize) -> (usize, u16) {
    let rendered_before_cursor = rendered_text_before_cursor(state);
    let (row, col) = visual_position(&rendered_before_cursor, width);
    (row, col)
}

fn rendered_text_before_cursor(state: &AppState) -> String {
    if let Some(pending) = &state.pending_context {
        let marker = if pending.dangerous {
            "[dangerous context confirmation: Y/n]"
        } else {
            "[context confirmation: Y/n]"
        };
        return format!("{}{}", state.prompt_prefix(), marker);
    }
    if state.pending_private_output.is_some() {
        return format!(
            "{}{}",
            state.prompt_prefix(),
            "[private output export confirmation: Y/n]"
        );
    }
    match state.mode {
        crate::modes::Mode::History => format!(
            "{}{}",
            state.prompt_prefix(),
            state.selected_history_command().unwrap_or("")
        ),
        crate::modes::Mode::Ai => format!(
            "{}{}",
            state.prompt_prefix(),
            state.selected_ai_command().unwrap_or("")
        ),
        crate::modes::Mode::Draft if state.draft_from_editor => {
            format!(
                "{}{}",
                state.prompt_prefix(),
                state.editor_draft_summary_for_terminal()
            )
        }
        _ => {
            let before_cursor = &state.draft.as_str()[..state.draft.cursor()];
            if before_cursor.contains('\n') {
                render_multiline_for_terminal(
                    &state.prompt_prefix(),
                    state.continuation_prompt.as_deref().unwrap_or(".. "),
                    before_cursor,
                )
            } else {
                format!("{}{}", state.prompt_prefix(), before_cursor)
            }
        }
    }
}

fn render_multiline_for_terminal(
    prompt_prefix: &str,
    continuation_prefix: &str,
    text: &str,
) -> String {
    let mut lines = text.split('\n');
    let mut rendered = String::from(prompt_prefix);
    rendered.push_str(lines.next().unwrap_or_default());
    for line in lines {
        rendered.push('\n');
        rendered.push_str(continuation_prefix);
        rendered.push_str(line);
    }
    rendered
}
