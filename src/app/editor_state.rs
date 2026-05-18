use anyhow::Result;

use crate::editor::{
    EditorCommand, EditorRunResult, PreparedEditorSession, prepare_editor_file, read_editor_file,
    run_editor_command,
};
use crate::input::InputBuffer;
use crate::modes::Mode;

use super::{
    AppState, ai_editor_initial_text, draft_is_ai_prompt_or_empty_editor_trigger,
    normalize_editor_draft_content,
};

impl AppState {
    pub fn prepare_editor_session(
        &mut self,
        temp_root: &std::path::Path,
    ) -> Result<PreparedEditorSession> {
        self.copy_read_only_selection_to_draft();
        self.mode = Mode::Draft;
        prepare_editor_file(temp_root, self.draft.as_str())
    }

    pub fn should_open_ai_prompt_editor(&self) -> bool {
        self.draft_from_ai_editor || draft_is_ai_prompt_or_empty_editor_trigger(self.draft.as_str())
    }

    pub fn prepare_ai_prompt_editor_session(
        &mut self,
        temp_root: &std::path::Path,
    ) -> Result<PreparedEditorSession> {
        self.mode = Mode::Draft;
        let initial_text = if self.draft_from_ai_editor {
            self.draft.as_str().to_string()
        } else {
            ai_editor_initial_text(self.draft.as_str()).unwrap_or_default()
        };
        prepare_editor_file(temp_root, &initial_text)
    }

    pub fn replace_draft_from_editor_session(
        &mut self,
        session: &PreparedEditorSession,
    ) -> Result<()> {
        let content = normalize_editor_draft_content(&read_editor_file(session)?);
        let keep_paste_preview = self.draft_has_paste_preview;
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = keep_paste_preview && self.draft_from_editor;
        self.mode = Mode::Draft;
        Ok(())
    }

    pub fn replace_draft_from_ai_prompt_editor_session(
        &mut self,
        session: &PreparedEditorSession,
    ) -> Result<()> {
        let content = normalize_editor_draft_content(&read_editor_file(session)?);
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = !self.draft.is_empty();
        self.draft_from_template = false;
        self.draft_has_paste_preview = false;
        self.mode = Mode::Draft;
        Ok(())
    }

    pub fn run_editor_roundtrip(
        &mut self,
        temp_root: &std::path::Path,
        command: &EditorCommand,
    ) -> Result<EditorRunResult> {
        let session = self.prepare_editor_session(temp_root)?;
        let result = run_editor_command(command, &session)?;
        if result.exit_code == Some(0) {
            self.replace_draft_from_editor_session(&session)?;
        }
        Ok(result)
    }

    pub fn run_ai_prompt_editor_roundtrip(
        &mut self,
        temp_root: &std::path::Path,
        command: &EditorCommand,
    ) -> Result<EditorRunResult> {
        let session = self.prepare_ai_prompt_editor_session(temp_root)?;
        let result = run_editor_command(command, &session)?;
        if result.exit_code == Some(0) {
            self.replace_draft_from_ai_prompt_editor_session(&session)?;
        }
        Ok(result)
    }

    pub fn replace_draft_from_editor_text(&mut self, content: impl Into<String>) {
        let content = normalize_editor_draft_content(&content.into());
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = false;
        self.mode = Mode::Draft;
    }

    pub fn replace_draft_from_paste_text(&mut self, content: impl Into<String>) {
        let content = normalize_editor_draft_content(&content.into());
        self.draft = InputBuffer::from(content);
        self.selected_draft_index = None;
        self.draft_from_editor = !self.draft.is_empty();
        self.draft_from_ai_editor = false;
        self.draft_from_template = false;
        self.draft_has_paste_preview = self.draft_from_editor;
        self.mode = Mode::Draft;
    }
}
