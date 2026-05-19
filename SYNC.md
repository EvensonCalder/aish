# Aish Sync

Aish sync stores Aish-managed data in a Git repository under the Aish home
directory, usually `~/.aish`.

## Commands

```text
#set-remote <git-url>
#sync now
#push
#sync resolve-union
#sync continue
#sync abort
#sync <schedule>
#sync startup on|off
#sync exit on|off
#sync ai|history|templates|drafts on|off
#sync off
```

`#push` is an alias for `#sync now`.

## What Sync Includes

By default, sync includes:

- `history/ai.jsonl`
- `history/regular.jsonl`
- `history/notes.jsonl`
- `history/draft.jsonl`
- `templates/templates.jsonl`
- encrypted equivalents when encryption is enabled
- Aish-managed `.gitignore` and `.gitattributes` metadata

By default, sync does not include local configuration, cache, logs, secrets, or
temporary files. `config.toml` stays local because it can contain machine-specific
paths and startup settings.

## Sync Flow

`#sync now` and `#push` run this flow:

```text
git add -- <managed Aish files>
git commit -m "sync aish data"   # skipped when nothing is staged
git pull --no-rebase --no-edit
git push -u origin HEAD
```

For a new Aish home that is not a Git repository yet, Aish runs `git init`, adds
the configured `origin`, skips the first pull, then stages, commits, and pushes.

## Conflict Handling

Aish configures Git's `merge=union` driver for plaintext Aish JSONL files. This
keeps both sides of independent appends, which avoids most conflicts when two
machines write history, drafts, notes, AI history, or templates before syncing.

If a conflict remains, Aish leaves the merge in progress and prints choices:

- `#sync resolve-union` keeps both sides for plaintext Aish conflict files, then
  commits and pushes.
- `#sync continue` continues after you manually edit conflicts and run `git add`.
- `#sync abort` cancels the interrupted merge or rebase.

Aish does not auto-union encrypted `*.jsonl.gpg` files because concatenating
ciphertext can corrupt the encrypted data. Resolve those manually or abort.

## Automatic Triggers

`#sync <schedule>` stores a startup-only due check. Aish does not install cron,
launchd, systemd, or other scheduler files.

Supported schedules are `@hourly`, `@daily`, `*/N * * * *`, `0 */N * * *`,
`0 0 * * *`, and `0 0 */N * *`.

`#sync startup on` runs one sync at each startup. `#sync exit on` runs one sync
at the exit durability boundary.
