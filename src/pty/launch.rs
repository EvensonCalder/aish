use std::env;
use std::path::Path;
use std::process::{Command, Command as ProcessCommand, Stdio};

use super::control::CONTROL_FD;
use super::{ShellIntegration, ready_marker, start_marker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellLaunch {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) init_command: String,
    pub(super) integration: ShellIntegration,
}

pub fn resolve_shell(configured_shell: &str) -> String {
    let configured_shell = configured_shell.trim();
    if configured_shell != "auto" && !configured_shell.is_empty() {
        return configured_shell.to_string();
    }
    env::var("SHELL")
        .ok()
        .map(|shell| shell.trim().to_string())
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string())
}

pub(super) fn shell_launch(configured_shell: &str) -> ShellLaunch {
    let program = resolve_shell(configured_shell);
    let shell_name = shell_name(&program);
    let ready_marker = ready_marker();
    let start_marker = start_marker();
    let control_fd = CONTROL_FD;

    let (args, init_command, integration) = match shell_name.as_str() {
        "bash" => (
            vec!["-i".to_string()],
            format!(
                " set +o history 2>/dev/null || true\n export HISTCONTROL=ignorespace${{HISTCONTROL:+:$HISTCONTROL}}; unset AISH_CONTROL_FD; __aish_prompt_command_set=0; __aish_prompt_command_is_array=0; __aish_prompt_command_string=; __aish_prompt_command_array=(); if declare -p PROMPT_COMMAND >/dev/null 2>&1; then __aish_prompt_command_set=1; case \"$(declare -p PROMPT_COMMAND 2>/dev/null)\" in declare\\ -a*|declare\\ -A*) __aish_prompt_command_is_array=1; __aish_prompt_command_array=(\"${{PROMPT_COMMAND[@]}}\");; *) __aish_prompt_command_string=$PROMPT_COMMAND;; esac; fi; PROMPT_COMMAND=; trap - DEBUG 2>/dev/null || true; __aish_preserve_status() {{ return \"$1\"; }}; __aish_run_prompt_command() {{ if [ \"$__aish_prompt_command_set\" = 1 ]; then if [ \"$__aish_prompt_command_is_array\" = 1 ]; then local __aish_pc; for __aish_pc in \"${{__aish_prompt_command_array[@]}}\"; do eval \"$__aish_pc\"; done; else eval \"$__aish_prompt_command_string\"; fi; fi; }}; __aish_emit_ready() {{ local __aish_status=$?; {{ printf '{ready_marker}\\t%s\\t%s\\n' \"$__aish_status\" \"$PWD\" >&{control_fd}; }} 2>/dev/null || true; stty -echo; __aish_run_prompt_command >/dev/null 2>&1; return \"$__aish_status\"; }}; PROMPT_COMMAND=__aish_emit_ready; bind 'set enable-bracketed-paste off' 2>/dev/null || true; PS1=''; PS2=''; set -o history 2>/dev/null || true; stty -echo; __aish_emit_ready\n"
            ),
            ShellIntegration::BashPromptCommand,
        ),
        "zsh" => (
            vec![
                "-i".to_string(),
                "-o".to_string(),
                "histignorespace".to_string(),
            ],
            format!(
                " setopt histignorespace; unset AISH_CONTROL_FD; stty -echo; unsetopt zle prompt_cr prompt_sp; PROMPT=''; RPROMPT=''; PROMPT2=''; autoload -Uz add-zsh-hook; typeset -ga __aish_user_preexec_functions __aish_user_precmd_functions; __aish_user_preexec_functions=(${{preexec_functions:#__aish_preexec}}); __aish_user_precmd_functions=(${{precmd_functions:#__aish_precmd}}); function __aish_preserve_status() {{ return \"$1\"; }}; function __aish_run_user_preexec() {{ local __aish_fn; for __aish_fn in ${{__aish_user_preexec_functions[@]}}; do if functions \"$__aish_fn\" >/dev/null 2>&1; then \"$__aish_fn\" \"$@\" >/dev/null 2>&1; fi; done; }}; function __aish_run_user_precmd() {{ local __aish_fn; for __aish_fn in ${{__aish_user_precmd_functions[@]}}; do if functions \"$__aish_fn\" >/dev/null 2>&1; then \"$__aish_fn\" >/dev/null 2>&1; fi; done; }}; function __aish_emit_start() {{ {{ printf '{start_marker}\\t%s\\n' \"$1\" >&{control_fd}; }} 2>/dev/null || true; }}; function __aish_preexec() {{ stty echo; __aish_run_user_preexec \"$@\"; __aish_emit_start \"$1\"; }}; function __aish_precmd() {{ local __aish_status=$?; __aish_run_user_precmd; stty -echo; {{ printf '{ready_marker}\\t%s\\t%s\\n' \"$__aish_status\" \"$PWD\" >&{control_fd}; }} 2>/dev/null || true; return \"$__aish_status\"; }}; preexec_functions=(__aish_preexec); precmd_functions=(__aish_precmd); fc -p 2>/dev/null || true; __aish_precmd\n"
            ),
            ShellIntegration::ZshHooks,
        ),
        "fish" => (
            fish_launch_args(&program),
            format!(
                "set -e AISH_CONTROL_FD; stty -echo; set -g fish_greeting; function fish_title; end; function __aish_preserve_status; return $argv[1]; end; function __aish_preexec --on-event fish_preexec; stty echo; printf '{start_marker}\\t%s\\n' $argv[1] >&{control_fd} 2>/dev/null; or true; end; function __aish_emit_ready; set -l __aish_status $status; if test (count $argv) -gt 0; set __aish_status $argv[1]; end; printf '{ready_marker}\\t%s\\t%s\\n' $__aish_status $PWD >&{control_fd} 2>/dev/null; or true; return $__aish_status; end; function __aish_postexec --on-event fish_postexec; set -l __aish_status $status; stty -echo; __aish_emit_ready $__aish_status; end; function fish_prompt; end; function fish_right_prompt; end; function fish_mode_prompt; end; __aish_emit_ready\n"
            ),
            ShellIntegration::FishEvents,
        ),
        _ => (
            Vec::new(),
            format!(
                "__aish_preserve_status() {{ return \"$1\"; }}; stty -echo; printf '\\n{ready_marker}\\t%s\\n' \"$PWD\"\n"
            ),
            ShellIntegration::MarkerCommand,
        ),
    };

    ShellLaunch {
        program,
        args,
        init_command,
        integration,
    }
}

fn shell_name(program: &str) -> String {
    let name = Path::new(program.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .trim_start_matches('-')
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

fn fish_launch_args(program: &str) -> Vec<String> {
    let mut args = Vec::new();
    if fish_supports_features(program, "no-query-term,no-mark-prompt") {
        args.push("--features".to_string());
        args.push("no-query-term,no-mark-prompt".to_string());
    }
    args
}

fn fish_supports_features(program: &str, features: &str) -> bool {
    ProcessCommand::new(program)
        .args(["--no-config", "--features", features, "-c", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn shell_command_builder(launch: &ShellLaunch) -> Command {
    let mut command = Command::new(&launch.program);
    for arg in &launch.args {
        command.arg(arg);
    }
    if let Ok(cwd) = env::current_dir() {
        command.current_dir(cwd);
    }
    command.env("BASH_SILENCE_DEPRECATION_WARNING", "1");
    command.env_remove("AISH_CONTROL_FD");
    command
}
