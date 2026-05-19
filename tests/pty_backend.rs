#[path = "support/env.rs"]
mod env_support;
#[path = "support/shell.rs"]
mod shell_support;

#[path = "pty_backend/common.rs"]
mod common;
#[path = "pty_backend/fish.rs"]
mod fish;
#[path = "pty_backend/support.rs"]
mod support;
#[path = "pty_backend/zsh.rs"]
mod zsh;
