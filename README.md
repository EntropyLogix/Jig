# Billet

Billet (command `blt`) is a file-based issue tracker and project memory for
people and coding agents. Issues are plain Markdown files with YAML frontmatter
under `.blt/`; there is no database, service, or mandatory Git integration.
Each checkout sees its own files. People can read the records without the CLI;
`blt` adds a shared workflow for dependencies, claims, and review.

See [the v1 specification](SPEC.md) for the workflow and file format. Billet
lives at [EntropyLogix/Billet](https://github.com/EntropyLogix/Billet).

## Install

Billet runs on macOS and Linux, and on Windows inside WSL. With Homebrew, on
macOS or Linux:

```sh
brew install entropylogix/tap/blt
```

`brew upgrade blt` installs later releases. Without Homebrew:

```sh
curl -fsSL https://raw.githubusercontent.com/EntropyLogix/Billet/main/install.sh | sh
```

The script downloads the latest release for your platform, checks it against
the release's `SHA256SUMS`, and installs `blt` into `~/.local/bin`. Pass
`--version X.Y.Z` or `--dir DIR` after `sh -s --` to choose another release or
directory. It installs nothing when the checksum does not match.

From source, with Rust 1.89 or newer:

```sh
cargo install --locked --git https://github.com/EntropyLogix/Billet --tag v1.0.3
cargo install --path . --locked    # from a checkout
```

Every release on GitHub carries `blt` for macOS (arm64 and x86_64) and Linux
(x86_64 and arm64, statically linked) with a `SHA256SUMS` file. Pushing a `v*`
tag runs `.github/workflows/release.yml`: it checks that the tag matches the
package version, runs the tests, builds the four archives, publishes the
release, and updates the Homebrew formula when the `TAP_TOKEN` secret is set;
without it, `packaging/homebrew/formula.sh VERSION SHA256SUMS` prints the
formula to commit by hand.

The `blt` binary needs no other checkout at runtime. Its only direct third-party
dependency is pinned `serde_json`, used for editing agent configuration during
`init` and for JSON-RPC messages in `blt mcp`. License texts for the resolved
runtime crates live in `licenses/`.

## Quick start

```sh
cd myproject
blt init                    # creates the registry and sets up agent integration
blt start                   # short workflow summary
id=$(blt add "Fix flaky test" -p 1 --body "details here")
blt approve "$id"           # a proposal becomes open work (human action)
blt ready                   # unblocked open issues
blt show "$id"              # full issue (works from subdirectories)
blt claim "$id" --by agent-1
blt note "$id" "reproduced on linux"
blt review "$id" --summary "approach: quarantine the test"
blt close "$id" --reason "verified fixed" --summary "quarantined"
blt remember build.cmd "cargo build --locked"
blt recall                  # list remembered topics
```

Files may be browsed and hand-edited directly; agents use `blt` for registry
changes so that mutations are serialized by `.blt/.lock` and cross-directory
moves are journaled through `.blt/.move` (a crashed move is finished on the
next command). Agents write issue documents directly under `.blt/docs/KEY/`.

## Workflow

An issue moves through five statuses:

| Status | Meaning | Next step |
| --- | --- | --- |
| `proposed` | definition: the contract is being worked out, not work yet | `approve` → `open`; `close` rejects it |
| `open` | the contract is agreed and can be implemented | `claim` → `claimed`; `revise` → `proposed` |
| `claimed` | one agent or person is working on it | `review` → `review`; `unclaim` → `open` |
| `review` | the work is handed over for review | `rework` → `claimed` (same assignee); `close` → `closed` |
| `closed` | done or rejected; the file moves to `.blt/closed/` | `reopen` → `open` |

A person may also `close` an issue from any active status, with or without a
review, for example as a duplicate or as obsolete.

`blt add` creates a proposal (`--approved` starts at `open`). Discussion and
analysis are folded into its description as the contract changes, and long
material belongs in its issue documents. Agents add without `--approved` and
do not treat a proposal as work. Approving, revising, returning for rework,
and closing issues are human decisions unless a person explicitly instructs
an agent for that issue.

## Issue numbers

People and agents refer to an issue by its handle: the project prefix and a
number, `B1`, `B2`, … in order of creation, and `B1.1` for a subtask of `B1`.
The prefix is set once with `blt init --prefix AN-` (default `B`) and lives in
`.blt/config.json`; changing it there changes every handle at once. Commands
also take the bare number (`blt show 12`) and ignore case (`b12`).

Behind every handle is a random six-character key (`5a3f35`) that names the
issue's file and documents directory and that `parent` and `depends_on`
store. It keeps copies of a registry edited on different machines from ever
merging two issues into one. Two copies can still give the same number to
different issues before they are synchronized: `list`, `ready`, and `tree` then
warn, the handle is refused as ambiguous, `blt check` reports both files, and
a person gives one of them a free number with `blt update KEY --number N`.
An issue whose number cannot be read, such as a blocker whose file is gone,
is shown by its key.

## MCP server

`blt mcp` is an optional Model Context Protocol server over stdio. The client
starts it for a session and ends it; `blt` does not run a daemon. Use it for an
agent without a shell or for a client where a person approves tool calls. An
agent with a shell can use the CLI and its hooks, which use less context than
the MCP tool schemas.

The server searches for `.blt/` upward from its working directory for every
tool call, so the client must start it in the project. `blt init` does not
register the server with a client.

Claude Code, using the project scope, can register it with:

```sh
claude mcp add --scope project blt -- blt mcp
```

For a project permission policy that asks before human decisions, use the
following entries in `.claude/settings.json`:

```json
{
  "permissions": {
    "ask": [
      "mcp__blt__approve",
      "mcp__blt__revise",
      "mcp__blt__rework",
      "mcp__blt__close",
      "mcp__blt__reopen",
      "mcp__blt__release",
      "mcp__blt__init"
    ]
  }
}
```

Codex uses a TOML entry in `~/.codex/config.toml`, or in `.codex/config.toml`
for a trusted project:

```toml
[mcp_servers.blt]
command = "blt"
args = ["mcp"]
```

EntropyForge uses this server entry in `config/config.json`. The server runs
in the repository of the session. Set the client permission at the top level:

```json
{
  "mcpServers": {
    "blt": {
      "command": "blt",
      "args": ["mcp"]
    }
  },
  "permission": "ask"
}
```

## Command reference

```text
blt init [--no-hooks] [--prefix P]
blt start [--json|--hook-json]
blt add TITLE [--approved] [-p|--priority N] [--body TEXT|-] [--parent ID] [--dep ID ...]
blt ready [--json]
blt show ID [--json]
blt list [--status proposed|open|claimed|review|closed|all] [--release NAME] [--sort oldest|newest] [--json | --markdown FILE]
blt tree ID [--json]
blt approve ID
blt revise ID --reason TEXT
blt claim ID [--by NAME] [--force]
blt unclaim ID
blt update ID [--title T] [-p|--priority N] [--body TEXT|-] [--release NAME] [--number N]
blt review ID [--summary TEXT]
blt rework ID --reason TEXT
blt close ID --reason TEXT [--summary TEXT]
blt reopen ID --reason TEXT
blt release NAME
blt dep add|rm ID BLOCKER
blt note ID TEXT [--by NAME]
blt defer ID --until YYYY-MM-DD | --clear
blt remember TOPIC TEXT
blt recall [TOPIC] [--json]
blt check [--json]
blt mcp
```

A bare `blt` prints a grouped summary of these commands (same as `blt help`,
`blt --help`, or `blt -h`). `--json` may appear before or after the command for
read commands. `--` stops option parsing. `--body -` reads standard input;
omitting `--body` never reads stdin. `ID` is a handle (`B12`, `B12.3`), a bare
number, or a key (see Issue numbers).

Text `blt list` without arguments groups issues by status in the order `review`,
`proposed`, `claimed`, `open`. A status, release, or chronological sort filter
keeps the flat list format; `--status all` is flat too. JSON is always a flat
array.

`blt list --markdown FILE` writes the same selection to FILE as one Markdown
document for reading: a contents list followed by every issue with its status
line and full description. It prints only `wrote N issues to FILE`, so an agent
that runs it does not fill its context. The file is a snapshot; edit issues
with `blt`. It cannot be written inside `.blt/`, and the MCP server does not
offer it:

```sh
blt list --markdown ~/issues.md
blt list --markdown ~/closed.md --status closed --sort newest
```

`blt check` audits the registry and reports every problem in one run —
unreadable issue files, name/ID mismatches, statuses in the wrong directory,
duplicate IDs, conflict markers, missing blockers or ancestors, dependency
cycles, inconsistent fields, a pending or mismatched `.blt/.move` journal, an
unreadable `memory.md`, unexpected files, and a leftover legacy `/.*.tmp-*` rule
in `.blt/.gitignore` (the anchored form never ignored nested temporary
files). Each problem prints as `{path,problem,hint}` (JSON or text); exit is 1
when problems exist, 0 when clean. `check` only reports — it never modifies
`.blt/`, finishes a pending move, or creates `.blt/.lock`. Agents do not
repair registries: when `blt ready` warns, run `blt check` and report the result
to a human instead of removing dependencies or editing files.

`blt ready` is tolerant: a corrupt or inconsistent open record is skipped with
one `blt: warning:` line on stderr while valid issues are still listed and the
exit stays 0. `blt list` and `blt tree` are tolerant the same way — every
readable issue is listed and each skipped file (unreadable, conflicted,
duplicated, or in the wrong directory) produces one warning; `tree` reattaches
children of an unusable intermediate node under the nearest visible dotted
ancestor. Pointed reads still fail on an unusable record: `show ID` and
`tree ID` refuse an ID whose own file is unusable, as does `claim`. Read
commands (`start`, `ready`, `show`, `list`, `tree`, `recall`, `check`) also
work on a checkout without write access to `.blt/`; they never need to create
`.blt/.lock`. On a writable registry a read command may finish an interrupted
move under an upgraded lock; where it cannot write, a pending move is reported
as an error.

`blt defer ID --until YYYY-MM-DD` hides an issue from `ready` until that date
(a deferral ending today no longer hides it); `blt defer ID --clear` removes
the marker and is allowed on a closed issue. Closed ancestors never defer
their descendants.

Use `blt list --sort oldest` or `blt list --sort newest` to order by creation
date rather than priority. Unknown creation dates sort last in either
direction. The text view includes the stored creation date; JSON keeps the
same fields.

Every issue has a `priority` from 0 (highest) to 4; `blt add` without `-p`
sets 2. `ready` and the default `list` order issues by priority.
`blt update ID -p N` changes it in any status and records no history entry.
A file whose `priority` is missing, `null`, or outside 0–4 is invalid:
`blt check` reports it, `ready`, `list`, and `tree` skip it with a warning,
`show` and writes refuse it, and a person fixes the field by hand.

## Issue documents

Long analyses, measurements, implementation instructions, and review or audit
reports for one issue live under `.blt/docs/KEY/`, named by the issue's key.
The directory stays put across status changes and renumbering. People and agents
write these document files directly, while `blt` does not create, edit, move, or
remove them. The issue description keeps the current contract and can reference
the documents that support it.

For example:

```text
.blt/docs/5a3f35/analysis.md
.blt/docs/5a3f35/review/round-1.md
.blt/docs/5a3f35/review/round-2.md
```

`blt show B12` always ends with its `Documents: .blt/docs/5a3f35/`
line. It adds `(none)` when the directory has no files, or indents the sorted
file paths by two spaces. JSON `show` returns the directory in `docs_dir` and
the files in `docs`. `blt check` reports a file directly under
`.blt/docs/` or a directory that is not named after an existing issue.

## Subtasks and dependencies

Choose the relationship first. A part of an issue's work is a subtask created
with `--parent`. Separate work that must happen first is a blocker added with
`blt dep add ID BLOCKER`. A parent is absent from `ready` while any descendant
is active, and returns after all subtasks close. A closed blocker stays in
`depends_on` and stops blocking until it is reopened. `blt show ID` reports the
same decision on its `Ready:` line.

For a work breakdown, create the subtasks under the parent and express order
inside that tree when one part must precede another:

```sh
parent=$(blt add "Implement renderer" --approved)
first=$(blt add "Prepare data" --parent "$parent" --approved)
second=$(blt add "Render data" --parent "$parent" --approved)
blt dep add "$second" "$first"
```

For example, within the `B312` tree, `blt dep add B312.2 B312.1` records that
subtask `B312.2` waits for `B312.1`.

When an agent discovers separate prerequisite work, it creates the issue,
adds the dependency, records a note, and releases its claim:

```sh
blocker=$(blt add "Fix prerequisite")
blt dep add CURRENT "$blocker"
blt note CURRENT "Blocked by prerequisite work"
blt unclaim CURRENT
```

Use `blt dep rm ID BLOCKER` only to undo a dependency added by mistake, and
leave a note explaining the correction.

## Releases

Plan a release with an ordinary issue and dependencies. Approve the release
issue, then add each planned issue as a dependency:

```sh
rel=$(blt add "Release 1.3.0" --approved)
blt dep add "$rel" ID
blt dep add "$rel" ANOTHER_ID
blt show "$rel"
```

The release issue shows its scope and what still blocks it. Move work to a
different release with `blt dep rm` followed by `blt dep add`.

An issue deliberately postponed stays `proposed`, because it is not work yet,
or receives `blt defer ID --until YYYY-MM-DD`. It belongs to a release through
the release issue's dependencies. Issues have no planning-release field.

When the release ships, seal the closed issues and inspect the result:

```sh
blt release 1.3.0
blt list --release 1.3.0
```

If `blt release 1.2.4` was forgotten, `blt release 1.2.5` seals every still
unsealed closed issue. Correct an individual issue with
`blt update ID --release 1.2.4`.

`release` records where an issue actually shipped. It does not record planned
scope, so it is set only on closed issues.

## Agent integrations

`blt init` installs agent integration into the project by default, so running
it in an already-initialized checkout is the opt-in path:

- `AGENTS.md` gains one fixed line that points agents at `blt start`:

  ```text
  This project tracks work with `blt` (https://github.com/EntropyLogix/Billet): run `blt start` before you begin and follow its rules; if `blt` is missing, ask a person to install it.
  ```

  The line never changes between versions, so there are no markers to
  maintain. `init` appends it when missing and leaves every other byte
  alone; `blt check` reports a file without it. The workflow rules
  themselves come from `blt start` through the hook below, so they are not
  repeated in `AGENTS.md`.
- `.claude/settings.json` and `.codex/hooks.json` each gain one SessionStart
  hook group running `blt start --hook-json` (no matcher: it fires for every
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
  or that already contains a different or duplicated Billet handler,
  refuses init rather than being repaired. `blt` must be on `PATH` for both
  clients. Codex shows the new project hook once via `/hooks` for manual
  confirmation; Claude applies its own project trust. No global settings and
  no Git hooks are touched.
- `blt init --no-hooks` skips the hook configuration (it does not remove
  existing hooks) but still updates `AGENTS.md`.

The hook payload is the static `blt start` workflow text wrapped as
`{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":…}}`.
It reads no issue or memory contents, takes no lock, and runs no other
executables.

## Optional: blt-crew

[`crew/`](crew/) is an optional skill for Claude Code and Codex that runs a
review loop on top of Billet:

1. Optionally, an architect turns a proposal into a contract, with an
   `architecture.md` and subtasks, before a person approves it.
2. A developer claims the issue, implements it, and hands it over with
   `blt review`.
3. Any number of auditors check the work independently, each writing its own
   report.
4. A lead auditor merges the reports into one decision. On `blt rework` the
   developer works only from that decision, and the loop repeats.
5. A person closes the issue.

`/blt-crew status ID` lists each round's handoff, audit reports with their
verdicts, and decision.

Crew uses only Billet's statuses, history, notes, and issue documents; the
`blt` binary does not depend on it. Install it with `crew/install.sh`. See
[`crew/README.md`](crew/README.md) for the full loop, rounds, and layout.

## Issue file format

Each issue is `<id>.md` under `.blt/open` or `.blt/closed`: a
`---`-delimited block of fixed-order one-line YAML fields (quoted scalars,
`status` plain, `null`/`[]` for absent values), then a blank line, an optional
`## Closing Summary` blockquote, the free-form Markdown body, and the managed
`## History` and `## Notes` sections. Those three headings are reserved at
column 0 outside fenced code; everything else in the body is preserved
byte-for-byte.

## Scope notes

- Coordination is per-checkout via the local `.blt/.lock` file lock. There is
  no global arbitration between independently edited copies; conflicts surface
  as diagnostics (duplicate IDs, merge markers, move-journal mismatches) for
  explicit resolution.
- New IDs come from a hash of fresh OS randomness and a random per-machine
  install identifier stored under `$XDG_STATE_HOME` (or
  `~/.local/state`); the IP, MAC, and hostname are never used, and the install
  identifier never enters the project.
- `blt` never runs Git, installs Git hooks, or picks winners by timestamp;
  people using the project choose when and how to synchronize `.blt/`.

## Development

The tests need `git` and `jq` on `PATH` (the `crew` loop test uses them).

```sh
cargo fmt --all -- --check
cargo test --locked --lib --test cli
cargo +1.89 test --locked
cargo clippy --locked --all-targets -- -D warnings
```

## License

Billet is released under the [MIT License](LICENSE). Third-party notices
and license texts are listed in [licenses/THIRD-PARTY.md](licenses/THIRD-PARTY.md).
