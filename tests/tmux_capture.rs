use std::path::PathBuf;
use std::process::Command;

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn tmux_output_visibility_matches_real_terminal_screen() {
    let captured = run_tmux_script("output_visibility.sh");
    let expected_user = std::env::var("USER").unwrap_or_else(|_| "evenson".to_string());
    assert_adjacent_output(&captured, "whoami", &expected_user);
    assert_adjacent_output(&captured, "echo 123", "123");
}

#[test]
fn tmux_unicode_output_matches_real_terminal_screen() {
    let captured = run_tmux_script("unicode_input.sh");
    assert_adjacent_output(
        &captured,
        "printf 'unicode:%s\\n' 'café-你好'",
        "unicode:café-你好",
    );
}

fn run_tmux_script(name: &str) -> String {
    if !tmux_available() {
        eprintln!("skipping {name}: tmux not installed");
        return String::new();
    }

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let script = repo.join("tests/tmux").join(name);
    assert!(script.exists(), "missing tmux script: {}", script.display());

    let output = Command::new("sh")
        .arg(&script)
        .current_dir(&repo)
        .output()
        .expect("failed to launch tmux script");

    if !output.status.success() {
        panic!(
            "tmux script failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_adjacent_output(captured: &str, command: &str, expected_output: &str) {
    let lines: Vec<&str> = captured.lines().collect();
    for pair in lines.windows(2) {
        if pair[0].ends_with(command) && pair[1] == expected_output {
            return;
        }
    }
    panic!(
        "expected {expected_output:?} immediately after {command:?}; captured pane was {captured:?}"
    );
}
