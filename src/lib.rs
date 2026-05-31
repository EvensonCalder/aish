pub mod ai;
pub mod app;
pub mod commands;
pub mod completion;
pub mod completion_worker;
pub mod config;
pub mod context;
mod display_width;
pub mod editor;
pub mod encrypted_writer;
pub mod encryption;
mod env_name;
mod git_remote;
pub mod history;
pub mod input;
pub mod keybindings;
pub mod log;
pub mod modes;
pub mod paste;
pub mod picker;
mod process_control;
pub mod pty;
pub mod shell_completion;
pub mod shell_integration;
pub mod sync;
pub mod templates;
pub mod terminal;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
