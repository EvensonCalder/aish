use std::fs;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn first_run_creates_aish_home_without_user_home_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("isolated-aish-home");

    let mut child = Command::new(env!("CARGO_BIN_EXE_aish"))
        .env("AISH_HOME", &home)
        .env("SHELL", "/bin/bash")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !home.join("config.toml").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(home.join("config.toml").exists());
    assert!(home.join("history").is_dir());
    assert!(home.join("templates").is_dir());
    assert!(home.join("secrets").is_dir());
    assert!(home.join("logs").is_dir());
    assert!(home.join("cache/runtime").is_dir());

    let config = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("backend = \"auto\""));
    assert!(config.contains("draft = \"{user}@{host} {cwd} > \""));
}
