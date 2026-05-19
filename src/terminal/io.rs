use std::io::{Result as IoResult, Write, stdout};
use std::panic;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Key(KeyEvent),
    Paste(String),
    Resize(u16, u16),
    Tick,
    Ignore,
}

pub struct TerminalGuard;

pub struct CrLfWriter<'a, W: Write> {
    inner: &'a mut W,
    previous_was_cr: bool,
}

impl<'a, W: Write> CrLfWriter<'a, W> {
    pub(crate) fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            previous_was_cr: false,
        }
    }
}

impl<W: Write> Write for CrLfWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        for &byte in buf {
            if byte == b'\n' && !self.previous_was_cr {
                self.inner.write_all(b"\r")?;
            }
            self.inner.write_all(&[byte])?;
            self.previous_was_cr = byte == b'\r';
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), crossterm::event::EnableBracketedPaste)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), crossterm::event::DisableBracketedPaste);
        if is_raw_mode_enabled().unwrap_or(false) {
            let _ = disable_raw_mode();
        }
    }
}

pub fn install_panic_cleanup() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), crossterm::event::DisableBracketedPaste);
        let _ = disable_raw_mode();
        previous(info);
    }));
}

pub(super) fn read_frontend_event(tick_interval: Duration) -> Result<TerminalEvent> {
    if !event::poll(tick_interval)? {
        return Ok(TerminalEvent::Tick);
    }
    Ok(terminal_event_from_crossterm(event::read()?))
}

pub(super) fn terminal_event_from_crossterm(event: Event) -> TerminalEvent {
    match event {
        Event::Key(key) => TerminalEvent::Key(key),
        Event::Paste(text) => TerminalEvent::Paste(text),
        Event::Resize(cols, rows) => TerminalEvent::Resize(cols, rows),
        _ => TerminalEvent::Ignore,
    }
}
