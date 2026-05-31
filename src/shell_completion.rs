use std::ffi::OsStr;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::completion::{
    CompletionCandidate, CompletionSource, current_token_context, shell_like_words,
};

const PROCESS_TIMEOUT: Duration = Duration::from_millis(1_200);
const ZSH_INIT_TIMEOUT: Duration = Duration::from_millis(2_000);
const ZSH_LIST_TIMEOUT: Duration = Duration::from_millis(1_200);
const ZSH_LIST_MIN_READ: Duration = Duration::from_millis(120);
const ZSH_LIST_IDLE: Duration = Duration::from_millis(40);
const MAX_BACKEND_CANDIDATES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCompletionRequest {
    pub shell: String,
    pub line: String,
    pub cursor: usize,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

pub fn complete_backend_shell(request: &ShellCompletionRequest) -> Vec<CompletionCandidate> {
    complete_backend_shell_with_cancel(request, || false)
}

pub fn complete_backend_shell_with_cancel<F>(
    request: &ShellCompletionRequest,
    is_cancelled: F,
) -> Vec<CompletionCandidate>
where
    F: Fn() -> bool,
{
    if !completion_request_is_supported(request) {
        return Vec::new();
    }
    if is_cancelled() {
        return Vec::new();
    }
    #[cfg(test)]
    if let Some(candidates) = test_backend_candidates(&request.shell, &is_cancelled) {
        if is_cancelled() {
            return Vec::new();
        }
        return normalize_backend_candidates(request, candidates);
    }
    let candidates = match shell_name(&request.shell).as_str() {
        "fish" => complete_fish(request, &is_cancelled),
        "bash" => complete_bash(request, &is_cancelled),
        "zsh" => complete_zsh(request, &is_cancelled),
        _ => Ok(Vec::new()),
    };
    let candidates = candidates.unwrap_or_default();
    if is_cancelled() {
        return Vec::new();
    }
    normalize_backend_candidates(request, candidates)
}

#[cfg(test)]
fn test_backend_candidates<F>(shell: &str, is_cancelled: &F) -> Option<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    if let Some(rest) = shell.strip_prefix("aish-test-backend-delay-ms:")
        && let Some((delay_ms, body)) = rest.split_once(':')
    {
        let delay_ms: u64 = delay_ms.parse().unwrap_or(0);
        for _ in 0..delay_ms.div_ceil(10) {
            if is_cancelled() {
                return Some(Vec::new());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        return Some(parse_test_backend_candidates(body));
    }
    shell
        .strip_prefix("aish-test-backend:")
        .map(|body| parse_test_backend_candidates(body))
}

#[cfg(test)]
fn parse_test_backend_candidates(body: &str) -> Vec<String> {
    body.split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn completion_request_is_supported(request: &ShellCompletionRequest) -> bool {
    request.cursor <= request.line.len()
        && !request.line.trim().is_empty()
        && !request.line.starts_with('#')
        && !request.line.contains(['\n', '\r', '\t'])
}

fn complete_fish<F>(request: &ShellCompletionRequest, is_cancelled: &F) -> Result<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    let query_line = line_prefix_at_cursor(&request.line, request.cursor);
    let mut command = Command::new(&request.shell);
    command
        .arg("-ic")
        .arg(
            r#"printf '%s\n' __AISH_BACKEND_COMPLETION_BEGIN__
set -l __aish_count 0
for __aish_candidate in (complete -C "$argv[1]" 2>/dev/null)
    printf '%s\n' $__aish_candidate
    set __aish_count (math $__aish_count + 1)
    if test $__aish_count -ge 100
        break
    end
end
printf '%s\n' __AISH_BACKEND_COMPLETION_END__"#,
        )
        .arg(query_line)
        .current_dir(&request.cwd)
        .envs(request.env.iter().map(|(key, value)| (key, value)));
    let output = run_process_with_timeout(&mut command, PROCESS_TIMEOUT, is_cancelled)?;
    Ok(parse_marked_lines(
        &output,
        BASH_BEGIN_MARKER,
        BASH_END_MARKER,
    ))
}

fn complete_bash<F>(request: &ShellCompletionRequest, is_cancelled: &F) -> Result<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    let words = bash_words_for_completion(&request.line, request.cursor);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let cword = words.len().saturating_sub(1).to_string();
    let mut command = Command::new(&request.shell);
    command
        .arg("-ic")
        .arg(BASH_COMPLETION_SCRIPT)
        .arg("aish-complete")
        .arg(&request.line)
        .arg(request.cursor.to_string())
        .arg(cword)
        .args(words)
        .current_dir(&request.cwd)
        .envs(request.env.iter().map(|(key, value)| (key, value)))
        .stderr(Stdio::null());
    let output = run_process_with_timeout(&mut command, PROCESS_TIMEOUT, is_cancelled)?;
    Ok(parse_marked_lines(
        &output,
        BASH_BEGIN_MARKER,
        BASH_END_MARKER,
    ))
}

const BASH_BEGIN_MARKER: &str = "__AISH_BACKEND_COMPLETION_BEGIN__";
const BASH_END_MARKER: &str = "__AISH_BACKEND_COMPLETION_END__";

const BASH_COMPLETION_SCRIPT: &str = r#"
__aish_line=$1
__aish_point=$2
__aish_cword=$3
shift 3
COMP_LINE=$__aish_line
COMP_POINT=$__aish_point
COMP_WORDS=("$@")
COMP_CWORD=$__aish_cword
COMP_TYPE=9
COMP_KEY=9
__aish_cur=${COMP_WORDS[$COMP_CWORD]}
__aish_prev=
if (( COMP_CWORD > 0 )); then
  __aish_prev=${COMP_WORDS[$((COMP_CWORD - 1))]}
fi
__aish_cmd=${COMP_WORDS[0]}
__aish_candidates=()
__aish_add_lines() {
  local __aish_line
  while IFS= read -r __aish_line; do
    [[ -n $__aish_line ]] && __aish_candidates+=("$__aish_line")
  done
}
__aish_run_compspec() {
  local __aish_spec=$1
  [[ -z $__aish_spec ]] && return 1
  local -a __aish_args
  eval "__aish_args=(${__aish_spec#complete })"
  local __aish_i __aish_func= __aish_cmdspec= __aish_wordlist= __aish_glob=
  local __aish_filter= __aish_prefix= __aish_suffix=
  local -a __aish_actions
  local __aish_default=0 __aish_bashdefault=0 __aish_dirnames=0 __aish_filenames=0
  for ((__aish_i = 0; __aish_i < ${#__aish_args[@]}; __aish_i++)); do
    case ${__aish_args[$__aish_i]} in
      -F) ((__aish_i++)); __aish_func=${__aish_args[$__aish_i]} ;;
      -C) ((__aish_i++)); __aish_cmdspec=${__aish_args[$__aish_i]} ;;
      -W) ((__aish_i++)); __aish_wordlist=${__aish_args[$__aish_i]} ;;
      -G) ((__aish_i++)); __aish_glob=${__aish_args[$__aish_i]} ;;
      -A) ((__aish_i++)); __aish_actions+=("${__aish_args[$__aish_i]}") ;;
      -X) ((__aish_i++)); __aish_filter=${__aish_args[$__aish_i]} ;;
      -P) ((__aish_i++)); __aish_prefix=${__aish_args[$__aish_i]} ;;
      -S) ((__aish_i++)); __aish_suffix=${__aish_args[$__aish_i]} ;;
      -[abcdefgjksuv]*)
        local __aish_letters=${__aish_args[$__aish_i]#-}
        local __aish_j __aish_letter
        for ((__aish_j = 0; __aish_j < ${#__aish_letters}; __aish_j++)); do
          __aish_letter=${__aish_letters:$__aish_j:1}
          case $__aish_letter in
            a) __aish_actions+=(alias) ;;
            b) __aish_actions+=(builtin) ;;
            c) __aish_actions+=(command) ;;
            d) __aish_actions+=(directory) ;;
            e) __aish_actions+=(export) ;;
            f) __aish_actions+=(file) ;;
            g) __aish_actions+=(group) ;;
            j) __aish_actions+=(job) ;;
            k) __aish_actions+=(keyword) ;;
            s) __aish_actions+=(service) ;;
            u) __aish_actions+=(user) ;;
            v) __aish_actions+=(variable) ;;
          esac
        done
        ;;
      -o)
        ((__aish_i++))
        case ${__aish_args[$__aish_i]} in
          default) __aish_default=1 ;;
          bashdefault) __aish_bashdefault=1 ;;
          dirnames) __aish_dirnames=1 ;;
          filenames) __aish_filenames=1 ;;
        esac
        ;;
      --) break ;;
    esac
  done
  if [[ -n $__aish_func ]] && declare -F "$__aish_func" >/dev/null 2>&1; then
    COMPREPLY=()
    "$__aish_func" "$__aish_cmd" "$__aish_cur" "$__aish_prev" >/dev/null 2>&1 || true
    __aish_candidates+=("${COMPREPLY[@]}")
  fi
  if [[ -n $__aish_cmdspec ]]; then
    __aish_add_lines < <($__aish_cmdspec "$__aish_cmd" "$__aish_cur" "$__aish_prev" 2>/dev/null || true)
  fi
  if [[ -n $__aish_wordlist ]]; then
    __aish_add_lines < <(compgen -W "$__aish_wordlist" -- "$__aish_cur")
  fi
  if [[ -n $__aish_glob ]]; then
    __aish_add_lines < <(compgen -G "$__aish_glob" -- "$__aish_cur")
  fi
  local __aish_action
  for __aish_action in "${__aish_actions[@]}"; do
    __aish_add_lines < <(compgen -A "$__aish_action" -- "$__aish_cur")
  done
  if [[ ${#__aish_candidates[@]} -eq 0 ]]; then
    if (( __aish_dirnames )); then
      __aish_add_lines < <(compgen -d -- "$__aish_cur")
    fi
    if (( __aish_filenames || __aish_default || __aish_bashdefault )); then
      __aish_add_lines < <(compgen -f -- "$__aish_cur")
    fi
  fi
  if [[ -n $__aish_filter ]]; then
    local -a __aish_filtered
    local __aish_candidate __aish_pattern=$__aish_filter __aish_negate=0
    if [[ $__aish_pattern == '!'* ]]; then
      __aish_negate=1
      __aish_pattern=${__aish_pattern:1}
    fi
    __aish_pattern=${__aish_pattern//&/$__aish_cur}
    for __aish_candidate in "${__aish_candidates[@]}"; do
      if [[ $__aish_candidate == $__aish_pattern ]]; then
        (( __aish_negate )) && __aish_filtered+=("$__aish_candidate")
      else
        (( __aish_negate )) || __aish_filtered+=("$__aish_candidate")
      fi
    done
    __aish_candidates=("${__aish_filtered[@]}")
  fi
  if [[ -n $__aish_prefix || -n $__aish_suffix ]]; then
    local -a __aish_decorated
    local __aish_candidate
    for __aish_candidate in "${__aish_candidates[@]}"; do
      __aish_decorated+=("$__aish_prefix$__aish_candidate$__aish_suffix")
    done
    __aish_candidates=("${__aish_decorated[@]}")
  fi
}
__aish_spec=$(complete -p -- "$__aish_cmd" 2>/dev/null || true)
if [[ -z $__aish_spec ]]; then
  __aish_spec=$(complete -p -D 2>/dev/null || true)
fi
if [[ -n $__aish_spec ]]; then
  __aish_run_compspec "$__aish_spec"
elif (( COMP_CWORD == 0 )); then
  __aish_add_lines < <(compgen -A command -- "$__aish_cur")
else
  __aish_add_lines < <(compgen -f -- "$__aish_cur")
fi
printf '%s\n' __AISH_BACKEND_COMPLETION_BEGIN__
__aish_printed=0
for __aish_candidate in "${__aish_candidates[@]}"; do
  [[ -n $__aish_candidate ]] || continue
  printf '%s\n' "$__aish_candidate"
  ((__aish_printed++))
  (( __aish_printed >= 100 )) && break
done
printf '%s\n' __AISH_BACKEND_COMPLETION_END__
"#;

fn complete_zsh<F>(request: &ShellCompletionRequest, is_cancelled: &F) -> Result<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    complete_zsh_with_pty(request, is_cancelled)
}

fn zsh_completion_command(request: &ShellCompletionRequest) -> Option<String> {
    shell_like_words(line_prefix_at_cursor(&request.line, request.cursor))
        .into_iter()
        .next()
        .map(|word| word.value)
        .filter(|word| !word.is_empty())
}

#[cfg(unix)]
fn complete_zsh_with_pty<F>(
    request: &ShellCompletionRequest,
    is_cancelled: &F,
) -> Result<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    if is_cancelled() {
        return Ok(Vec::new());
    }
    let marker = format!("__AISH_ZSH_COMPLETION_DONE_{}__", std::process::id());
    let mut command = Command::new(&request.shell);
    command
        .arg("-f")
        .arg("-i")
        .current_dir(&request.cwd)
        .envs(request.env.iter().map(|(key, value)| (key, value)))
        .env("AISH_ZSH_COMPLETION_MARKER", &marker)
        .env(
            "AISH_ZSH_COMPLETION_COMMAND",
            zsh_completion_command(request).unwrap_or_default(),
        );
    let mut pty = QueryPty::spawn(command)?;
    let _ = pty.read_for(Duration::from_millis(20), is_cancelled);
    let init = r#"typeset -U fpath
typeset -a __aish_completion_dirs
if [[ -n ${ZDOTDIR:-} && -d "$ZDOTDIR/completions" ]]; then
  __aish_completion_dirs+=("$ZDOTDIR/completions")
fi
if [[ -n ${ZDOTDIR:-} && -d "$ZDOTDIR/.zsh/completions" ]]; then
  __aish_completion_dirs+=("$ZDOTDIR/.zsh/completions")
fi
if [[ -n ${HOME:-} && -d "$HOME/.zsh/completions" ]]; then
  __aish_completion_dirs+=("$HOME/.zsh/completions")
fi
if (( ${#__aish_completion_dirs} )); then
  fpath=("${__aish_completion_dirs[@]}" $fpath)
fi
__aish_source_zshrc() {
  source "$1"
}
__aish_zdotdir=${ZDOTDIR:-${HOME:-}}
if [[ -n $__aish_zdotdir && -r "$__aish_zdotdir/.zshrc" ]]; then
  __aish_source_zshrc "$__aish_zdotdir/.zshrc"
fi
typeset -U fpath
__aish_completion_dirs=()
if [[ -n ${ZDOTDIR:-} && -d "$ZDOTDIR/completions" ]]; then
  __aish_completion_dirs+=("$ZDOTDIR/completions")
fi
if [[ -n ${ZDOTDIR:-} && -d "$ZDOTDIR/.zsh/completions" ]]; then
  __aish_completion_dirs+=("$ZDOTDIR/.zsh/completions")
fi
if [[ -n ${HOME:-} && -d "$HOME/.zsh/completions" ]]; then
  __aish_completion_dirs+=("$HOME/.zsh/completions")
fi
if (( ${#__aish_completion_dirs} )); then
  fpath=("${__aish_completion_dirs[@]}" $fpath)
fi
typeset -A __aish_saved_comps
if (( $+parameters[_comps] )); then
  __aish_saved_comps=("${(@kv)_comps}")
fi
if ! (( $+functions[compdef] && $+parameters[_comps] )); then
  autoload -Uz compinit
  compinit -u -C -D 2>/dev/null || true
  if ! (( $+functions[compdef] && $+parameters[_comps] )); then
    compinit -u -D 2>/dev/null || compinit -i -D 2>/dev/null || true
  fi
  if (( ${#__aish_saved_comps} && $+parameters[_comps] )); then
    for __aish_saved_key in "${(@k)__aish_saved_comps}"; do
      _comps[$__aish_saved_key]=${__aish_saved_comps[$__aish_saved_key]}
    done
  fi
fi
if (( ${#__aish_completion_dirs} && $+functions[compdef] )); then
  __aish_completion_command=${AISH_ZSH_COMPLETION_COMMAND:-}
  if [[ -n $__aish_completion_command ]]; then
    __aish_completion_function="_${__aish_completion_command:t}"
    for __aish_completion_dir in "${__aish_completion_dirs[@]}"; do
      if [[ -r "$__aish_completion_dir/$__aish_completion_function" ]]; then
        autoload -Uz "$__aish_completion_function"
        compdef "$__aish_completion_function" "$__aish_completion_command"
        break
      fi
    done
  fi
fi
PROMPT=''
RPROMPT=''
unsetopt BEEP 2>/dev/null || true
setopt AUTO_LIST
zstyle ':completion:*' verbose no
zstyle ':completion:*' format ''
zstyle ':completion:*' group-name ''
zstyle ':completion:*' list-colors ''
bindkey '^I' list-choices
rehash 2>/dev/null || true
print -r -- $AISH_ZSH_COMPLETION_MARKER
"#;
    let _init_file = source_zsh_init_script(&mut pty, init)?;
    let _ = pty.read_until(marker.as_bytes(), ZSH_INIT_TIMEOUT, is_cancelled)?;
    let _ = pty.read_for(Duration::from_millis(50), is_cancelled);
    if is_cancelled() {
        return Ok(Vec::new());
    }
    pty.write_all(b"\x15")?;
    pty.write_all(line_prefix_at_cursor(&request.line, request.cursor).as_bytes())?;
    pty.write_all(b"\t")?;
    let raw = pty.read_until_quiet(
        ZSH_LIST_TIMEOUT,
        ZSH_LIST_MIN_READ,
        ZSH_LIST_IDLE,
        is_cancelled,
    )?;
    if is_cancelled() {
        return Ok(Vec::new());
    }
    Ok(parse_zsh_list_output(request, &raw))
}

#[cfg(unix)]
fn source_zsh_init_script(pty: &mut QueryPty, init: &str) -> Result<ZshInitScript> {
    let script = ZshInitScript::create()?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&script.path)
        .context("failed to open zsh completion init script")?;
    file.write_all(init.as_bytes())
        .context("failed to write zsh completion init script")?;
    file.flush()
        .context("failed to flush zsh completion init script")?;
    let command = format!(
        "source {}\n",
        zsh_single_quote(&script.path.to_string_lossy())
    );
    pty.write_all(command.as_bytes())?;
    Ok(script)
}

#[cfg(unix)]
struct ZshInitScript {
    path: PathBuf,
}

#[cfg(unix)]
impl ZshInitScript {
    fn create() -> Result<Self> {
        let base = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..100_u32 {
            let path = base.join(format!(
                "aish-zsh-completion-{}-{unique}-{attempt}.zsh",
                std::process::id()
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).context("failed to create zsh completion init script");
                }
            }
        }
        anyhow::bail!("failed to create unique zsh completion init script")
    }
}

#[cfg(unix)]
impl Drop for ZshInitScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn zsh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(not(unix))]
fn complete_zsh_with_pty<F>(
    _request: &ShellCompletionRequest,
    _is_cancelled: &F,
) -> Result<Vec<String>>
where
    F: Fn() -> bool + ?Sized,
{
    Ok(Vec::new())
}

#[cfg(unix)]
struct QueryPty {
    master: std::fs::File,
    child: std::process::Child,
    pending: Vec<u8>,
}

#[cfg(unix)]
impl QueryPty {
    fn spawn(mut command: Command) -> Result<Self> {
        let (master, slave) = openpty()?;
        set_nonblocking(master.as_raw_fd())?;
        let stdin = slave.try_clone().context("failed to clone PTY stdin")?;
        let stdout = slave.try_clone().context("failed to clone PTY stdout")?;
        let stderr = slave.try_clone().context("failed to clone PTY stderr")?;
        command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .spawn()
            .context("failed to spawn zsh completion helper")?;
        drop(slave);
        Ok(Self {
            master,
            child,
            pending: Vec::new(),
        })
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut written = 0;
        while written < bytes.len() {
            match self.master.write(&bytes[written..]) {
                Ok(0) => anyhow::bail!("failed to write to zsh completion PTY: wrote zero bytes"),
                Ok(n) => written += n,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        anyhow::bail!("timed out writing to zsh completion PTY");
                    }
                    self.buffer_available(Duration::from_millis(10))?;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    fn buffer_available(&mut self, timeout: Duration) -> Result<()> {
        let data = self.read_available_from_fd(timeout)?;
        self.pending.extend(data);
        Ok(())
    }

    fn read_until<F>(
        &mut self,
        needle: &[u8],
        timeout: Duration,
        is_cancelled: &F,
    ) -> Result<Vec<u8>>
    where
        F: Fn() -> bool + ?Sized,
    {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        while Instant::now() < deadline && !is_cancelled() {
            data.extend(self.read_available(Duration::from_millis(20))?);
            if data.windows(needle.len()).any(|window| window == needle) {
                return Ok(data);
            }
        }
        Ok(data)
    }

    fn read_for<F>(&mut self, duration: Duration, is_cancelled: &F) -> Result<Vec<u8>>
    where
        F: Fn() -> bool + ?Sized,
    {
        let deadline = Instant::now() + duration;
        let mut data = Vec::new();
        while Instant::now() < deadline && !is_cancelled() {
            data.extend(self.read_available(Duration::from_millis(20))?);
        }
        Ok(data)
    }

    fn read_until_quiet<F>(
        &mut self,
        timeout: Duration,
        min_duration: Duration,
        quiet_duration: Duration,
        is_cancelled: &F,
    ) -> Result<Vec<u8>>
    where
        F: Fn() -> bool + ?Sized,
    {
        let started = Instant::now();
        let deadline = started + timeout;
        let min_deadline = started + min_duration;
        let mut last_output = started;
        let mut data = Vec::new();
        while Instant::now() < deadline && !is_cancelled() {
            let chunk = self.read_available(Duration::from_millis(10))?;
            let now = Instant::now();
            if !chunk.is_empty() {
                data.extend(chunk);
                last_output = now;
            }
            if !data.is_empty()
                && now >= min_deadline
                && now.saturating_duration_since(last_output) >= quiet_duration
            {
                break;
            }
        }
        Ok(data)
    }

    fn read_available(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let mut data = std::mem::take(&mut self.pending);
        data.extend(self.read_available_from_fd(timeout)?);
        Ok(data)
    }

    fn read_available_from_fd(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let mut fds = [libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        }];
        let ready = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                1,
                timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int,
            )
        };
        if ready <= 0 {
            return Ok(Vec::new());
        }
        let mut data = Vec::new();
        loop {
            let mut buf = [0_u8; 4096];
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    break;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(data)
    }
}

#[cfg(unix)]
impl Drop for QueryPty {
    fn drop(&mut self) {
        let pgid = self.child.id() as libc::pid_t;
        let _ = self.write_all(b"\x03exit\n");
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGTERM);
        }
        wait_for_query_child_exit(&mut self.child, Duration::from_millis(100));
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGKILL);
        }
        let _ = self.child.kill();
        wait_for_query_child_exit(&mut self.child, Duration::from_millis(100));
    }
}

#[cfg(unix)]
fn wait_for_query_child_exit(child: &mut std::process::Child, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[cfg(unix)]
fn openpty() -> Result<(std::fs::File, std::fs::File)> {
    let mut master = -1;
    let mut slave = -1;
    let mut winsize = libc::winsize {
        ws_row: 24,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let status = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if status != 0 {
        anyhow::bail!("failed to open PTY: {}", std::io::Error::last_os_error());
    }
    let master = unsafe { std::fs::File::from_raw_fd(master) };
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    Ok((master, slave))
}

#[cfg(unix)]
fn set_nonblocking(fd: RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn run_process_with_timeout<F>(
    command: &mut Command,
    timeout: Duration,
    is_cancelled: &F,
) -> Result<Vec<u8>>
where
    F: Fn() -> bool + ?Sized,
{
    command.stdout(Stdio::piped());
    let mut child = command
        .spawn()
        .context("failed to spawn completion helper")?;
    let stdout_reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stdout.read_to_end(&mut output);
            output
        })
    });
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(join_stdout_reader(stdout_reader));
        }
        if is_cancelled() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_stdout_reader(stdout_reader);
            return Ok(Vec::new());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn join_stdout_reader(reader: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn bash_words_for_completion(line: &str, cursor: usize) -> Vec<String> {
    let before_cursor = line_prefix_at_cursor(line, cursor);
    let mut words = shell_like_words(before_cursor)
        .into_iter()
        .map(|word| word.value)
        .collect::<Vec<_>>();
    if before_cursor
        .chars()
        .last()
        .is_some_and(char::is_whitespace)
    {
        words.push(String::new());
    }
    words
}

fn normalize_backend_candidates(
    request: &ShellCompletionRequest,
    candidates: Vec<String>,
) -> Vec<CompletionCandidate> {
    let token = current_token_context(&request.line, request.cursor);
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for candidate in candidates {
        let candidate = trim_candidate_description(&candidate);
        let candidate = normalize_backend_candidate(&candidate, &token.text);
        if candidate.is_empty() || candidate == token.text || !seen.insert(candidate.clone()) {
            continue;
        }
        let is_dir = candidate.ends_with('/');
        normalized.push(CompletionCandidate {
            display: candidate.clone(),
            replacement: candidate,
            is_dir,
            source: CompletionSource::BackendShell,
        });
        if normalized.len() >= MAX_BACKEND_CANDIDATES {
            break;
        }
    }
    normalized
}

fn normalize_backend_candidate(candidate: &str, token: &str) -> String {
    normalize_candidate_for_token(candidate, token).unwrap_or_else(|| candidate.to_string())
}

fn normalize_candidate_for_token(candidate: &str, token: &str) -> Option<String> {
    if token.is_empty() || candidate.starts_with(token) {
        return Some(candidate.to_string());
    }
    let (dir, prefix) = token.rsplit_once('/')?;
    if candidate.starts_with(&format!("{dir}/")) {
        return Some(candidate.to_string());
    }
    if candidate.starts_with(prefix) {
        return Some(format!("{dir}/{candidate}"));
    }
    None
}

fn trim_candidate_description(candidate: &str) -> String {
    let candidate = candidate.split('\t').next().unwrap_or(candidate);
    let candidate = candidate.split(" -- ").next().unwrap_or(candidate);
    candidate.trim().to_string()
}

fn parse_marked_lines(output: &[u8], begin: &str, end: &str) -> Vec<String> {
    let text = String::from_utf8_lossy(output);
    let Some((_, after_begin)) = text.split_once(begin) else {
        return Vec::new();
    };
    let candidates = after_begin
        .split_once(end)
        .map_or(after_begin, |(body, _)| body);
    candidates
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_zsh_list_output(request: &ShellCompletionRequest, output: &[u8]) -> Vec<String> {
    let token = current_token_context(&request.line, request.cursor);
    let query_line = line_prefix_at_cursor(&request.line, request.cursor);
    let query_words: Vec<String> = shell_like_words(query_line)
        .into_iter()
        .map(|word| word.value)
        .collect();
    let text = clean_terminal_output(output);
    let mut candidates = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.contains("__AISH_ZSH_COMPLETION_DONE_") {
            continue;
        }
        if line == query_line || line.ends_with(query_line) {
            continue;
        }
        let before_description = line.split(" -- ").next().unwrap_or(line);
        for part in before_description.split_whitespace() {
            if zsh_output_part_is_query_echo(part, &query_words) {
                continue;
            }
            if let Some(candidate) = normalize_candidate_for_token(part, &token.text) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn zsh_output_part_is_query_echo(part: &str, query_words: &[String]) -> bool {
    query_words
        .iter()
        .any(|word| part == word || (!part.is_empty() && word.starts_with(part)))
}

fn clean_terminal_output(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut clean = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\x1b' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
            }
            '\x08' => {
                clean.pop();
            }
            ch if ch == '\n' || ch == '\t' || !ch.is_control() => clean.push(ch),
            _ => {}
        }
    }
    clean
}

fn shell_name(program: &str) -> String {
    let name = Path::new(program.trim())
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .trim_start_matches('-')
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

fn line_prefix_at_cursor(line: &str, cursor: usize) -> &str {
    let mut cursor = cursor.min(line.len());
    while !line.is_char_boundary(cursor) {
        cursor -= 1;
    }
    &line[..cursor]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(shell: &str, line: &str) -> ShellCompletionRequest {
        ShellCompletionRequest {
            shell: shell.to_string(),
            line: line.to_string(),
            cursor: line.len(),
            cwd: std::env::current_dir().unwrap(),
            env: Vec::new(),
        }
    }

    #[test]
    fn backend_candidates_are_deduped_and_labeled() {
        let candidates = normalize_backend_candidates(
            &request("/bin/fish", "git st"),
            vec![
                "status\tShow status".to_string(),
                "status\tDuplicate".to_string(),
                "stash\tStash".to_string(),
                "branch\tBranch".to_string(),
            ],
        );

        assert_eq!(
            candidates,
            [
                CompletionCandidate {
                    display: "status".to_string(),
                    replacement: "status".to_string(),
                    is_dir: false,
                    source: CompletionSource::BackendShell,
                },
                CompletionCandidate {
                    display: "stash".to_string(),
                    replacement: "stash".to_string(),
                    is_dir: false,
                    source: CompletionSource::BackendShell,
                },
                CompletionCandidate {
                    display: "branch".to_string(),
                    replacement: "branch".to_string(),
                    is_dir: false,
                    source: CompletionSource::BackendShell,
                },
            ]
        );
    }

    #[test]
    fn backend_candidates_preserve_typed_path_directory_prefix() {
        let candidates = normalize_backend_candidates(
            &request("/bin/zsh", "cat ./s"),
            vec!["src/".to_string(), "./same-prefix".to_string()],
        );

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.replacement.as_str())
                .collect::<Vec<_>>(),
            ["./src/", "./same-prefix"]
        );
    }

    #[test]
    fn bash_words_include_empty_trailing_token() {
        assert_eq!(bash_words_for_completion("git ", 4), ["git", ""]);
        assert_eq!(bash_words_for_completion("git st", 6), ["git", "st"]);
    }

    #[test]
    fn zsh_list_parser_extracts_described_rows() {
        let output = b"git st\r\nstash      -- stash changes\r\nstatus     -- show status\r\nstripspace -- filter\r\n";
        assert_eq!(
            parse_zsh_list_output(&request("/bin/zsh", "git st"), output),
            ["stash", "status", "stripspace"]
        );
    }

    #[test]
    fn zsh_list_parser_ignores_query_echo_and_internal_marker() {
        let output = b"kagi \r\nk\r\nkagi\r\nask-page  assistant  auth\r\n__AISH_ZSH_COMPLETION_DONE_123__\r\n";
        assert_eq!(
            parse_zsh_list_output(&request("/bin/zsh", "kagi "), output),
            ["ask-page", "assistant", "auth"]
        );
    }

    #[test]
    fn fish_completion_loads_user_completion_directory_when_available() {
        let Some(fish) = find_shell(&["fish", "/opt/homebrew/bin/fish", "/usr/bin/fish"]) else {
            eprintln!("skipping fish backend completion test: fish not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("fish");
        let completions = config.join("completions");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("foocmd"));
        std::fs::write(
            completions.join("foocmd.fish"),
            "complete -c foocmd -a fishbar\n",
        )
        .unwrap();
        let mut request = request(&fish, "foocmd f");
        request.env = vec![
            (
                "XDG_CONFIG_HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "fishbar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn fish_completion_loads_interactive_config_when_available() {
        let Some(fish) = find_shell(&["fish", "/opt/homebrew/bin/fish", "/usr/bin/fish"]) else {
            eprintln!("skipping fish backend completion test: fish not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("fish");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("foocmd"));
        std::fs::write(
            config.join("config.fish"),
            "complete -c foocmd -a fishconfigbar\n",
        )
        .unwrap();
        let mut request = request(&fish, "foocmd fishc");
        request.env = vec![
            (
                "XDG_CONFIG_HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "fishconfigbar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn bash_completion_loads_interactive_bashrc_when_available() {
        let Some(bash) = find_shell(&["bash", "/bin/bash", "/usr/bin/bash"]) else {
            eprintln!("skipping bash backend completion test: bash not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".bashrc"),
            "_foocmd() { COMPREPLY=(bashbar); }\ncomplete -F _foocmd foocmd\n",
        )
        .unwrap();
        let mut request = request(&bash, "foocmd b");
        request.env = vec![(
            "HOME".to_string(),
            temp.path().to_string_lossy().into_owned(),
        )];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "bashbar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn bash_completion_supports_common_compspec_flags_when_available() {
        let Some(bash) = find_shell(&["bash", "/bin/bash", "/usr/bin/bash"]) else {
            eprintln!("skipping bash backend completion test: bash not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir(temp.path().join("src")).unwrap();
        make_executable(&bin.join("foocmd"));
        std::fs::write(
            temp.path().join(".bashrc"),
            "complete -W 'alpha beta skip' -X skip foocmd\ncomplete -W 'alpha' -P pre- -S = prefixcmd\ncomplete -d dircmd\n",
        )
        .unwrap();

        let mut word_request = request(&bash, "foocmd a");
        word_request.env = vec![
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];
        let word_candidates = complete_backend_shell(&word_request);
        assert!(
            word_candidates
                .iter()
                .any(|candidate| candidate.replacement == "alpha"),
            "{word_candidates:?}"
        );
        assert!(
            !word_candidates
                .iter()
                .any(|candidate| candidate.replacement == "skip"),
            "{word_candidates:?}"
        );

        let mut prefix_request = request(&bash, "prefixcmd a");
        prefix_request.env = vec![
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];
        let prefix_candidates = complete_backend_shell(&prefix_request);
        assert!(
            prefix_candidates
                .iter()
                .any(|candidate| candidate.replacement == "pre-alpha="),
            "{prefix_candidates:?}"
        );

        let mut dir_request = request(&bash, "dircmd s");
        dir_request.cwd = temp.path().to_path_buf();
        dir_request.env = vec![(
            "HOME".to_string(),
            temp.path().to_string_lossy().into_owned(),
        )];
        let dir_candidates = complete_backend_shell(&dir_request);
        assert!(
            dir_candidates
                .iter()
                .any(|candidate| candidate.replacement == "src" || candidate.replacement == "src/"),
            "{dir_candidates:?}"
        );
    }

    #[test]
    fn zsh_completion_loads_fpath_completion_directory_when_available() {
        let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"])
        else {
            eprintln!("skipping zsh backend completion test: zsh not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let completions = temp.path().join(".zsh/completions");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("foocmd"));
        std::fs::write(
            temp.path().join(".zshrc"),
            "fpath=(\"$ZDOTDIR/.zsh/completions\" $fpath)\n",
        )
        .unwrap();
        std::fs::write(
            completions.join("_foocmd"),
            "#compdef foocmd\n_arguments '1:arg:(zshbar)'\n",
        )
        .unwrap();
        let mut request = request(&zsh, "foocmd z");
        request.env = vec![
            (
                "ZDOTDIR".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "zshbar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn zsh_completion_loads_home_completion_directory_without_zshrc_fpath_when_available() {
        let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"])
        else {
            eprintln!("skipping zsh backend completion test: zsh not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let completions = temp.path().join(".zsh/completions");
        let zdotdir = temp.path().join("zdot");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&zdotdir).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("homecmd"));
        std::fs::write(
            completions.join("_homecmd"),
            "#compdef homecmd\n_arguments '1:arg:(homebar)'\n",
        )
        .unwrap();
        let mut request = request(&zsh, "homecmd h");
        request.env = vec![
            (
                "ZDOTDIR".to_string(),
                zdotdir.to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "homebar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn zsh_completion_loads_home_completion_directory_after_user_compinit_when_available() {
        let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"])
        else {
            eprintln!("skipping zsh backend completion test: zsh not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let completions = temp.path().join(".zsh/completions");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("latecmd"));
        std::fs::write(
            temp.path().join(".zshrc"),
            "autoload -Uz compinit\ncompinit -u -D\n",
        )
        .unwrap();
        std::fs::write(
            completions.join("_latecmd"),
            "#compdef latecmd\n_arguments '1:arg:(latebar)'\n",
        )
        .unwrap();
        let mut request = request(&zsh, "latecmd l");
        request.env = vec![
            (
                "ZDOTDIR".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "latebar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn zsh_completion_loads_zdotdir_completion_directory_without_zshrc_fpath_when_available() {
        let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"])
        else {
            eprintln!("skipping zsh backend completion test: zsh not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let zdotdir = temp.path().join("zdot");
        let completions = zdotdir.join("completions");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("zdotcmd"));
        std::fs::write(
            completions.join("_zdotcmd"),
            "#compdef zdotcmd\n_arguments '1:arg:(zdotbar)'\n",
        )
        .unwrap();
        let mut request = request(&zsh, "zdotcmd z");
        request.env = vec![
            (
                "ZDOTDIR".to_string(),
                zdotdir.to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "zdotbar"
            }),
            "{candidates:?}"
        );
    }

    #[test]
    fn zsh_completion_preserves_zshrc_compdef_mappings_after_compinit_when_available() {
        let Some(zsh) = find_shell(&["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"])
        else {
            eprintln!("skipping zsh backend completion test: zsh not found");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let completions = temp.path().join(".zsh/completions");
        let bin = temp.path().join("bin");
        std::fs::create_dir_all(&completions).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        make_executable(&bin.join("manualcmd"));
        std::fs::write(
            temp.path().join(".zshrc"),
            "fpath=(\"$ZDOTDIR/.zsh/completions\" $fpath)\n\
             autoload -Uz compinit\n\
             compinit -u -D\n\
             autoload -Uz _manualcmd\n\
             compdef _manualcmd manualcmd\n",
        )
        .unwrap();
        std::fs::write(
            completions.join("_manualcmd"),
            "_arguments '1:arg:(manualbar)'\n",
        )
        .unwrap();
        let mut request = request(&zsh, "manualcmd m");
        request.env = vec![
            (
                "ZDOTDIR".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            (
                "HOME".to_string(),
                temp.path().to_string_lossy().into_owned(),
            ),
            ("PATH".to_string(), test_path_with_bin(&bin)),
        ];

        let candidates = complete_backend_shell(&request);

        assert!(
            candidates.iter().any(|candidate| {
                candidate.source == CompletionSource::BackendShell
                    && candidate.replacement == "manualbar"
            }),
            "{candidates:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn query_pty_drop_terminates_child_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("child.pid");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("(sleep 30) & printf '%s\\n' \"$!\" > \"$1\"; sleep 30")
            .arg("aish-query-pty-cleanup-test")
            .arg(&pid_file);
        let pty = QueryPty::spawn(command).unwrap();
        let child_pid = wait_for_pid_file(&pid_file);
        assert!(process_exists(child_pid), "child process was not started");

        drop(pty);

        let started = Instant::now();
        while process_exists(child_pid) && started.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !process_exists(child_pid),
            "PTY helper child process {child_pid} survived QueryPty drop"
        );
    }

    fn find_shell(candidates: &[&str]) -> Option<String> {
        for candidate in candidates {
            let path = Path::new(candidate);
            if path.components().count() > 1 && path.exists() {
                return Some(candidate.to_string());
            }
            if path.components().count() == 1
                && let Some(path) = find_on_path(candidate)
            {
                return Some(path.to_string_lossy().into_owned());
            }
        }
        None
    }

    fn find_on_path(name: &str) -> Option<PathBuf> {
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.exists())
    }

    fn test_path_with_bin(bin: &Path) -> String {
        let mut paths = vec![bin.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        std::env::join_paths(paths)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn make_executable(path: &Path) {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
    }

    #[cfg(unix)]
    fn wait_for_pid_file(path: &Path) -> libc::pid_t {
        let started = Instant::now();
        loop {
            if let Ok(contents) = std::fs::read_to_string(path)
                && let Ok(pid) = contents.trim().parse::<libc::pid_t>()
            {
                return pid;
            }
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "timed out waiting for pid file {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    fn process_exists(pid: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
