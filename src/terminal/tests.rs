use super::*;
use crate::config::{CompletionConfig, CompletionMode, CompletionTabAccept, EditorConfig};
use crate::display_width::display_width;
use crate::encrypted_writer::EncryptedWriteQueue;
use crate::history::{DraftEntry, HistoryEntry, HistorySource};
use crate::keybindings::{KeySequenceConfig, KeybindingConfig};
use crate::modes::Mode;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

mod completion;
mod external;
mod input;
mod lifecycle;
mod paste;
mod render;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn alt(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::ALT)
}

fn alt_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::ALT)
}

fn wait_for_inline_suffix(state: &mut AppState, suffix: &str) {
    let mut output = Vec::new();
    for _ in 0..50 {
        refresh_after_background_events(state, &mut output).unwrap();
        if state
            .completion_inline
            .as_ref()
            .is_some_and(|inline| inline.suffix == suffix)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing inline suffix {suffix:?}; inline was {:?}, panel was {:?}",
        state.completion_inline, state.completion_panel
    );
}

fn wait_for_visible_completion_panel_contains(state: &mut AppState, needle: &str) {
    let mut output = Vec::new();
    for _ in 0..50 {
        refresh_after_background_events(state, &mut output).unwrap();
        if state
            .completion_panel
            .iter()
            .any(|row| row.contains(needle))
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing visible completion panel containing {needle:?}; inline was {:?}, panel was {:?}",
        state.completion_inline, state.completion_panel
    );
}

#[cfg(unix)]
fn write_copying_fake_gpg(temp: &tempfile::TempDir) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_gpg = temp.path().join("copy-gpg");
    std::fs::write(
            &fake_gpg,
            "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  cp \"$input\" \"$out\"\nfi\n",
        )
        .unwrap();
    std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
    fake_gpg
}

#[cfg(unix)]
fn write_blocking_fake_gpg(
    temp: &tempfile::TempDir,
    started_path: &Path,
    release_path: &Path,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let fake_gpg = temp.path().join("blocking-gpg");
    std::fs::write(
            &fake_gpg,
            format!(
                "#!/bin/sh\nmode=encrypt\nout=\"\"\ninput=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --decrypt) mode=decrypt ;;\n    --output) shift; out=\"$1\" ;;\n    --recipient|--trust-model) shift ;;\n    --batch|--yes|--no-tty|--encrypt|always) ;;\n    *) input=\"$1\" ;;\n  esac\n  shift\ndone\nif [ \"$mode\" = decrypt ]; then\n  cat \"$input\"\nelse\n  : > '{}'\n  while [ ! -f '{}' ]; do sleep 0.02; done\n  cp \"$input\" \"$out\"\nfi\n",
                started_path.display(),
                release_path.display()
            ),
        )
        .unwrap();
    std::fs::set_permissions(&fake_gpg, std::fs::Permissions::from_mode(0o755)).unwrap();
    fake_gpg
}

fn fixed_clock() -> i64 {
    42
}

struct TestScreen {
    rows: Vec<Vec<char>>,
    scrollback: Vec<Vec<char>>,
    row: usize,
    col: usize,
    saved_position: Option<(usize, usize)>,
    height: Option<usize>,
}

impl TestScreen {
    fn from_output(output: &str) -> Self {
        Self::from_output_with_optional_height(output, None)
    }

    fn from_output_with_height(output: &str, height: usize) -> Self {
        Self::from_output_with_optional_height(output, Some(height.max(1)))
    }

    fn from_output_with_optional_height(output: &str, height: Option<usize>) -> Self {
        let mut screen = Self {
            rows: vec![Vec::new(); height.unwrap_or(8)],
            scrollback: Vec::new(),
            row: 0,
            col: 0,
            saved_position: None,
            height,
        };
        let chars: Vec<char> = output.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\x1b' if chars.get(i + 1) == Some(&'[') => {
                    i = screen.apply_csi(&chars, i + 2);
                }
                '\x1b' if chars.get(i + 1) == Some(&'7') => {
                    screen.saved_position = Some((screen.row, screen.col));
                    i += 2;
                }
                '\x1b' if chars.get(i + 1) == Some(&'8') => {
                    if let Some((row, col)) = screen.saved_position {
                        screen.row = row;
                        screen.col = col;
                        screen.ensure_row();
                    }
                    i += 2;
                }
                '\r' => {
                    screen.col = 0;
                    i += 1;
                }
                '\n' => {
                    screen.newline();
                    i += 1;
                }
                ch => {
                    screen.put(ch);
                    i += 1;
                }
            }
        }
        screen
    }

    fn apply_csi(&mut self, chars: &[char], mut i: usize) -> usize {
        let start = i;
        while i < chars.len() && !chars[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i >= chars.len() {
            return i;
        }
        let params: String = chars[start..i].iter().collect();
        match chars[i] {
            'A' => {
                let amount = csi_amount(&params);
                self.row = self.row.saturating_sub(amount);
            }
            'B' => {
                self.move_down(csi_amount(&params));
                self.ensure_row();
            }
            'F' => {
                let amount = csi_amount(&params);
                self.row = self.row.saturating_sub(amount);
                self.col = 0;
            }
            'H' => {
                self.row = 0;
                self.col = 0;
            }
            'J' => {
                self.clear_for_j(&params);
            }
            'K' => {
                self.clear_for_k(&params);
            }
            'G' => {
                self.col = params.parse::<usize>().unwrap_or(1).saturating_sub(1);
            }
            _ => {}
        }
        i + 1
    }

    fn put(&mut self, ch: char) {
        self.ensure_row();
        if self.rows[self.row].len() <= self.col {
            self.rows[self.row].resize(self.col + 1, ' ');
        }
        self.rows[self.row][self.col] = ch;
        self.col += 1;
    }

    fn newline(&mut self) {
        if let Some(height) = self.height
            && self.row + 1 >= height
        {
            self.scroll_up();
            self.col = 0;
            return;
        }
        self.row += 1;
        self.col = 0;
        self.ensure_row();
    }

    fn move_down(&mut self, amount: usize) {
        self.row += amount;
        if let Some(height) = self.height {
            self.row = self.row.min(height.saturating_sub(1));
        }
    }

    fn scroll_up(&mut self) {
        if self.rows.is_empty() {
            self.rows.push(Vec::new());
            self.row = 0;
            return;
        }
        self.scrollback.push(self.rows.remove(0));
        self.rows.push(Vec::new());
        self.row = self.rows.len().saturating_sub(1);
    }

    fn ensure_row(&mut self) {
        if let Some(height) = self.height {
            if self.rows.len() < height {
                self.rows.resize_with(height, Vec::new);
            }
            self.row = self.row.min(height.saturating_sub(1));
            return;
        }
        if self.rows.len() <= self.row {
            self.rows.resize_with(self.row + 1, Vec::new);
        }
    }

    fn clear_for_j(&mut self, params: &str) {
        match params {
            "" | "0" => {
                self.clear_for_k("0");
                for row in self.row + 1..self.rows.len() {
                    self.rows[row].clear();
                }
            }
            "1" => {
                for row in 0..self.row {
                    self.rows[row].clear();
                }
                self.clear_for_k("1");
            }
            "2" | "3" => {
                self.rows = vec![Vec::new(); self.height.unwrap_or(8)];
                self.row = 0;
                self.col = 0;
            }
            _ => {}
        }
    }

    fn clear_for_k(&mut self, params: &str) {
        self.ensure_row();
        let line = &mut self.rows[self.row];
        match params {
            "" | "0" => line.truncate(self.col.min(line.len())),
            "1" => {
                let end = self.col.saturating_add(1).min(line.len());
                for ch in line.iter_mut().take(end) {
                    *ch = ' ';
                }
            }
            "2" => line.clear(),
            _ => {}
        }
    }

    fn line(&self, row: usize) -> String {
        self.rows
            .get(row)
            .map(|line| line.iter().collect::<String>())
            .unwrap_or_default()
    }

    fn first_non_empty_line(&self) -> Option<usize> {
        self.rows.iter().position(|line| !line.is_empty())
    }

    fn lines(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect()
    }

    fn scrollback_lines(&self) -> Vec<String> {
        self.scrollback
            .iter()
            .map(|line| line.iter().collect::<String>())
            .collect()
    }
}

fn csi_amount(params: &str) -> usize {
    params.parse::<usize>().unwrap_or(1).max(1)
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}
