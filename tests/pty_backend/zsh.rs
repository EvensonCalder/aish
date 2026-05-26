use std::fs;
use std::path::Path;
use std::time::Duration;

use aish::pty::PtyBackend;

use crate::support::{EnvVarGuard, find_shell, pty_test_guard};

#[test]
fn zsh_pty_backend_does_not_export_control_fd_to_user_commands_when_available() {
    let _guard = pty_test_guard();
    let Some(zsh) = find_shell(&["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]) else {
        return;
    };
    let mut backend = PtyBackend::spawn(zsh).unwrap();

    let result = backend
        .run_command(
            "printf 'control-fd=%s\\n' \"${AISH_CONTROL_FD-unset}\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output.trim(), "control-fd=unset");
}

#[test]
fn zsh_pty_backend_runs_commands_and_preserves_shell_state_when_available() {
    let _guard = pty_test_guard();
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let mut backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let first = backend
        .run_command("printf 'zsh-ok\\n'", Duration::from_secs(5))
        .unwrap();
    assert_eq!(first.exit_code, 0);
    assert_eq!(first.started_command.as_deref(), Some("printf 'zsh-ok\\n'"));
    assert_eq!(first.output.trim(), "zsh-ok");
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
fn zsh_pty_backend_does_not_record_aish_commands_in_native_history() {
    let _guard = pty_test_guard();
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let user_command = "printf 'aish-zsh-history-target\\n'";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-zsh-history-target");

    let history = backend
        .run_command("fc -l 1 2>/dev/null || true", Duration::from_secs(5))
        .unwrap();

    assert_eq!(history.exit_code, 0);
    assert!(!history.output.contains(user_command), "{history:?}");
    assert!(!history.output.contains("__aish_status=$?"));
    assert!(!history.output.contains("__AISH_STATUS__"));
    assert!(!history.output.contains("__AISH_READY__"));
}

#[test]
fn zsh_pty_backend_does_not_flush_aish_commands_to_zsh_history_file() {
    let _guard = pty_test_guard();
    if !Path::new("/bin/zsh").exists() {
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let history_path = home.path().join(".zsh_history");
    fs::write(&history_path, ": 1:0;preexisting-zsh-history\n").unwrap();
    fs::write(
        home.path().join(".zshrc"),
        "HISTFILE=$HOME/.zsh_history\n\
         HISTSIZE=1000\n\
         SAVEHIST=1000\n\
         setopt append_history inc_append_history share_history\n",
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn("/bin/zsh").unwrap();

    let user_command = "printf 'aish-zsh-disk-history-target\\n'";
    let run = backend
        .run_command(user_command, Duration::from_secs(5))
        .unwrap();
    assert_eq!(run.exit_code, 0);
    assert_eq!(run.output.trim(), "aish-zsh-disk-history-target");

    let flush = backend
        .run_command("fc -W 2>/dev/null || true", Duration::from_secs(5))
        .unwrap();
    assert_eq!(flush.exit_code, 0);

    let disk_history = fs::read_to_string(&history_path).unwrap();
    assert_eq!(disk_history, ": 1:0;preexisting-zsh-history\n");
    assert!(!disk_history.contains("aish-zsh-disk-history-target"));
}

#[test]
fn zsh_pty_backend_loads_user_zshrc_when_available() {
    let _guard = pty_test_guard();
    let Some(zsh) = find_shell(&["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    let bin_dir = home.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(
        bin_dir.join("from-aish-zshrc-path"),
        "#!/bin/sh\nprintf 'path-from-zshrc\\n'\n",
    )
    .unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(bin_dir.join("from-aish-zshrc-path"))
        .status()
        .unwrap();
    fs::write(
        home.path().join(".zshrc"),
        format!(
            "alias aish_alias_from_zshrc='printf alias-from-zshrc\\\\n'\n\
             aish_function_from_zshrc() {{ printf 'function-from-zshrc\\n'; }}\n\
             function aish_user_preexec() {{ export AISH_ZSH_USER_PREEXEC=\"$1\"; }}\n\
             function aish_user_precmd() {{ export AISH_ZSH_USER_PRECMD=ran; }}\n\
             autoload -Uz add-zsh-hook\n\
             add-zsh-hook preexec aish_user_preexec\n\
             add-zsh-hook precmd aish_user_precmd\n\
             export AISH_ZSH_RC_ENV=env-from-zshrc\n\
             export PATH=\"{}:$PATH\"\n\
             PROMPT='zshrc-prompt> '\n",
            bin_dir.display()
        ),
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(zsh).unwrap();

    let alias = backend
        .run_command("aish_alias_from_zshrc", Duration::from_secs(5))
        .unwrap();
    let function = backend
        .run_command("aish_function_from_zshrc", Duration::from_secs(5))
        .unwrap();
    let env = backend
        .run_command(
            "printf '%s\\n' \"$AISH_ZSH_RC_ENV\"",
            Duration::from_secs(5),
        )
        .unwrap();
    let path = backend
        .run_command("from-aish-zshrc-path", Duration::from_secs(5))
        .unwrap();
    let hooks = backend
        .run_command(
            "printf '%s|%s\\n' \"$AISH_ZSH_USER_PRECMD\" \"$AISH_ZSH_USER_PREEXEC\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(alias.exit_code, 0);
    assert_eq!(alias.output.trim(), "alias-from-zshrc");
    assert_eq!(function.exit_code, 0);
    assert_eq!(function.output.trim(), "function-from-zshrc");
    assert_eq!(env.exit_code, 0);
    assert_eq!(env.output.trim(), "env-from-zshrc");
    assert_eq!(path.exit_code, 0);
    assert_eq!(path.output.trim(), "path-from-zshrc");
    assert_eq!(hooks.exit_code, 0);
    assert!(hooks.output.trim().starts_with("ran|printf "), "{hooks:?}");
    for result in [&alias, &function, &env, &path, &hooks] {
        assert!(!result.output.contains("zshrc-prompt"), "{result:?}");
        assert!(!result.output.contains("__AISH_READY__"), "{result:?}");
    }
}

#[test]
fn zsh_pty_backend_preserves_direct_hook_functions_when_available() {
    let _guard = pty_test_guard();
    let Some(zsh) = find_shell(&["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]) else {
        return;
    };

    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join(".zshrc"),
        "function preexec() { export AISH_ZSH_DIRECT_PREEXEC=\"$1\"; printf zsh-direct-preexec-noise\\\\n; }\n\
         function precmd() { export AISH_ZSH_DIRECT_PRECMD=ran; printf zsh-direct-precmd-noise\\\\n; }\n\
         PROMPT='zsh-direct-prompt> '\n",
    )
    .unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path().as_os_str());
    let mut backend = PtyBackend::spawn(zsh).unwrap();

    let command = backend
        .run_command("printf 'zsh-direct-body\\n'", Duration::from_secs(5))
        .unwrap();
    let hooks = backend
        .run_command(
            "printf '%s|%s\\n' \"$AISH_ZSH_DIRECT_PRECMD\" \"$AISH_ZSH_DIRECT_PREEXEC\"",
            Duration::from_secs(5),
        )
        .unwrap();

    assert_eq!(command.exit_code, 0);
    assert_eq!(command.output.trim(), "zsh-direct-body");
    assert_eq!(hooks.exit_code, 0);
    assert!(hooks.output.trim().starts_with("ran|printf "), "{hooks:?}");
    for result in [&command, &hooks] {
        assert!(
            !result.output.contains("zsh-direct-preexec-noise"),
            "{result:?}"
        );
        assert!(
            !result.output.contains("zsh-direct-precmd-noise"),
            "{result:?}"
        );
        assert!(!result.output.contains("zsh-direct-prompt"), "{result:?}");
    }
}
