use std::ffi::OsStr;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::completion::{
    CompletionCandidate, CompletionSource, current_token_context, shell_like_words,
};

const PROCESS_TIMEOUT: Duration = Duration::from_millis(1_200);
const ZSH_INIT_TIMEOUT: Duration = Duration::from_millis(2_000);
const ZSH_LIST_TIMEOUT: Duration = Duration::from_millis(800);
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
    if !completion_request_is_supported(request) {
        return Vec::new();
    }
    #[cfg(test)]
    if let Some(candidates) = test_backend_candidates(&request.shell) {
        return normalize_backend_candidates(request, candidates);
    }
    let candidates = match shell_name(&request.shell).as_str() {
        "fish" => complete_fish(request),
        "bash" => complete_bash(request),
        "zsh" => complete_zsh(request),
        _ => Ok(Vec::new()),
    }
    .unwrap_or_default();
    normalize_backend_candidates(request, candidates)
}

#[cfg(test)]
fn test_backend_candidates(shell: &str) -> Option<Vec<String>> {
    shell.strip_prefix("aish-test-backend:").map(|body| {
        body.split(',')
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn completion_request_is_supported(request: &ShellCompletionRequest) -> bool {
    request.cursor <= request.line.len()
        && !request.line.trim().is_empty()
        && !request.line.starts_with('#')
        && !request.line.contains(['\n', '\r', '\t'])
}

fn complete_fish(request: &ShellCompletionRequest) -> Result<Vec<String>> {
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
    let output = run_process_with_timeout(&mut command, PROCESS_TIMEOUT)?;
    Ok(parse_marked_lines(
        &output,
        BASH_BEGIN_MARKER,
        BASH_END_MARKER,
    ))
}

fn complete_bash(request: &ShellCompletionRequest) -> Result<Vec<String>> {
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
    let output = run_process_with_timeout(&mut command, PROCESS_TIMEOUT)?;
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

fn complete_zsh(request: &ShellCompletionRequest) -> Result<Vec<String>> {
    complete_zsh_with_pty(request)
}

#[cfg(unix)]
fn complete_zsh_with_pty(request: &ShellCompletionRequest) -> Result<Vec<String>> {
    let marker = format!("__AISH_ZSH_COMPLETION_DONE_{}__", std::process::id());
    let mut command = Command::new(&request.shell);
    command
        .arg("-i")
        .current_dir(&request.cwd)
        .envs(request.env.iter().map(|(key, value)| (key, value)))
        .env("AISH_ZSH_COMPLETION_MARKER", &marker);
    let mut pty = QueryPty::spawn(command)?;
    let _ = pty.read_for(Duration::from_millis(250));
    let init = r#"autoload -Uz compinit
compinit -D 2>/dev/null || true
PROMPT=''
RPROMPT=''
unsetopt BEEP 2>/dev/null || true
setopt AUTO_LIST
zstyle ':completion:*' verbose no
zstyle ':completion:*' format ''
zstyle ':completion:*' group-name ''
zstyle ':completion:*' list-colors ''
bindkey '^I' list-choices
print -r -- $AISH_ZSH_COMPLETION_MARKER
"#;
    pty.write_all(init.as_bytes())?;
    let _ = pty.read_until(marker.as_bytes(), ZSH_INIT_TIMEOUT)?;
    let _ = pty.read_for(Duration::from_millis(50));
    pty.write_all(line_prefix_at_cursor(&request.line, request.cursor).as_bytes())?;
    pty.write_all(b"\t")?;
    let raw = pty.read_for(ZSH_LIST_TIMEOUT)?;
    Ok(parse_zsh_list_output(request, &raw))
}

#[cfg(not(unix))]
fn complete_zsh_with_pty(_request: &ShellCompletionRequest) -> Result<Vec<String>> {
    Ok(Vec::new())
}

#[cfg(unix)]
struct QueryPty {
    master: std::fs::File,
    child: std::process::Child,
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
        Ok(Self { master, child })
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.master.write_all(bytes)?;
        self.master.flush()?;
        Ok(())
    }

    fn read_until(&mut self, needle: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut data = Vec::new();
        while Instant::now() < deadline {
            data.extend(self.read_available(Duration::from_millis(20))?);
            if data.windows(needle.len()).any(|window| window == needle) {
                return Ok(data);
            }
        }
        Ok(data)
    }

    fn read_for(&mut self, duration: Duration) -> Result<Vec<u8>> {
        let deadline = Instant::now() + duration;
        let mut data = Vec::new();
        while Instant::now() < deadline {
            data.extend(self.read_available(Duration::from_millis(20))?);
        }
        Ok(data)
    }

    fn read_available(&mut self, timeout: Duration) -> Result<Vec<u8>> {
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
        let _ = self.write_all(b"\x03exit\n");
        let _ = self.child.kill();
        let _ = self.child.wait();
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

fn run_process_with_timeout(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
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
        if Instant::now() >= deadline {
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
    let text = clean_terminal_output(output);
    let mut candidates = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line == query_line || line.ends_with(query_line) {
            continue;
        }
        let before_description = line.split(" -- ").next().unwrap_or(line);
        for part in before_description.split_whitespace() {
            if let Some(candidate) = normalize_candidate_for_token(part, &token.text) {
                candidates.push(candidate);
            }
        }
    }
    candidates
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
}
