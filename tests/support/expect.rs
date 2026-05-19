use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static EXPECT_RUN_LOCK: Mutex<()> = Mutex::new(());

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
    let _guard = EXPECT_RUN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    let output = Command::new(&expect)
        .arg(&script)
        .env("AISH_BIN", aish_binary())
        .output()
        .expect("failed to launch expect");

    if !output.status.success() {
        panic!(
            "expect script failed: {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            script.display(),
            output.status,
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
