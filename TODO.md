# Aish TODO

This is the current source of truth for unfinished work. Older phase logs,
completed checklist history, and test-result snapshots live in git history.

Do not add completed work here. When a task is finished, either remove it or
move the result into user-facing documentation if it changes behavior.

## Release-Blocking Validation

- [ ] Promote fish from opt-in validation to default required support only after
  cross-platform evidence is strong enough.
  - Validate fish on macOS in at least one real terminal.
  - Validate fish on a Debian/Ubuntu-family Linux system.
  - Validate fish on a Fedora/openEuler-family Linux system.
  - Run the opt-in fish automation on each platform:
    `AISH_TEST_FISH=1 cargo test --test pty_backend -- --nocapture`,
    `tmux_common_shell_workflow_matches_fish_backend_real_terminal_screen`,
    `tmux_backend_rc_inheritance_matches_fish_real_terminal_screen`, and
    `tmux_rm_write_protected_prompt_waits_for_user_input_fish_backend`.
  - Run the manual fish rc compatibility check from `MANUAL_TESTS.md`.
  - If the results are clean, remove the experimental/opt-in wording from
    `README.md`, `FULL_TESTS.md`, `MANUAL_TESTS.md`, and
    `TESTING_MANUAL.md`, and make fish coverage part of the default expected
    validation path where practical.

- [ ] Complete a fresh manual release sweep on real terminals.
  - Bash rc compatibility with aliases, functions, `PATH`, `PROMPT_COMMAND`,
    `PS0`, `clear`, and plain `exit`.
  - Zsh rc compatibility with aliases, functions, direct `preexec`/`precmd`,
    hook arrays, `clear`, and plain `exit`.
  - Nested shell foreground behavior: Aish -> bash/zsh/fish -> another shell,
    including `clear`, `Ctrl-C`, and clean layer-by-layer exit.
  - TTY stdin prompts: `cat`, `grep`, `sed`, `awk`, and write-protected `rm`
    prompt visibility before the answer is typed.
  - Full-screen and alternate-screen programs: `vim`/`nvim`, `less`, `top`,
    real `fzf`, and nested `tmux`/`screen` where available.
  - Network-adjacent passthrough with safe targets: invalid or disposable `ssh`
    and real remote auth prompts only when the tester intentionally provides
    non-production credentials.
  - Terminal resize, Unicode input, and long command editing in real terminal
    emulators.

- [ ] Keep real passphrase/pinentry behavior validated without pretending it is
  deterministic CI coverage.
  - Use isolated `GNUPGHOME`, disposable keys, and isolated `AISH_HOME`.
  - Cover lazy `#unlock`, `#encrypt unlock-mode prompt`, `#key set`, optional
    key rotation, `#encrypt off`, and direct `gpg` passthrough.
  - Add focused automated regressions only for failures that can be reproduced
    without real pinentry UI or personal secrets.

- [ ] Keep real provider and real remote behavior validated manually.
  - Test a disposable OpenAI-compatible endpoint and key before claiming live
    provider compatibility.
  - Test SSH/HTTPS sync auth only with non-production remotes.
  - Treat missing GitHub credentials as a skipped optional check, not a product
    failure.

## Template Sharing

- [ ] Extend template sharing review workflows after the first static remote
  flow is stable.
  - Add richer diff/preview commands for fetched templates before import.
  - Add trust/signature guidance for public template remotes if sharing expands
    beyond known collaborators.
  - Keep template sharing independent from private sync; do not mix private
    history, drafts, AI history, notes, config, logs, cache, or secrets into
    template remotes.

## PTY And Passthrough Hardening

- [ ] Add focused regressions for any newly reported interactive, raw-mode,
  alternate-screen, or job-control program that exposes a real bug.
  - Prefer tmux final-screen capture when byte-stream assertions can miss the
    failure.
  - Do not reintroduce command-name allowlists, prompt guessing, or timeouts as
    the correctness boundary.
  - Preserve the backend-driven model: foreground child owns the PTY, Aish only
    bridges input/output and waits for backend completion markers.

- [ ] Decide whether zsh command-start events need a user-visible terminal
  event-loop notification.
  - Current zsh `preexec` start markers are parsed into command results.
  - Add a separate frontend event only if a concrete UX or correctness issue
    needs it.

- [ ] Evaluate search-specific indexes only if scale or UX requires them.
  - Current browsing/search uses loaded JSONL state and in-memory completion
    caches.
  - If new indexes are added, they must not create plaintext persistent indexes
    while encryption is enabled.

## Documentation And Test Hygiene

- [ ] Design tmux test acceleration/parallelization as a separate effort.
  - Preserve current single-user terminal semantics while making independent
    scenarios safe to schedule concurrently.
  - Do not change tmux assertions or timing as part of unrelated sync,
    template, or terminal fixes.
  - Document the approved execution model before changing the harness.

- [ ] Keep the living docs aligned after behavior or test changes:
  - `README.md` for user-facing behavior.
  - `TESTS.md` for the maintainer-facing coverage map and latest test
    inventory.
  - `FULL_TESTS.md` for the complete distributed checklist.
  - `MANUAL_TESTS.md` for human-only checks.
  - `TESTING_MANUAL.md` for step-by-step external tester guidance.
  - `SPEC.md` for intended behavior and command semantics.

- [ ] When a manual failure is reproducible, add an automated regression at the
  highest practical layer.
  - Rust tests for pure logic and state transitions.
  - Expect tests for terminal byte-stream behavior.
  - Tmux capture tests for final rendered screen state.

- [ ] Before release or major merges, run the current verification set:

```sh
cargo fmt --check
cargo build
cargo test --lib
cargo test --test draft_execution -- --nocapture
cargo test --test first_run -- --nocapture
cargo test --test pty_backend -- --nocapture
cargo test --test expect_runner -- --test-threads=1 --nocapture
cargo test --test tmux_capture -- --test-threads=1 --nocapture
cargo clippy --all-targets -- -D warnings
git diff --check
```
