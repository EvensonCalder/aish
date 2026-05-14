# Aish Phase 2

Phase 2 is the hardening phase for turning the current implementation into a reliable real-world terminal wrapper. The core interactive wrapper is usable and well covered, but remaining work must focus on truthfully finishing incomplete features, expanding end-to-end coverage for actual user workflows, and fixing every issue discovered during testing.

## Audit Summary

Reviewed sources:

- `SPEC.md`
- `TODO.md`
- `TESTS.md`
- `README.md`
- `tests/expect/*.exp`
- `tests/expect_runner.rs`
- representative implementation modules: `app`, `config`, `terminal`, `sync`, `encryption`, `ai`, and PTY/test integration paths

Current state:

- The PTY shell wrapper, prompt rendering, draft editor, history/AI modes, private command parser, context flow, event log, templates, completion, external editor, multiline paste review, shell continuation handling, sync flow, diagnostics, and shell integration scaffolding are implemented and tested.
- Expect-driven end-to-end coverage exists and is the acceptance layer for visible terminal behavior.
- Phase 18 encryption/GPG remains the largest intentionally incomplete product area.
- Configurable key rebinding remains incomplete.
- PTY output events and timer/background events are not represented as independent central event-loop sources yet.
- Full automatic passthrough for arbitrary interactive/alternate-screen programs remains incomplete; allowlisted foreground passthrough exists.
- Some documentation had become stale during implementation and must stay aligned with actual behavior as Phase 2 continues.
- Fixed during Phase 2: `#completion` no longer reports the completion engine as unimplemented; it reports config and persists `#completion max <count>`.
- New critical defect found in real manual use: backend shell output can disappear from the final visible screen in actual `zsh` terminal sessions even though earlier PTY/unit and expect-byte-stream tests passed.
- Testing route correction: prompt/output regressions must be verified against final rendered terminal state; byte-stream-only expect assertions are not sufficient when later redraw clears or cursor motion can visually erase output.
- Test-harness correction: real interactive expect scenarios must not run concurrently inside one test binary. Parallel terminal sessions produced false `no prompt`/SIGBUS failures and do not represent actual single-user terminal behavior.
- Test-harness correction: Unicode final-screen behavior is covered through `tmux` pane capture instead of Tcl/expect when expect itself is unstable with the input encoding.

## Phase 2 Rules

- All work must be real-world use oriented.
- Do not mark a feature complete until the implementation, documentation, Rust tests, and expect tests agree.
- Every user-visible behavior must have an expect scenario unless it is impossible to exercise through a portable terminal test; the reason must be documented.
- Every bug found during Phase 2 must be recorded, fixed, and covered by a regression test at the highest practical layer.
- When byte-stream expect tests are insufficient for a real bug, add persistent real-terminal capture coverage, such as `tmux` pane capture scripts, and treat that as the acceptance layer for the defect.
- Useless tests should be replaced by tests that prove user-visible behavior, safety boundaries, persistence, or integration correctness.
- Do not create scheduler files.
- Do not rewrite git history, auto-resolve sync conflicts, or remove tracked files automatically.
- Do not overclaim encryption or secret storage before GPG-backed behavior is actually implemented.
- Keep `SPEC.md`, `TODO.md`, `TESTS.md`, `README.md`, and this file accurate after every feature or test change.

## Required Verification Before Phase 2 Commits

Run the focused verification set before committing meaningful code changes:

```text
cargo fmt --check
cargo test --lib
cargo test --test draft_execution
cargo test --test pty_backend
cargo test --test expect_runner
cargo test --test tmux_capture -- --test-threads=1
cargo test --test first_run
cargo clippy --all-targets -- -D warnings
git diff --check
cargo build
```

For documentation-only changes, `git diff --check` is sufficient unless the documentation changes test inventory, commands, or behavior claims.

## Workstream 1: Encryption And GPG

Goal: complete Phase 18 without weakening confidentiality or overstating implementation status.

Tasks:

- Implement a tested GPG command boundary before wiring interactive secret flows.
- Add fake-GPG or isolated test-key integration coverage for encrypt/decrypt behavior.
- Implement `#key set` using GPG-backed storage.
- Make AI key resolution follow the spec priority: configured environment variable first, then stored encrypted key.
- Implement `#encrypt on` only when future writes actually switch to encrypted storage.
- Implement `#encrypt off` only when decrypt/migration semantics are explicit and safe.
- Encrypt regular history, AI history, draft history, notes, and templates when encryption is enabled.
- Ensure no plaintext search or completion index is written when encryption is enabled.
- Add atomic write helpers for encrypted output.
- Add asynchronous unlock/loading behavior so Aish remains usable while encrypted history/templates are unavailable.
- Show a user-visible `history is still unlocking...` state where encrypted history/template data is not ready.
- Handle GPG/pinentry through `UnlockPassthrough` or an equivalent terminal-safe handoff.

Required tests:

- Unit tests for GPG command planning, error classification, redaction, and atomic write path selection.
- Integration tests with fake GPG or isolated test keys for `#key set`, stored-key reads, and encryption failures.
- Expect tests for `#key set` success or safe failure, `#key clear`, `#encrypt on`, `#encrypt off`, locked history behavior, and no plaintext secret leakage in output/logs.

## Workstream 2: End-To-End User Workflows

Goal: use expect scenarios to validate complete workflows as users experience them.

Add or strengthen expect coverage for:

- First-run startup, immediate `#doctor`, and clean exit with isolated `AISH_HOME`.
- Invalid config startup failure with a readable error and no terminal corruption.
- History persistence across Aish restarts.
- Draft persistence across Aish restarts when enabled.
- Notes persistence and later visibility through history/log-related surfaces once user-facing note browsing exists.
- Editor failure path: failed editor exits must preserve the draft and show a useful message.
- Picker cancellation for `Ctrl-R`, file picker, template picker, git branch picker, and env var picker where portable with fake `fzf`.
- Sync conflict presentation with local temporary remotes when a deterministic conflict can be created without network access.
- Mixed stdout/stderr output followed by prompt redraw.
- Passthrough behavior for another portable interactive fixture beyond `less` if feasible.

Every scenario must:

- Launch the real binary.
- Use isolated `AISH_HOME`.
- Avoid network access.
- Cleanly exit.
- Assert the visible terminal behavior, not just internal file state.

## Workstream 3: Event Loop And Terminal Robustness

Goal: close gaps between the spec's event model and the current terminal loop without overengineering.

Tasks:

- Decide whether independent PTY output events are required for real-world behavior beyond command-response execution.
- If needed, add a small event-loop path for PTY output that preserves prompt redraw correctness.
- Add timer/background event support only when a concrete feature requires it.
- Keep terminal cleanup reliable for normal exits, panics, editor handoff, passthrough, and interrupted commands.
- Ensure child PTY output remains terminal protocol, with no Aish-added framing newlines.

Required tests:

- Rust virtual-screen regressions for every redraw/framing fix.
- Expect tests for visible output ordering after commands, editor returns, paste review, completion panels, and clears.
- Add persistent `tmux`-driven scripts for actual-screen capture of prompt/output regressions that can escape byte-stream-only tests.

## Workstream 4: Keybindings And Pickers

Goal: finish user-facing input control without breaking common readline expectations.

Tasks:

- Implement configurable key rebinding only after a minimal, stable config shape is chosen.
- Keep default keybindings non-conflicting.
- Ensure `#help` and `#status` accurately describe configured bindings.
- Strengthen picker tests around cancellation, shell quoting, spaces, and replacement actions.

Required tests:

- Unit tests for keybinding config load/validation once implemented.
- Expect tests for at least one configured rebind if key rebinding ships.
- Expect tests for fake-`fzf` picker success and cancellation where practical.

## Workstream 5: Sync Hardening

Goal: preserve conservative sync semantics under real failure modes.

Tasks:

- Keep manual `#push` deterministic and local-testable.
- Improve conflict messages only when tests prove the current UX is insufficient.
- Ensure startup sync never creates scheduler files and never runs concurrently.
- Keep category toggles privacy-first and make documentation reflect the actual defaults.

Required tests:

- Existing local-remote success/failure expect scenarios must keep passing.
- Add conflict-specific expect coverage if conflict UX changes.
- Add tests for startup sync user-visible warnings if startup sync becomes more visible.

## Workstream 6: Documentation And Test Inventory

Goal: keep docs as trustworthy as tests.

Tasks:

- Update `TESTS.md` after every test count or coverage change.
- Update `README.md` when command behavior changes.
- Update `TODO.md` checkboxes only when the implementation and tests prove completion.
- Keep `PHASE2.md` as the active hardening checklist until these gaps are closed.
- Remove or rewrite stale statements instead of adding contradictory notes.

Required tests:

- Documentation-only changes must pass `git diff --check`.
- If docs mention a command or workflow as implemented, there must be a Rust or expect test proving it unless the doc explicitly marks it as a limitation.

## Completion Criteria

Phase 2 is complete when:

- All critical incomplete items from Phase 18 are either implemented with tests or explicitly deferred with clear non-goal language.
- `TODO.md` has no stale completion claims.
- `TESTS.md` accurately reflects current test inventory and known gaps.
- All user-visible command behavior in `README.md` has corresponding implementation and tests.
- The full verification set passes.
- No known Phase 2 bug remains unrecorded, unfixed, or without regression coverage.
