use std::fs;
use std::time::{Duration, Instant};

use aish::pty::PtyBackend;

use crate::support::{EnvVarGuard, find_shell, fish_backend_tests_enabled, pty_test_guard};

#[test]
fn fish_pty_backend_streams_output_before_command_completion_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend streaming test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let mut backend = PtyBackend::spawn(fish).unwrap();
    let started = Instant::now();
    let mut first_chunk_at = None;
    let mut displayed = Vec::new();

    let result = backend
        .run_command_streaming_with_wait_callback(
            "printf 'fish-stream-first\\n'; sleep 2; printf 'fish-stream-second\\n'",
            Duration::from_secs(5),
            |_| Ok(false),
            |chunk| {
                if first_chunk_at.is_none() {
                    first_chunk_at = Some(started.elapsed());
                }
                displayed.extend_from_slice(chunk);
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert!(
        first_chunk_at.unwrap() < Duration::from_millis(1500),
        "first streamed fish chunk arrived after command completion: {first_chunk_at:?}"
    );
    let displayed = String::from_utf8(displayed).unwrap();
    assert!(displayed.contains("fish-stream-first"), "{displayed:?}");
    assert!(displayed.contains("fish-stream-second"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_READY__"), "{displayed:?}");
    assert!(!displayed.contains("__AISH_START__"), "{displayed:?}");
    assert!(!displayed.contains('\u{23ce}'), "{displayed:?}");
}

#[test]
fn fish_pty_backend_runs_commands_and_reports_cwd_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let mut backend = PtyBackend::spawn(fish).unwrap();
    let first = backend
        .run_command("printf 'fish-ok\\n'", Duration::from_secs(5))
        .unwrap();

    assert_eq!(first.exit_code, 0);
    assert_eq!(
        first.started_command.as_deref(),
        Some("printf 'fish-ok\\n'")
    );
    assert_eq!(first.output.trim(), "fish-ok");
    assert_eq!(first.cwd.as_deref(), Some(backend.initial_cwd().unwrap()));

    let cd = backend
        .run_command("cd /tmp", Duration::from_secs(5))
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let pwd = backend.run_command("pwd", Duration::from_secs(5)).unwrap();
    assert_eq!(pwd.exit_code, 0);
    assert_eq!(pwd.started_command.as_deref(), Some("pwd"));
    assert_eq!(pwd.cwd.as_deref(), Some("/tmp"));
    assert_eq!(pwd.output.trim(), "/tmp");
}

#[test]
fn fish_pty_backend_does_not_record_aish_commands_in_native_history_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend history test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(fish).unwrap();

    let user_command = "echo aish-fish-history-target";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-fish-history-target");

    let history = backend
        .run_command(
            "history search --exact 'echo aish-fish-history-target'",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(history.exit_code, 0);
    assert!(!history.output.contains(user_command), "{history:?}");
    assert!(!history.output.contains("__AISH_START__"));
    assert!(!history.output.contains("__AISH_READY__"));
}

#[test]
fn fish_pty_backend_does_not_flush_aish_commands_to_history_file_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend disk history test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let data_home = home.path().join("data");
    let fish_data_dir = data_home.join("fish");
    fs::create_dir_all(&fish_data_dir).unwrap();
    let history_path = fish_data_dir.join("fish_history");
    fs::write(&history_path, "- cmd: preexisting-fish-history\n").unwrap();
    let fish_config_dir = home.path().join(".config/fish");
    fs::create_dir_all(&fish_config_dir).unwrap();
    fs::write(
        fish_config_dir.join("config.fish"),
        "set -g fish_history fish\n",
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let _xdg_guard = EnvVarGuard::set("XDG_DATA_HOME", data_home.as_os_str());
    let mut backend = PtyBackend::spawn(fish).unwrap();

    let user_command = "echo aish-fish-disk-history-target";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-fish-disk-history-target");

    let flush = backend
        .run_command("history save; or true", Duration::from_secs(5))
        .unwrap();
    assert_eq!(flush.exit_code, 0);

    let disk_history = fs::read_to_string(&history_path).unwrap();
    assert_eq!(disk_history, "- cmd: preexisting-fish-history\n");
    assert!(!disk_history.contains("aish-fish-disk-history-target"));
}

#[test]
fn fish_pty_backend_does_not_export_control_fd_to_user_commands_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let mut backend = PtyBackend::spawn(fish).unwrap();
    let result = backend
        .run_command(
            "set -q AISH_CONTROL_FD; and printf 'control-fd=%s\\n' $AISH_CONTROL_FD; or printf 'control-fd=unset\\n'",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output.trim(), "control-fd=unset");
}

#[test]
fn fish_pty_backend_preserves_output_that_matches_command_tokens_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let temp = tempfile::Builder::new()
        .prefix("aish-fish-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut backend = PtyBackend::spawn(fish).unwrap();
    let cd = backend
        .run_command(
            &format!("cd {}", temp.path().display()),
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(cd.exit_code, 0);

    let setup = backend
        .run_command(
            "mkdir -p c; printf 'alpha\\nbeta\\n' > c/i",
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(setup.exit_code, 0);

    let grep = backend
        .run_command("cat c/i | grep beta", Duration::from_secs(5))
        .unwrap();
    assert_eq!(grep.exit_code, 0);
    assert_eq!(grep.output.trim(), "beta", "{grep:?}");

    let file_test = backend
        .run_command("test -f c/i; and echo file-exists", Duration::from_secs(5))
        .unwrap();
    assert_eq!(file_test.exit_code, 0);
    assert_eq!(file_test.output.trim(), "file-exists", "{file_test:?}");

    let echo = backend
        .run_command("echo after-failure", Duration::from_secs(5))
        .unwrap();
    assert_eq!(echo.exit_code, 0);
    assert_eq!(echo.output.trim(), "after-failure", "{echo:?}");
}

#[test]
fn fish_pty_backend_loads_user_config_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend config test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    let fish_config_dir = home.path().join(".config/fish");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&fish_config_dir).unwrap();
    fs::write(
        bin_dir.join("from-aish-fish-config-path"),
        "#!/bin/sh\nprintf 'path-from-fish-config\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(bin_dir.join("from-aish-fish-config-path"))
        .status()
        .unwrap();
    fs::write(
        fish_config_dir.join("config.fish"),
        format!(
            "function aish_function_from_fish_config\n\
                 printf 'function-from-fish-config\\n'\n\
             end\n\
             set -gx AISH_FISH_RC_ENV env-from-fish-config\n\
             set -gx PATH {} $PATH\n\
             function aish_user_fish_preexec --on-event fish_preexec\n\
                 set -gx AISH_FISH_USER_PREEXEC $argv[1]\n\
             end\n\
             function aish_user_fish_postexec --on-event fish_postexec\n\
                 set -gx AISH_FISH_USER_POSTEXEC ran\n\
             end\n\
             function fish_prompt\n\
                 printf 'fish-config-prompt> '\n\
             end\n",
            bin_dir.display()
        ),
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(fish).unwrap();

    let function = backend
        .run_command("aish_function_from_fish_config", Duration::from_secs(5))
        .unwrap();
    let env = backend
        .run_command("printf '%s\\n' $AISH_FISH_RC_ENV", Duration::from_secs(5))
        .unwrap();
    let path = backend
        .run_command("from-aish-fish-config-path", Duration::from_secs(5))
        .unwrap();
    let events = backend
        .run_command(
            "printf '%s|%s\\n' $AISH_FISH_USER_POSTEXEC $AISH_FISH_USER_PREEXEC",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(function.exit_code, 0);
    assert_eq!(function.output.trim(), "function-from-fish-config");
    assert_eq!(env.exit_code, 0);
    assert_eq!(env.output.trim(), "env-from-fish-config");
    assert_eq!(path.exit_code, 0);
    assert_eq!(path.output.trim(), "path-from-fish-config");
    assert_eq!(events.exit_code, 0);
    assert!(
        events.output.trim().starts_with("ran|printf "),
        "{events:?}"
    );
    for result in [&function, &env, &path, &events] {
        assert!(!result.output.contains("fish-config-prompt"), "{result:?}");
        assert!(!result.output.contains("__AISH_READY__"), "{result:?}");
    }
}

#[test]
fn fish_pty_backend_wraps_user_event_handlers_when_available() {
    let _guard = pty_test_guard();
    if !fish_backend_tests_enabled() {
        eprintln!("skipping fish PTY backend event test: set AISH_TEST_FISH=1 to opt in");
        return;
    }
    let Some(fish) = find_shell(&[
        "/opt/homebrew/bin/fish",
        "/usr/local/bin/fish",
        "/usr/bin/fish",
        "/bin/fish",
    ]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let fish_config_dir = home.path().join(".config/fish");
    fs::create_dir_all(&fish_config_dir).unwrap();
    fs::write(
        fish_config_dir.join("config.fish"),
        "function aish_user_fish_preexec --on-event fish_preexec\n\
             set -gx AISH_FISH_DIRECT_PREEXEC $argv[1]\n\
             printf 'fish-direct-preexec-noise\\n'\n\
         end\n\
         function aish_user_fish_postexec --on-event fish_postexec\n\
             set -gx AISH_FISH_DIRECT_POSTEXEC ran\n\
             set -gx AISH_FISH_DIRECT_POSTEXEC_STATUS $status\n\
             printf 'fish-direct-postexec-noise\\n'\n\
         end\n\
         function fish_prompt\n\
             printf 'fish-direct-prompt> '\n\
         end\n",
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(fish).unwrap();

    let command = backend
        .run_command("printf 'fish-direct-body\\n'", Duration::from_secs(5))
        .unwrap();
    let failure = backend
        .run_command("false", Duration::from_secs(5))
        .unwrap();
    let events = backend
        .run_command(
            "printf '%s|%s|%s\\n' $AISH_FISH_DIRECT_POSTEXEC $AISH_FISH_DIRECT_POSTEXEC_STATUS $AISH_FISH_DIRECT_PREEXEC",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(command.exit_code, 0);
    assert_eq!(command.output.trim(), "fish-direct-body");
    assert_eq!(failure.exit_code, 1);
    assert!(failure.output.trim().is_empty(), "{failure:?}");
    assert_eq!(events.exit_code, 0);
    assert!(
        events.output.trim().starts_with("ran|1|printf "),
        "{events:?}"
    );
    for result in [&command, &failure, &events] {
        assert!(
            !result.output.contains("fish-direct-preexec-noise"),
            "{result:?}"
        );
        assert!(
            !result.output.contains("fish-direct-postexec-noise"),
            "{result:?}"
        );
        assert!(!result.output.contains("fish-direct-prompt"), "{result:?}");
    }
}
