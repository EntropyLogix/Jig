# EntropyJig

EntropyJig (`jig`) is a file-based issue tracker and project memory for people
and coding agents. Issues are plain Markdown files with YAML frontmatter under
`.jig/`; there is no database, service, or mandatory Git integration. Each
checkout sees its own files. People can read the records without the CLI;
`jig` adds a shared workflow for dependencies, claims, and review.

See [the v1 specification](SPEC.md) for the workflow and file format.

## Install

Use Rust 1.89 or newer on Linux or macOS. From this repository:

```sh
cargo install --path . --locked
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
jig add TITLE [-p|--priority N] [--body TEXT|-] [--parent ID] [--dep ID ...]
jig ready [--json]
jig show ID [--json]
jig list [--status open|in_progress|review|closed|all] [--sort oldest|newest] [--json]
jig tree ID [--json]
jig claim ID [--by NAME] [--force]
jig unclaim ID
jig update ID [--title T] [-p|--priority N] [--body TEXT|-]
jig review ID [--summary TEXT]
jig request-changes ID --reason TEXT
jig close ID --reason TEXT [--summary TEXT]
jig reopen ID --reason TEXT
jig dep add|rm ID BLOCKER
jig note ID TEXT [--by NAME]
jig defer ID --until YYYY-MM-DD | --clear
jig remember TOPIC TEXT
jig recall [TOPIC] [--json]
```

`--json` may appear before or after the command for read commands. `--` stops
option parsing. `--body -` reads standard input; omitting `--body` never reads
stdin. A bare six-hex suffix (`a3f8c1`) is accepted as shorthand for `B-a3f8c1`.

Use `jig list --sort oldest` or `jig list --sort newest` to order by creation
date rather than priority. Unknown creation dates sort last in either
direction. The text view includes the stored creation date; JSON keeps the
same fields.

## Agent integrations

`jig init` installs agent integration into the project by default, so running
it in an already-initialized checkout is the opt-in path:

- `AGENTS.md` gains a `<!-- BEGIN ENTROPYJIG -->…<!-- END ENTROPYJIG -->`
  managed block with the workflow rules. Existing file bytes are preserved;
  an already-present marker pair is left alone (even if edited), and malformed
  or duplicated markers refuse the edit.
- `.claude/settings.json` and `.codex/hooks.json` each gain one SessionStart
  hook group running `jig start --hook-json` (no matcher: it fires for every
  session source, including after `/clear` and compaction). Codex 0.133 added the compact
  source; 0.145 or later restores the workflow before continuing an
  automatically compacted turn. The merge preserves all existing keys, groups,
  and array order; a file that is not valid JSON with the expected hook
  shape, or that already contains a different/duplicated EntropyJig handler,
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
cargo clippy --locked --all-targets -- -D warnings
```

## License

EntropyJig is released under the [MIT License](LICENSE). Third-party notices
and license texts are listed in [licenses/THIRD-PARTY.md](licenses/THIRD-PARTY.md).
