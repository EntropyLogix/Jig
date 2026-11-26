# Jig

Jig is a file-based issue tracker and project memory for people
and coding agents. Issues are plain Markdown files with YAML frontmatter under
`.jig/`; there is no database, service, or mandatory Git integration. Each
checkout sees its own files. People can read the records without the CLI;
`jig` adds a shared workflow for dependencies, claims, and review.

See [the v1 specification](SPEC.md) for the workflow and file format. Jig lives
at [EntropyLogix/Jig](https://github.com/EntropyLogix/Jig).

## Install

Use Rust 1.89 or newer on Linux or macOS. From this repository checkout:

```sh
cargo install --path . --locked
```

To install a tagged release from GitHub (once the owner has published the
tag), for example `v1.0.0`:

```sh
cargo install --locked --git https://github.com/EntropyLogix/Jig --tag v1.0.0
```

The `jig` binary lands on your Cargo `PATH` (usually `~/.cargo/bin`) and needs
no other checkout at runtime. Its only direct third-party dependency is pinned
`serde_json`, used for editing agent configuration during `init`; license
texts for the resolved runtime crates live in `licenses/`.

## Quick start

```sh
cd myproject
jig init                    # creates the registry and sets up agent integration
jig start                   # short workflow summary
id=$(jig add "Fix flaky test" -p 1 --body "details here")
jig approve "$id"           # a proposal becomes open work (human action)
jig ready                   # unblocked open issues
jig show "$id"              # full issue (works from subdirectories)
jig claim "$id" --by agent-1
jig note "$id" "reproduced on linux"
jig review "$id" --summary "approach: quarantine the test"
jig close "$id" --reason "verified fixed" --summary "quarantined"
jig remember build.cmd "cargo build --locked"
jig recall                  # list remembered topics
```

Files may be browsed and hand-edited directly; agents go through `jig` so that
mutations are serialized by `.jig/.lock` and cross-directory moves are journaled
through `.jig/.move` (a crashed move is finished on the next command).

## Command reference

```text
jig init [--no-hooks]
jig start [--json|--hook-json]
jig add TITLE [--approved] [-p|--priority N] [--body TEXT|-] [--parent ID] [--dep ID ...]
jig ready [--json]
jig show ID [--json]
jig list [--status proposed|open|claimed|review|closed|all] [--sort oldest|newest] [--json]
jig tree ID [--json]
jig approve ID
jig revise ID --reason TEXT
jig claim ID [--by NAME] [--force]
jig unclaim ID
jig update ID [--title T] [-p|--priority N] [--body TEXT|-]
jig review ID [--summary TEXT]
jig rework ID --reason TEXT
jig close ID --reason TEXT [--summary TEXT]
jig reopen ID --reason TEXT
jig dep add|rm ID BLOCKER
jig note ID TEXT [--by NAME]
jig defer ID --until YYYY-MM-DD | --clear
jig remember TOPIC TEXT
jig recall [TOPIC] [--json]
jig check [--json]
```

A bare `jig` prints the command reference above (same as `jig help`,
`jig --help`, or `jig -h`). `--json` may appear before or after the command for
read commands. `--` stops option parsing. `--body -` reads standard input;
omitting `--body` never reads stdin. A bare six-hex suffix (`a3f8c1`) is
accepted as shorthand for `B-a3f8c1`.

`jig check` audits the registry and reports every problem in one run —
unreadable issue files, name/ID mismatches, statuses in the wrong directory,
duplicate IDs, conflict markers, missing blockers or ancestors, dependency
cycles, inconsistent fields, a pending or mismatched `.jig/.move` journal, an
unreadable `memory.md`, unexpected files, and a leftover legacy `/.*.tmp-*`
rule in `.jig/.gitignore` (the anchored form never ignored nested temporary
files). Each problem prints as `{path,problem,hint}` (JSON or text);
exit is 1 when problems exist, 0 when clean. `check` only reports — it never
modifies `.jig/`, finishes a pending move, or creates `.jig/.lock`. Agents do
not repair registries: when `jig ready` warns, run `jig check` and report the
result to a human instead of removing dependencies or editing files.

`jig ready` is tolerant: a corrupt or inconsistent open record is skipped with
one `jig: warning:` line on stderr while valid issues are still listed and the
exit stays 0. `jig list` and `jig tree` are tolerant the same way — every
readable issue is listed and each skipped file (unreadable, conflicted,
duplicated, or in the wrong directory) produces one warning; `tree` reattaches
children of an unusable intermediate node under the nearest visible dotted
ancestor. Pointed reads still fail on an unusable record: `show ID` and
`tree ID` refuse an ID whose own file is unusable, as does `claim`. Read
commands (`start`, `ready`, `show`, `list`, `tree`, `recall`, `check`) also
work on a checkout without write access to `.jig/`; they never need to create
`.jig/.lock`. On a writable registry a read command may finish an interrupted
move under an upgraded lock; where it cannot write, a pending move is reported
as an error.

`jig defer ID --until YYYY-MM-DD` hides an issue from `ready` until that date
(a deferral ending today no longer hides it); `jig defer ID --clear` removes
the marker and is allowed on a closed issue. Closed ancestors never defer
their descendants.

`jig add` creates a `proposed` issue — a contract draft, not work; agents add
without `--approved`. A human moves it to `open` with `jig approve` (or
`--approved` at creation), returns it with `jig revise --reason`, or rejects it
with `jig close`. Work runs `open` → `claimed` (`jig claim`) → `review`
(`jig review`, `jig rework` sends it back) → `closed`.

Use `jig list --sort oldest` or `jig list --sort newest` to order by creation
date rather than priority. Unknown creation dates sort last in either
direction. The text view includes the stored creation date; JSON keeps the
same fields.

## Agent integrations

`jig init` installs agent integration into the project by default, so running
it in an already-initialized checkout is the opt-in path:

- `AGENTS.md` gains a `<!-- BEGIN JIG v:N hash:H -->…<!-- END JIG -->`
  managed block with the workflow rules. Existing file bytes are preserved;
  an unmodified older-version block is upgraded in place, a block edited by
  hand (hash mismatch) is left alone with a notice, and malformed or
  duplicated markers refuse the edit. `jig check` reports a missing or
  outdated block.
- `.claude/settings.json` and `.codex/hooks.json` each gain one SessionStart
  hook group running `jig start --hook-json` (no matcher: it fires for every
  session source, including after `/clear` and compaction). Codex 0.133 added the compact
  source; 0.145 or later restores the workflow before continuing an
  automatically compacted turn. The merge is byte-surgical: the new hook group
  text is spliced into the original bytes, so every existing key, group, array
  order, and byte of unrelated content is preserved — nothing is re-serialized
  or reformatted. Inserted lines follow the file's own indentation (measured
  tab or N-space unit), its newline style (LF or CRLF), and a one-line file
  gains the group compactly without added whitespace. A file that is not
  valid JSON with the expected hook shape,
  that spells `hooks` or `SessionStart` twice (including escaped spellings),
  or that already contains a different or duplicated Jig handler,
  refuses init rather than being repaired. `jig` must be on `PATH` for both
  clients. Codex shows the new project hook once via `/hooks` for manual
  confirmation; Claude applies its own project trust. No global settings and
  no Git hooks are touched.
- `jig init --no-hooks` skips the hook configuration (it does not remove
  existing hooks) but still updates `AGENTS.md`.

The hook payload is the static `jig start` workflow text wrapped as
`{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":…}}`.
It reads no issue or memory contents, takes no lock, and runs no other
executables.

## Issue file format

Each issue is `<id>.md` under `.jig/open` or `.jig/closed`: a `---`-delimited
block of fixed-order one-line YAML fields (quoted scalars, `status` plain,
`null`/`[]` for absent values), then a blank line, an optional
`## Closing Summary` blockquote, the free-form Markdown body, and the managed
`## History` and `## Notes` sections. Those three headings are reserved at
column 0 outside fenced code; everything else in the body is preserved
byte-for-byte.

## Scope notes

- Coordination is per-checkout via the local `.jig/.lock` file lock. There is no
  global arbitration between independently edited copies; conflicts surface as
  diagnostics (duplicate IDs, merge markers, move-journal mismatches) for
  explicit resolution.
- New IDs come from a hash of fresh OS randomness and a random per-machine
  install identifier stored under `$XDG_STATE_HOME` (or
  `~/.local/state`); the IP, MAC, and hostname are never used, and the install
  identifier never enters the project.
- `jig` never runs Git, installs Git hooks, or picks winners by timestamp;
  people using the project choose when and how to synchronize `.jig/`.

## Development

```sh
cargo fmt --all -- --check
cargo test --locked --lib --test cli
cargo +1.89 test --locked
cargo clippy --locked --all-targets -- -D warnings
```

## License

Jig is released under the [MIT License](LICENSE). Third-party notices
and license texts are listed in [licenses/THIRD-PARTY.md](licenses/THIRD-PARTY.md).
