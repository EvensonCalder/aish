use std::path::PathBuf;

use crate::config::PromptConfig;
use crate::display_width::display_width;
use crate::modes::Mode;
use crate::paste;

use super::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplates {
    pub draft: String,
    pub history: String,
    pub ai: String,
}

impl Default for PromptTemplates {
    fn default() -> Self {
        Self {
            draft: "{mode} ".to_string(),
            history: "{mode} ".to_string(),
            ai: "{mode} ".to_string(),
        }
    }
}

impl From<PromptConfig> for PromptTemplates {
    fn from(config: PromptConfig) -> Self {
        Self {
            draft: config.draft,
            history: config.history,
            ai: config.ai,
        }
    }
}

impl AppState {
    pub fn prompt_prefix(&self) -> String {
        let template = match self.mode {
            Mode::History => &self.prompt_templates.history,
            Mode::Ai => &self.prompt_templates.ai,
            _ => &self.prompt_templates.draft,
        };
        self.render_prompt_template(template)
    }

    fn render_prompt_template(&self, template: &str) -> String {
        let mode = self.mode.symbol().to_string();
        let cwd = self
            .current_cwd
            .as_ref()
            .map(|cwd| display_cwd(cwd))
            .unwrap_or_default();
        let basename = self
            .current_cwd
            .as_ref()
            .and_then(|cwd| cwd.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        template
            .replace("{user}", &prompt_user())
            .replace("{host}", &prompt_host())
            .replace("{cwd}", &cwd)
            .replace("{basename}", basename)
            .replace("{mode}", &mode)
            .replace(
                "{last_status}",
                &self
                    .last_status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "none".to_string()),
            )
    }

    pub fn render_prompt_line(&self) -> String {
        self.rendered_text()
    }

    pub fn rendered_text(&self) -> String {
        if let Some(pending) = &self.pending_context {
            let marker = if pending.dangerous {
                "[dangerous context confirmation: Y/n]"
            } else {
                "[context confirmation: Y/n]"
            };
            return format!("{}{}", self.prompt_prefix(), marker);
        }
        if self.pending_private_output.is_some() {
            return format!(
                "{}{}",
                self.prompt_prefix(),
                "[private output export confirmation: Y/n]"
            );
        }
        let text = match self.mode {
            Mode::History => self
                .selected_history_command()
                .or_else(|| {
                    self.encrypted_storage_is_locked()
                        .then_some("history is still unlocking...")
                })
                .unwrap_or(""),
            Mode::Ai => self
                .selected_ai_command()
                .or_else(|| {
                    self.encrypted_storage_is_locked()
                        .then_some("history is still unlocking...")
                })
                .unwrap_or(""),
            Mode::Draft if self.draft_from_editor => {
                let mut rendered = format!(
                    "{}{}",
                    self.prompt_prefix(),
                    self.editor_draft_summary_for_terminal()
                );
                if let Some(preview) = self.paste_preview_for_terminal() {
                    rendered.push('\n');
                    rendered.push_str(&preview);
                }
                return rendered;
            }
            _ => self.draft.as_str(),
        };
        if self.mode == Mode::Draft && text.contains('\n') {
            return render_multiline_draft(
                &self.prompt_prefix(),
                self.continuation_prompt
                    .as_deref()
                    .unwrap_or(AppState::CONTINUATION_PREFIX),
                text,
            );
        }
        format!("{}{}", self.prompt_prefix(), text)
    }

    pub fn terminal_cursor_column(&self) -> u16 {
        self.terminal_cursor_position().1
    }

    pub fn terminal_cursor_position(&self) -> (u16, u16) {
        if let Some(pending) = &self.pending_context {
            let marker = if pending.dangerous {
                "[dangerous context confirmation: Y/n]"
            } else {
                "[context confirmation: Y/n]"
            };
            return (
                0,
                display_width(&format!("{}{}", self.prompt_prefix(), marker)).min(u16::MAX as usize)
                    as u16,
            );
        }
        if self.pending_private_output.is_some() {
            return (
                0,
                display_width(&format!(
                    "{}{}",
                    self.prompt_prefix(),
                    "[private output export confirmation: Y/n]"
                ))
                .min(u16::MAX as usize) as u16,
            );
        }
        let rendered_before_cursor = match self.mode {
            Mode::History => format!(
                "{}{}",
                self.prompt_prefix(),
                self.selected_history_command()
                    .or_else(|| self
                        .encrypted_storage_is_locked()
                        .then_some("history is still unlocking..."))
                    .unwrap_or("")
            ),
            Mode::Ai => format!(
                "{}{}",
                self.prompt_prefix(),
                self.selected_ai_command()
                    .or_else(|| self
                        .encrypted_storage_is_locked()
                        .then_some("history is still unlocking..."))
                    .unwrap_or("")
            ),
            Mode::Draft if self.draft_from_editor => {
                format!(
                    "{}{}",
                    self.prompt_prefix(),
                    self.editor_draft_summary_for_terminal()
                )
            }
            _ => {
                let before_cursor = &self.draft.as_str()[..self.draft.cursor()];
                if before_cursor.contains('\n') {
                    render_multiline_draft(
                        &self.prompt_prefix(),
                        self.continuation_prompt
                            .as_deref()
                            .unwrap_or(AppState::CONTINUATION_PREFIX),
                        before_cursor,
                    )
                } else {
                    format!("{}{}", self.prompt_prefix(), before_cursor)
                }
            }
        };
        let mut lines = rendered_before_cursor.split('\n');
        let last = lines.next_back().unwrap_or_default();
        let row = rendered_before_cursor.split('\n').count().saturating_sub(1);
        (
            row.min(u16::MAX as usize) as u16,
            display_width(last).min(u16::MAX as usize) as u16,
        )
    }

    pub fn rendered_line_count(&self) -> usize {
        self.rendered_text().split('\n').count().max(1)
    }

    pub fn rendered_last_line_column(&self) -> u16 {
        let rendered = self.rendered_text();
        display_width(rendered.rsplit('\n').next().unwrap_or_default()).min(u16::MAX as usize)
            as u16
    }

    pub(crate) fn editor_draft_summary_for_terminal(&self) -> String {
        let bytes = self.draft.as_str().len();
        let lines = self
            .draft
            .as_str()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            .max(1);
        let line_label = if lines == 1 { "line" } else { "lines" };
        if self.draft_from_ai_editor {
            format!(
                "[ai prompt: {lines} {line_label}, {bytes} bytes; Enter send, Ctrl-X Ctrl-E edit]"
            )
        } else {
            format!("[draft: {lines} {line_label}, {bytes} bytes; Enter run, Ctrl-X Ctrl-E edit]")
        }
    }

    fn paste_preview_for_terminal(&self) -> Option<String> {
        if !self.draft_has_paste_preview
            || !self.paste_config.preview
            || self.draft_from_ai_editor
            || self.draft.is_empty()
        {
            return None;
        }

        let lines = paste::preview_lines(
            self.draft.as_str(),
            self.paste_config.preview_lines,
            self.paste_config.preview_bytes,
        );
        if lines.is_empty() {
            return None;
        }

        let mut rendered = String::from("paste preview:");
        for line in lines {
            rendered.push('\n');
            rendered.push_str("  ");
            rendered.push_str(&line);
        }
        Some(rendered)
    }
}

fn render_multiline_draft(prompt_prefix: &str, continuation_prefix: &str, text: &str) -> String {
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

fn prompt_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

fn prompt_host() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_default()
}

fn display_cwd(cwd: &std::path::Path) -> String {
    let Some(home) = prompt_home_dir() else {
        return cwd.display().to_string();
    };
    if cwd == home {
        return "~".to_string();
    }
    if let Ok(rest) = cwd.strip_prefix(&home) {
        if rest.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", rest.display())
        }
    } else {
        cwd.display().to_string()
    }
}

fn prompt_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}
