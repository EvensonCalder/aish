use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use crate::process_support::{RunLimiter, default_test_jobs, run_with_timeout, script_timeout};

static EXPECT_RUN_LIMITER: OnceLock<RunLimiter> = OnceLock::new();

fn expect_run_limiter() -> &'static RunLimiter {
    EXPECT_RUN_LIMITER.get_or_init(|| RunLimiter::new(default_test_jobs("AISH_EXPECT_TEST_JOBS")))
}

fn expect_bin() -> Option<PathBuf> {
    for candidate in [
        "/usr/bin/expect",
        "/usr/local/bin/expect",
        "/opt/homebrew/bin/expect",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn scripts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/expect")
}

fn aish_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aish"))
}

pub(crate) fn run_script(name: &str) {
    let _permit = expect_run_limiter().acquire();
    let Some(expect) = expect_bin() else {
        eprintln!("skipping {name}: `expect` not installed");
        return;
    };
    let script = scripts_dir().join(name);
    assert!(
        script.exists(),
        "missing expect script: {}",
        script.display()
    );

    let artifact_dir = tempfile::Builder::new()
        .prefix("aish-expect-")
        .tempdir_in("/tmp")
        .expect("failed to create expect test artifact dir");

    let mut command = Command::new(&expect);
    command
        .arg(&script)
        .env("AISH_BIN", aish_binary())
        .env("AISH_EXPECT_ARTIFACT_DIR", artifact_dir.path());
    let result = run_with_timeout(
        command,
        script_timeout("AISH_EXPECT_TEST_TIMEOUT_SECS", 90),
        || {},
    )
    .expect("failed to launch expect");
    let output = result.output;

    if result.timed_out || !output.status.success() {
        panic!(
            "expect script failed: {}\nstatus: {}\ntimed out: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
            result.timed_out,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

pub(crate) fn list_scripts() -> Vec<String> {
    let dir = scripts_dir();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("cannot read tests/expect") {
        let entry = entry.expect("bad dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("exp") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if name.starts_with('_') {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names
}
