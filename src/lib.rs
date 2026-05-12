pub mod ai;
pub mod app;
pub mod commands;
pub mod completion;
pub mod config;
pub mod editor;
pub mod encryption;
pub mod history;
pub mod input;
pub mod keybindings;
pub mod log;
pub mod modes;
pub mod paste;
pub mod picker;
pub mod pty;
pub mod shell_integration;
pub mod templates;
pub mod terminal;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
