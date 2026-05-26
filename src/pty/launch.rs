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
            bash_init_command(ready_marker, control_fd),
            ShellIntegration::BashPromptCommand,
        ),
        "zsh" => (
            vec![
                "-i".to_string(),
                "-o".to_string(),
                "histignorespace".to_string(),
            ],
            zsh_init_command(ready_marker, start_marker, control_fd),
            ShellIntegration::ZshHooks,
        ),
        "fish" => (
            fish_launch_args(&program),
            fish_init_command(ready_marker, start_marker, control_fd),
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
    if fish_supports_private_mode(program) {
        args.push("--private".to_string());
    }
    if fish_supports_features(program, "no-query-term,no-mark-prompt") {
        args.push("--features".to_string());
        args.push("no-query-term,no-mark-prompt".to_string());
    }
    args
}

fn bash_init_command(ready_marker: &str, control_fd: i32) -> String {
    fill_shell_init_template(
        r#" set +o history 2>/dev/null || true
stty -echo
 {
export HISTCONTROL=ignorespace${HISTCONTROL:+:$HISTCONTROL}
unset AISH_CONTROL_FD
__aish_prompt_command_set=0
__aish_prompt_command_is_array=0
__aish_prompt_command_string=
__aish_prompt_command_array=()
if declare -p PROMPT_COMMAND >/dev/null 2>&1; then
  __aish_prompt_command_set=1
  case "$(declare -p PROMPT_COMMAND 2>/dev/null)" in
    declare\ -a*|declare\ -A*)
      __aish_prompt_command_is_array=1
      __aish_prompt_command_array=("${PROMPT_COMMAND[@]}")
      ;;
    *)
      __aish_prompt_command_string=$PROMPT_COMMAND
      ;;
  esac
fi
PROMPT_COMMAND=
trap - DEBUG 2>/dev/null || true
__aish_preserve_status() {
  return "$1"
}
__aish_disable_history() {
  unset HISTFILE
  HISTSIZE=0
  HISTIGNORE='*'
  set +o history 2>/dev/null || true
  history -c 2>/dev/null || true
}
__aish_run_prompt_command() {
  if [ "$__aish_prompt_command_set" = 1 ]; then
    if [ "$__aish_prompt_command_is_array" = 1 ]; then
      local __aish_pc
      for __aish_pc in "${__aish_prompt_command_array[@]}"; do
        eval "$__aish_pc"
      done
    else
      eval "$__aish_prompt_command_string"
    fi
  fi
}
__aish_emit_ready() {
  local __aish_status=$?
  __aish_disable_history
  {
    printf '@READY_MARKER@\t%s\t%s\n' "$__aish_status" "$PWD" >&@CONTROL_FD@
  } 2>/dev/null || true
  stty -echo
  __aish_run_prompt_command >/dev/null 2>&1
  __aish_disable_history
  return "$__aish_status"
}
bind 'set enable-bracketed-paste off' 2>/dev/null || true
PS0=''
PS1=''
PS2=''
__aish_disable_history
stty -echo
PROMPT_COMMAND=__aish_emit_ready
}
"#,
        ready_marker,
        None,
        control_fd,
    )
}

fn zsh_init_command(ready_marker: &str, start_marker: &str, control_fd: i32) -> String {
    fill_shell_init_template(
        r#" setopt histignorespace; unset HISTFILE; HISTSIZE=0; SAVEHIST=0
 unsetopt append_history inc_append_history inc_append_history_time share_history 2>/dev/null || true
 fc -p /dev/null 0 0 2>/dev/null || true
stty -echo
 {
unset AISH_CONTROL_FD
unset HISTFILE
HISTSIZE=0
SAVEHIST=0
unsetopt append_history inc_append_history inc_append_history_time share_history 2>/dev/null || true
fc -p /dev/null 0 0 2>/dev/null || true
unsetopt zle prompt_cr prompt_sp
PROMPT=''
RPROMPT=''
PROMPT2=''
autoload -Uz add-zsh-hook
typeset -ga __aish_user_preexec_functions
typeset -ga __aish_user_precmd_functions
__aish_user_preexec_functions=(${preexec_functions:#__aish_preexec})
__aish_user_precmd_functions=(${precmd_functions:#__aish_precmd})
if functions preexec >/dev/null 2>&1; then
  functions -c preexec __aish_user_preexec_function 2>/dev/null || true
  unfunction preexec 2>/dev/null || true
fi
if functions precmd >/dev/null 2>&1; then
  functions -c precmd __aish_user_precmd_function 2>/dev/null || true
  unfunction precmd 2>/dev/null || true
fi
function __aish_preserve_status() {
  return "$1"
}
function __aish_disable_history() {
  unset HISTFILE
  HISTSIZE=0
  SAVEHIST=0
  unsetopt append_history inc_append_history inc_append_history_time share_history 2>/dev/null || true
}
function __aish_run_user_preexec() {
  if functions __aish_user_preexec_function >/dev/null 2>&1; then
    __aish_user_preexec_function "$@" >/dev/null 2>&1
  fi
  local __aish_fn
  for __aish_fn in ${__aish_user_preexec_functions[@]}; do
    if functions "$__aish_fn" >/dev/null 2>&1; then
      "$__aish_fn" "$@" >/dev/null 2>&1
    fi
  done
}
function __aish_run_user_precmd() {
  if functions __aish_user_precmd_function >/dev/null 2>&1; then
    __aish_user_precmd_function >/dev/null 2>&1
  fi
  local __aish_fn
  for __aish_fn in ${__aish_user_precmd_functions[@]}; do
    if functions "$__aish_fn" >/dev/null 2>&1; then
      "$__aish_fn" >/dev/null 2>&1
    fi
  done
}
function __aish_emit_start() {
  {
    printf '@START_MARKER@\t%s\n' "$1" >&@CONTROL_FD@
  } 2>/dev/null || true
}
function __aish_preexec() {
  stty echo
  __aish_disable_history
  __aish_run_user_preexec "$@"
  __aish_emit_start "$1"
}
function __aish_precmd() {
  local __aish_status=$?
  __aish_disable_history
  __aish_run_user_precmd
  __aish_disable_history
  stty -echo
  {
    printf '@READY_MARKER@\t%s\t%s\n' "$__aish_status" "$PWD" >&@CONTROL_FD@
  } 2>/dev/null || true
  return "$__aish_status"
}
preexec_functions=(__aish_preexec)
precmd_functions=(__aish_precmd)
__aish_disable_history
}
"#,
        ready_marker,
        Some(start_marker),
        control_fd,
    )
}

fn fish_init_command(ready_marker: &str, start_marker: &str, control_fd: i32) -> String {
    fill_shell_init_template(
        r#"set -g fish_history ""; builtin history clear-session >/dev/null 2>&1; or true
stty -echo
begin
set -g __aish_initializing
set -e AISH_CONTROL_FD
set -g fish_greeting
set -g fish_history ""
function __aish_clear_fish_history
  set -g fish_history ""
  builtin history clear-session >/dev/null 2>&1; or true
end
__aish_clear_fish_history
function fish_title
end
set -g __aish_user_fish_preexec_functions
set -g __aish_user_fish_postexec_functions
set -l __aish_i 0
for __aish_line in (functions --handlers-type generic 2>/dev/null | string match -r '^fish_preexec[ \t].+')
  set -l __aish_fn (string replace -r '^fish_preexec[ \t]+' '' -- $__aish_line)
  if test "$__aish_fn" != "__aish_preexec"
    set __aish_i (math $__aish_i + 1)
    set -l __aish_copy "__aish_user_fish_preexec_$__aish_i"
    functions -c $__aish_fn $__aish_copy 2>/dev/null
    and functions -e $__aish_fn 2>/dev/null
    and set -a __aish_user_fish_preexec_functions $__aish_copy
  end
end
set __aish_i 0
for __aish_line in (functions --handlers-type generic 2>/dev/null | string match -r '^fish_postexec[ \t].+')
  set -l __aish_fn (string replace -r '^fish_postexec[ \t]+' '' -- $__aish_line)
  if test "$__aish_fn" != "__aish_postexec"
    set __aish_i (math $__aish_i + 1)
    set -l __aish_copy "__aish_user_fish_postexec_$__aish_i"
    functions -c $__aish_fn $__aish_copy 2>/dev/null
    and functions -e $__aish_fn 2>/dev/null
    and set -a __aish_user_fish_postexec_functions $__aish_copy
  end
end
function __aish_preserve_status
  return $argv[1]
end
function __aish_restore_status
  return $argv[1]
end
function __aish_run_user_fish_preexec
  for __aish_fn in $__aish_user_fish_preexec_functions
    $__aish_fn $argv >/dev/null 2>&1
  end
end
function __aish_run_user_fish_postexec
  set -l __aish_status $argv[1]
  set -e argv[1]
  for __aish_fn in $__aish_user_fish_postexec_functions
    __aish_restore_status $__aish_status
    $__aish_fn $argv >/dev/null 2>&1
  end
end
function __aish_preexec --on-event fish_preexec
  if set -q __aish_initializing
    return
  end
  __aish_clear_fish_history
  stty echo
  __aish_run_user_fish_preexec $argv
  printf '@START_MARKER@\t%s\n' $argv[1] >&@CONTROL_FD@ 2>/dev/null; or true
end
function __aish_emit_ready
  set -l __aish_status $status
  if test (count $argv) -gt 0
    set __aish_status $argv[1]
  end
  printf '@READY_MARKER@\t%s\t%s\n' $__aish_status $PWD >&@CONTROL_FD@ 2>/dev/null; or true
  return $__aish_status
end
function __aish_postexec --on-event fish_postexec
  set -l __aish_status $status
  if set -q __aish_initializing
    return
  end
  if set -q __aish_suppress_next_postexec
    set -e __aish_suppress_next_postexec
    return
  end
  stty -echo
  __aish_run_user_fish_postexec $__aish_status $argv
  __aish_clear_fish_history
  __aish_emit_ready $__aish_status
end
function fish_prompt
end
function fish_right_prompt
end
function fish_mode_prompt
end
function __aish_finish_init
  set -g __aish_suppress_next_postexec
  set -e __aish_initializing
  __aish_clear_fish_history
  __aish_emit_ready
end
end
__aish_finish_init
"#,
        ready_marker,
        Some(start_marker),
        control_fd,
    )
}

fn fill_shell_init_template(
    template: &str,
    ready_marker: &str,
    start_marker: Option<&str>,
    control_fd: i32,
) -> String {
    template
        .replace("@READY_MARKER@", ready_marker)
        .replace("@START_MARKER@", start_marker.unwrap_or(""))
        .replace("@CONTROL_FD@", &control_fd.to_string())
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

fn fish_supports_private_mode(program: &str) -> bool {
    ProcessCommand::new(program)
        .args(["--private", "-c", "true"])
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
    if launch.integration == ShellIntegration::BashPromptCommand {
        command.env("HISTCONTROL", "ignorespace");
        command.env("HISTIGNORE", "*");
    }
    if launch.integration == ShellIntegration::FishEvents {
        command.env("fish_history", "");
    }
    command.env_remove("AISH_CONTROL_FD");
    command
}
