# Jig v1

Jig is a file-based issue tracker and project memory for people and
coding agents. The executable is `jig`. This document records the agreed
contract and acceptance checks for v1.

## Agreed Contract

### Installation And Project Discovery

- Install `jig` once per user and machine, on `PATH`. The project lives at
  `https://github.com/EntropyLogix/Jig`; the package, library, and binary
  are all named `jig`. Machine-local state lives under
  `$XDG_STATE_HOME/jig` (or `~/.local/state/jig`).
- `jig init`, run in the project root, creates `.jig/`. Subsequent commands
  find it by walking up from the current directory. Without a registry in the
  current project, report an error. No path argument, project override,
  machine-local project config, or redirect to another checkout is needed.
  `jig start` does not need to print the absolute path on each run.
- `jig init [--no-hooks]` also installs the approved agent integration by
  default: a marker-delimited `## Issue tracking with Jig` block in
  the project's `AGENTS.md` (see Agent Instructions Block), and a
  project-local SessionStart hook running
  `jig start --hook-json` merged into `.claude/settings.json` and
  `.codex/hooks.json`. The merge inserts only the new hook group into the
  existing text: every other byte of the file, including key order,
  indentation, and whitespace, stays unchanged. A missing file is created
  with only the required structure. The merge refuses malformed files
  instead of rewriting them, and is idempotent.
  `--no-hooks` skips the hook configuration only; it is not an uninstall
  and `AGENTS.md` is still updated. `jig` must be on `PATH` for the clients;
  users confirm the hook once via `/hooks` (Codex) or the client's own
  project trust. The hook payload is the static `jig start` workflow text
  only, including sessions resumed or compacted via SessionStart — never
  issue or memory contents — and `jig` performs no Git hooks or other
  runtime hook operations.
- There is no database, central service, or mandatory Git integration.
  Each checkout sees its own files. If you use Git, commit and synchronize
  those files like any other project files. `jig` does not run Git, commit,
  push, install Git hooks, or require a dedicated branch.

### Files, IDs, And Concurrent Work

- There is one Markdown file per issue, with YAML frontmatter and a
  human-readable body. Active statuses (`proposed`, `open`, `claimed`,
  `review`) live in `.jig/open/`; closed issues live in `.jig/closed/`. The
  archive can be browsed without `jig`. Closing and reopening move the file,
  not its ID.
- Every issue has two identifiers with separate jobs. The **key** is the
  internal, random identity: it names the file, the documents directory,
  and every reference in `parent` and `depends_on`. The **handle** is what
  people and agents read and write: the project's prefix followed by the
  issue's number (`B494`), and for a subtask its root's handle plus the
  dotted part of its key (`B494.2`). Jig shows handles everywhere except
  inside paths; an issue whose number cannot be read (a missing or
  unreadable root) is shown by its key.
- A top-level key is six lowercase hex digits that start with a digit and
  contain at least one letter (`5a3f35`). Generate it from a hash of a
  random local installation identifier and fresh system randomness, and
  retry values of any other shape. The installation identifier stays out
  of the project; do not use the IP, MAC, or hostname. Check both
  directories under the lock and retry on a local collision.
- `add` gives a top-level issue the largest number in the registry, open
  and closed, plus one, under the lock; numbers are never reused. The
  `number:` line of a file Jig cannot read still counts, so repairing that
  file never creates a duplicate. Copies edited independently may assign
  the same number; identity stays intact because keys differ. A handle
  that matches two issues is refused and names every match; `list`,
  `ready`, and `tree` warn once per duplicate; `check` reports it. A person
  fixes it with `jig update ID --number N` (an unused number, on a
  top-level issue, recorded in the history); agents do not renumber.
- The prefix is the project setting `prefix` in `.jig/config.json`
  (`{"prefix": "B"}`), written by `jig init [--prefix P]` with default `B`:
  1 to 8 uppercase ASCII letters, optionally followed by `-` (`B`, `AN`,
  `AN-`). Handles are derived, so changing the prefix changes every handle
  at once. A missing or invalid configuration stops every command except
  `check`, which reports it, and `init`, which writes a missing one.
- Commands take an issue as a handle (case-insensitive: `B494`, `b494.2`),
  a bare number (`494`), or a key (`5a3f35`, for tools that read paths and
  for references whose issue no longer exists). A handle always starts with
  a letter, a number has none, and a key starts with a digit and has one,
  so no input is ambiguous between the forms.
- Subtasks use dotted keys (`5a3f35.1`, `5a3f35.1.1`) and store no number.
  Allocate the next
  number after the largest direct child under the lock, including closed
  children. A closed child does not free its number.
- Writes use a local lock and atomic file replacement. Processes using the
  same `.jig/` cannot both claim the same issue. Independently edited copies
  may conflict after synchronization; we accept that rather than partition
  the queue. Do not silently overwrite an issue, renumber one side, or pick
  the newest claim by timestamp. When a conflict is present in the files,
  surface it for explicit resolution.
- Humans may read and edit the Markdown directly, provided no one else is
  writing the same issue at the time. Agents make changes through `jig`,
  except issue documents, which they write directly (see Issue Documents).

### Issue Workflow

- The lifecycle is `proposed` → `open` → `claimed` → `review` → `closed`.
  `proposed` means the contract is not approved yet and the issue is not
  work. `add` creates a `proposed` issue, including subtasks created with
  `--parent`; `add --approved` creates it directly as `open`. Agents create
  issues without `--approved`; approving is a human action.
- `proposed` is where an issue is defined. Discussion and analysis happen
  while the issue is `proposed`; their outcome is folded into the
  description, which states the current contract (problem, decisions,
  scope, acceptance) and is rewritten as decisions change rather than
  appended to. Agents may `update` the description and add issue documents
  (see Issue Documents) while working on the definition with a person;
  `claim` still refuses a `proposed` issue. `approve` means the contract is
  agreed and the issue can be implemented.
- Every issue has a priority from 0 (highest) to 4; there is no issue
  without a priority. `add` without `-p` sets 2. People set the intended
  priority with `update ID -p N`, typically when approving. `update -p`
  works in every status and adds no history entry.
- `approve ID` moves a `proposed` issue to `open` and records a history
  entry. `revise ID --reason TEXT` returns an `open` issue to `proposed`
  when its approved contract has to be reworked before anyone starts on
  it. Both are workflow rules for humans, not an access-control boundary.
  The status applies to one issue only: a `proposed` parent does not hide
  its subtree, and approving a parent does not approve its children.
- `ready` shows only `open` issues that are not deferred and whose blockers
  are all closed. A missing blocker is not a closed blocker. `depends_on`
  records blockers on the waiting issue; `dep add` rejects cycles. A
  closed blocker stays in `depends_on`: it no longer blocks, and blocks
  again if it is reopened. `dep rm` removes a link that was added by
  mistake. A parent is not ready while any of its descendants is active
  (`proposed`, `open`, `claimed`, or `review`); this is derived from the
  tree and adds nothing to `depends_on`. Subtasks are the parts of the
  parent's scope, and the parent returns to `ready` for the final check
  once they are closed. Deferring a parent hides the entire subtree
  without editing the children; their own blockers and deferrals still apply.
- An issue whose blocker or ancestor is missing or unreadable is not ready.
  The same holds for an issue with an unusable descendant: a file whose
  `parent` field places it under the issue or, when the file cannot be
  parsed, whose dotted ID in the file name does. `ready` skips such an
  issue, still lists every other eligible issue, prints one
  warning per skipped issue to stderr naming the unresolved reference and
  pointing to `jig check`, and exits 0. JSON on stdout keeps its normal
  shape. `claim` of such an issue fails as before, naming the unusable
  file; `--force` does not bypass it.
- Every refusal and every skip names its cause, so it never has to be
  guessed: blocker IDs and active subtask IDs as separate lists, and the
  path of an unusable file.
- `show ID` states readiness without claiming, on a `Ready:` line just
  before the `Documents:` line: `Ready: yes`, or `Ready: no — ` followed by
  every reason, in this order and joined by `; `: `status S` when the
  issue is not `open`, `deferred until DATE` (with `by ancestor ID` when
  inherited), `unresolved REF` for a missing or unusable blocker, ancestor,
  or descendant (followed once by a pointer to `jig check`), where REF
  names the role and the cause: `blocker ID (missing)`, `ancestor ID
  (missing)`, `blocker ID (unusable PATH)`, `ancestor ID (unusable PATH)`,
  or `subtask PATH (unusable)`, with PATH relative to the project root;
  JSON `unresolved` carries the same REF strings,
  `blocked by IDS`, and `waiting for subtasks IDS`. `ready`, `claim`, and
  `show` derive this from one rule, and `claim` refuses with the same
  reasons. JSON `show` adds `readiness`: `{ready, deferred_until,
  deferred_by, unresolved, blockers, subtasks}` with `null` or `[]` where a
  reason is absent.
- `list` and `tree` never fail as a whole because of one bad issue file
  (unreadable, conflicted, duplicated, or in the wrong directory). They
  list every readable issue, print one warning per skipped file to stderr
  pointing to `jig check`, and exit 0 with JSON on stdout in its normal
  shape. `tree` of an ID whose own file is unusable fails, and `show` of
  such an ID fails as before. An issue with inconsistent fields (see
  Registry Check) is readable: `list`, `tree`, and `show` include it, and
  `list` and `tree` warn about it; `ready` skips it and `claim` refuses it.
- `claim ID` rechecks eligibility under the lock before switching to
  `claimed`. It refuses `proposed` issues, also with `--force`.
  `--by` is an optional label, not identity or authentication.
  Do not infer a label from the account or machine. `ready` and `show` never
  reserve work. Agents read the issue and start work only after `claim`
  succeeds. There is no separate read receipt.
- `claim ID --force` bypasses only existing unfinished blockers and active
  descendants (any active status: `proposed`, `open`, `claimed`, or
  `review`), not the issue's own
  `proposed` status, deferral, malformed or missing dependencies, or a
  claim held by someone else. It
  does not change dependencies or make the issue appear in `ready`. It
  records one short history entry with the time, supplied assignee (if any),
  and bypassed blocker and subtask IDs. No `--reason` is required. Agents use `--force`
  only at a person's explicit instruction for that issue.
- `unclaim` releases a `claimed` issue back to `open`. `review` submits
  the work for verification, preserves the assignment, and does not unblock
  dependents. `rework ID --reason TEXT` returns an issue in `review`
  to `claimed` with the same assignment. There may be any number of
  review rounds; none overwrites a previous round. Reviewers can use `note`
  without changing the status.
- A human may `close ID --reason TEXT` from any active status, with or
  without a code change or prior review, including a rejected `proposed`
  issue. Fixed, duplicate, rejected, obsolete, and working-as-designed
  issues use the same `closed` status.
  Agents submit work for review rather than close issues; this is a workflow
  rule, not an access-control boundary.
- `reopen ID --reason TEXT` moves a closed issue back to `open/`, clears its
  current closing date and assignment, and keeps the same ID and history.
  It returns to `ready` only if eligible. Unfinished dependents that require
  this issue are blocked again.
- A closed issue records the release it shipped in, in `release`. Only
  closed issues carry it. `release NAME` is a human action: it sets
  `release` to `NAME` on every closed issue whose `release` is `null` and
  prints their IDs; a later run stamps only issues closed since. People
  correct one issue with `update ID --release NAME`, for example after a
  skipped release; it works only on closed issues and records the old and
  new value in history. `reopen` clears `release` and records the former
  value in its history entry. `list --release NAME` lists the closed
  issues of one release. Planning what a release must contain uses an
  ordinary issue with dependencies; there is no target-release field.
- `update` is non-interactive and changes only supplied fields: title,
  priority, description, or the release of a closed issue. It does not change
  lifecycle state or overwrite the history or notes. There is no interactive
  `edit`. For both `add` and `update`, `--body TEXT` takes literal text and
  `--body -` reads stdin. Omission does not read stdin; `update` then leaves
  the description alone. There is no `--body-file`.

### History, Notes, And Manual Summaries

- `## History` records approvals, revisions, review submissions, rework
  requests, closure, reopening, release corrections, and forced claims. Append
  entries with a UTC date and time down to seconds. Keep existing entries even
  when the same transition occurs several times. Timestamps are not used to
  pick winners after synchronization.
- `--reason` says why someone closed, reopened, revised, or returned an
  issue for rework. Record it in the history entry; do not duplicate it in
  YAML.
  Closing and reopening update the current state and closing date in YAML.
- An agent can add a manual summary with `review ID --summary TEXT`. Keep
  it with that round's history entry. `close ID --reason TEXT` also accepts
  optional `--summary TEXT`. Put it in `## Closing Summary` before the full
  description for easy browsing without the CLI. Keep the reason in
  `## History` and the full description, notes, and history in `.jig/closed/`
  whether or not a summary was supplied. Show the summary with the full
  issue, not in `start`, `ready`, or default lists. No summary-only view
  is needed in v1.
- Closing summaries are additive. Jig never replaces or shortens
  existing issue content when closing or archiving an issue.
- `jig note ID TEXT [--by NAME]` appends to `## Notes` in the issue file, with
  UTC date and time and an author if supplied. A new note is one line of at
  most 500 characters. Reject longer or multiline input without truncation.
  The limit does not affect imported text or the total size of an issue.
  Notes do not change status and are not duplicated in history.

### Issue Documents

- Long material that belongs to one issue — analyses, review or audit
  reports, measurements, instructions for an implementer — lives in
  `.jig/docs/KEY/`, where `KEY` is the issue's key (dotted subtask keys
  included), so renumbering never moves documents. Any files and
  subdirectories may be stored there; Markdown is
  the convention. The description references the documents that matter;
  it stays the contract.
- `jig` has no command that creates, edits, or removes documents; people
  and agents write the files directly. `init` does not create `docs/`, and
  a missing `docs/` is not a problem.
- The directory belongs to the key, not to a status or a number: closing,
  reopening, renumbering, and every other change leave `.jig/docs/KEY/`
  where it is.
- `show ID` ends with a `Documents: .jig/docs/KEY/` line naming the
  directory even when it does not exist yet, followed by the regular files
  under it, recursively, as paths relative to the project root (for example
  `.jig/docs/5a3f35/analysis.md`), sorted, or by `(none)` on the same line
  when there are none. It does not print their contents. JSON adds
  `docs_dir` (the directory path) and `docs`, an array of the file paths
  (`[]` when there are none).
- A broken document location never hides the issue. When `.jig/docs` or
  `.jig/docs/KEY` is not a real directory, or part of it cannot be read,
  `show` still prints the issue and its `Documents:` line (JSON with the
  files it could list), warns on stderr with the path relative to the
  project root and a pointer to `jig check`, and exits 0. `check` reports
  the location.

### Agent Context And Project Memory

- `start` gives short workflow instructions. `ready`, `list`, and `tree`
  show compact views, including in JSON. `show ID` opens one full issue,
  regardless of whether it is in `open/` or `closed/`. Keep results,
  decisions, and references to evidence, not session transcripts or logs.
- `.jig/memory.md` is a human-readable store of current facts, not a cache
  or append-only log. `remember TOPIC TEXT` updates one named fact, `recall
  TOPIC` reads one fact, and `recall` with no topic lists only the names.
  No automatic key generation or `--key` is needed. `start` never loads the
  full memory automatically.

### Agent Instructions Block

- The block is delimited by `<!-- BEGIN JIG v:N hash:H -->` and
  `<!-- END JIG -->`, where `N` is the block format version and `H` is a
  short hash of the text between the markers as `jig` wrote it.
- `init` appends the block when no markers exist. When the markers exist
  and the hash matches the text, the block is unmodified: `init` replaces
  an older version with the current one and leaves a current one alone.
  When the hash does not match, a person edited the block: `init` leaves
  it untouched and reports that it is customized. A block of a newer
  version than this `jig` supports is left untouched. Malformed or
  duplicated markers refuse the edit, as before.
- `check` reports a missing block and an outdated unmodified block (hint:
  `jig init`). A customized block of the current version is not a problem.
  A customized block of an older version is reported with the hint to
  remove the block, run `jig init`, and reapply the local edits. A block of
  a newer version is reported with the hint to update `jig`.
- Whenever `init` leaves an existing block untouched in a state that
  `check` reports, `init` prints the same problem and hint, prefixed with
  `AGENTS.md:`, and still exits 0. A refusal also names `AGENTS.md`.
- The block text of a released version `N` never changes. Any change to the
  text, apart from the substituted `jig` version, comes with `N + 1`.
- The block text tells agents, in this order:
  - the project tracks work with `jig` and keeps no task lists in other
    Markdown or TODO files; the agent's built-in todo or plan tool holds
    only the steps of the current task, and anything that must outlive the
    session goes to `jig`;
  - `jig` comes from `https://github.com/EntropyLogix/Jig`; if it is not
    installed, ask a person to install it with
    `cargo install --locked --git https://github.com/EntropyLogix/Jig --tag vX.Y.Z`,
    where `X.Y.Z` is the version of `jig` that wrote the block;
  - run `jig start`; `jig ready` lists approved, unblocked work; `jig show ID`
    reads one issue; an issue authorizes only its described scope; refer to
    issues by the handle `jig` prints, in commands and in messages, and a
    person may give only the number;
  - claim before working with `jig claim ID --by NAME` using the agent's
    name; `--force` only on a person's explicit instruction for that issue;
  - submit with `jig review ID --summary TEXT`; approving, revising,
    returning for rework, and closing are for people unless a person
    explicitly says otherwise for that issue;
  - work found along the way becomes `jig add TITLE --body TEXT` (a
    proposal, not work) plus a `jig note` on the issue where it was found;
  - the issue description is the current contract: rewrite it with
    `jig update ID --body -` as decisions change instead of appending, and
    write long analyses and reports directly in the `Documents:` directory
    from `jig show ID`, not in other directories;
  - a part of an issue's work becomes a subtask (`jig add TITLE --parent ID
    --body TEXT`), separate work that must happen first becomes a blocker
    (`jig dep add`); a parent is ready only after its subtasks close; `jig dep add ID BLOCKER`
    means ID waits for BLOCKER, and a closed blocker stops blocking without
    removing the link; when the agent's issue needs other work first, it
    adds that work as a subtask or as a blocker, leaves a `jig note`, and
    runs `jig unclaim ID`; it removes a dependency with `jig dep rm` only to
    undo its own mistake, with a `jig note`;
  - when stopping before the work is done, leave a `jig note` and run
    `jig unclaim ID` if the agent will not continue;
  - read project facts with `jig recall [TOPIC]`, record durable project
    facts with `jig remember TOPIC TEXT`, and keep personal preferences in
    the agent's own memory;
  - if `jig ready` warns or lists nothing, run `jig check` and report to a
    person; never remove dependencies or approve proposals to get unblocked.
- The `jig start` workflow text states the same rules.

### Registry Check

- `jig check [--json]` reports every integrity problem in one run instead
  of stopping at the first: unreadable issue files, a file name that does
  not match the ID, a status that does not belong in its directory,
  duplicate IDs, merge conflict markers, missing blockers or ancestors,
  dependency cycles, inconsistent fields (for example a `proposed` or
  `open` issue with `claim_id` or `assignee`, a `claimed` issue without
  `claim_id`, or `closed_at`, `release`, or a closing summary on an active
  issue), a pending or mismatched move record, an unreadable `memory.md`,
  a missing or invalid `.jig/config.json`, a number used by two issues,
  unexpected files in `open/` or `closed/`, an entry in `.jig/docs/` that
  is not a directory or is not named after an existing issue key (hint:
  rename it to the issue key or remove it), and the legacy
  `/.*.tmp-*` rule in `.jig/.gitignore`.
- Unknown legacy metadata is not a problem: a `closed` issue may have
  `closed_at: null`, and an issue in `review` may have no `assignee` or
  `claim_id` if it was never claimed. `rework` of such an issue assigns a
  new `claim_id` and invents no assignee.
- Each problem names the file, what is wrong, and a concrete way to
  resolve it (a `jig` command or the manual edit or Git action). JSON is an
  array of `{path,problem,hint}`. Exit 0 when clean, 1 otherwise.
- `check` only reports. It never modifies, repairs, or finishes a pending
  move; every resolution is an explicit human decision. Agents run `check`
  when `ready` warns and report the result to a person instead of removing
  dependencies or editing files to get unblocked.

### MCP Server

- `jig mcp` is a Model Context Protocol server over stdio in the same
  binary. The client starts it and ends it with the session; there is no
  daemon, port, or background service. It is optional and meant for
  agents without a shell or for clients where a person approves tool calls;
  an agent with a shell works through the CLI, the `AGENTS.md` block, and
  the hooks, which cost less context than tool schemas.
- The server finds the registry exactly like the CLI, by walking up from
  its working directory, on every tool call. It does not use MCP roots and
  takes no project argument; the client starts it in the project. It keeps
  no registry state between calls: each call reads the files again and
  takes `.lock` like the CLI, so CLI use, hand edits, and other servers on
  the same registry are seen immediately.
- Every command except `mcp` itself, `--help`, and `--version` is a tool
  with the command's name (`dep add` and `dep rm` become `dep_add` and
  `dep_rm`), including `init` and `start`. Tool arguments are typed JSON
  fields mirroring the command's arguments and options (`body` is the text
  itself; there is no stdin). A tool call runs the same command code as
  the CLI: validation, refusals, messages, and effects are identical, and
  a text value that looks like an option (a title `--force`) is still
  text. The result is one text content item: for read tools the CLI's
  `--json` output, for write tools the CLI's stdout. Warnings the CLI
  prints on stderr are appended to the result text. A refused or failed
  command returns `isError: true` with the CLI's error message; JSON-RPC
  errors are reserved for protocol faults (malformed message, unknown
  method or tool).
- Read tools (`ready`, `show`, `list`, `tree`, `check`, `recall`, `start`)
  carry `readOnlyHint: true`. The description of every tool for a person's
  decision (`approve`, `revise`, `rework`, `close`, `reopen`, `release`,
  `init`) starts with "A person's decision:". Jig enforces no permissions;
  the client's per-tool approval is where a person confirms such a call.
- The server implements MCP revision `2025-06-18` and answers
  `initialize` with that `protocolVersion` whatever the client requested,
  leaving the decision to the client as the protocol prescribes.
  `initialize` returns as `instructions` the `jig start` workflow text
  followed by one MCP-only paragraph: use these tools instead of running
  `jig` in the shell, each tool mirrors the command of the same name, and a
  person's refusal of a tool call is final and is not retried through the
  shell. Only agents connected to the server see that paragraph; the
  `AGENTS.md` block and `jig start` do not mention MCP. `initialize` also
  returns `serverInfo` with name `jig` and the package version. The server answers
  `ping`, accepts and ignores notifications it does not use, rejects
  unknown methods with `-32601`, and writes nothing but JSON-RPC messages
  to stdout. `init` does not register the server with any client; the
  README shows the entries for Claude Code, Codex, and EntropyForge.

## File And CLI Details

- **Reopening a summarized issue.** Preserve the former closing summary
  verbatim in `## History` and remove the top section on `reopen`. That way
  the active issue does not present an old outcome as its current summary.
- **On-disk format.** Fixed, one-line YAML fields: `id` (the key), `number`
  (a positive integer on a top-level issue, `null` on a subtask),
  `title`, `status`,
  `priority`, `parent`, `depends_on`, `assignee`, `defer_until`, `created_at`,
  `updated_at`, `closed_at`, and `release`. Add a nullable `claim_id` to
  distinguish independent claims, even with identical or absent assignee
  labels. Jig always writes the fields in this order and reads them in any
  order, so a hand edit or a resolved merge conflict that reorders lines
  stays valid, and the next write restores the order. Use `null` for unset
  scalar fields and `[]` for no dependencies.
  `priority` is never `null`: it is always an integer from 0 to 4, and any
  other value, `null` included, makes the file invalid like any other invalid
  field. `check` names the field and hints to set it to 0–4. `release` is
  `null` or a non-empty single-line string without surrounding whitespace;
  every file has the key, and a file without it is invalid like one without
  any other fixed key. Reserve
  top-level `## Closing Summary`, `## History`, and `## Notes` for the managed
  sections. `.jig/config.json` holds the per-project settings; today only
  `prefix`, and an unknown key makes it invalid.
- **Dates and readiness.** New timestamps use UTC. `defer_until` is a UTC
  calendar date; the issue is eligible on that day. Sort `ready` by priority
  (0 highest; new issues default to 2), then creation date, then ID. `list`
  without a filter shows all active statuses. Issue references follow
  Files, IDs, And Concurrent Work.
- **Read JSON.** `id` is always the handle and `key` the key; `parent`,
  `depends_on`, and readiness lists hold handles. `ready` and `list` return
  arrays of `{id,key,title,status,priority,assignee}`. `tree` returns a
  recursive `{id,key,title,status,children}` object. `show` returns the
  issue's metadata (excluding internal `claim_id`)
  and `body`, `history`, `notes`, and `closing_summary` as separate Markdown
  values (`closing_summary` is `null` when absent), plus `readiness` (see
  Issue Workflow) and `docs_dir` and `docs` (see Issue Documents). `start`
  returns
  `{workflow}`; `recall` returns an array of topic names or `{topic,value}`
  for one topic. Successful output goes to stdout; errors go to stderr
  with a nonzero exit code and no partial JSON.
- **Read-only access.** Read commands (`start`, `ready`, `show`, `list`,
  `tree`, `recall`, `check`) work in a checkout without write access to
  `.jig/`; they do not need to create or write `.jig/.lock`. Where they can
  write, the non-`check` read commands may finish an interrupted move as
  before; where they cannot, a pending move is reported as an error.
- **Grouped lists.** Only `jig list` without arguments (apart from
  `--json`) is an overview: its text output groups the active issues by
  status in the order `review`, `proposed`, `claimed`, `open`, so the work
  waiting for a person comes first. Each group starts with a `STATUS (N)`
  header line, followed by its rows indented by two spaces, in the usual
  `list` order. Empty groups are omitted; when nothing matches, the output
  is empty. Any argument (`--status`, `--release`, `--sort`) makes it a
  query with a flat list. A row has the same columns in both forms,
  including the status. JSON is never grouped: it stays a flat array in
  the usual order. `--status` takes one of `proposed`, `open`, `claimed`,
  `review`, `closed`, or `all`; there is no `active` value, since the
  default already shows the active issues.
- **Help text.** `jig --help` groups commands by purpose (browsing,
  creating and editing, agent work, human decisions, memory, setup),
  indents commands by two spaces, and aligns descriptions in one column.
  A command whose syntax does not fit puts its description on the next
  line in that column. Options shared by several commands (`--json`,
  `-p`/`--priority`, the status values, `--`) are explained once, not on
  every command. No line is longer than 80 columns.
- **Chronological lists.** `list --sort oldest|newest` sorts by `created_at`
  regardless of priority, keeping unknown dates last and breaking ties by ID.
  Text includes the stored date (`-` when missing); JSON changes only row
  order. Default `list` and `ready` ordering are unchanged.
- **Interrupted file moves.** Stage a local, unversioned move record under
  the registry lock before changing directories. Finish the same operation
  on the next invocation after an interruption. If the files no longer
  match the recorded operation, report the conflict instead of guessing.
  A normal command never observes a half-completed move as a valid issue.
- **Legacy metadata.** Preserve dates known only to the day without inventing
  hours. Use `null` for unknown timestamps, preserving the original wording
  in the body, and sort unknown dates after known ones. An importer sets a
  missing or unrecognized priority to 2.
- **Platform and dependencies.** Start with macOS and Linux. Use Rust's
  standard file locking, with no extra locking dependency. Choose and
  approve any external argument, serialization, or hashing libraries before
  adding them. The only approved dependency is pinned `serde_json`
  (`=1.0.154`, `arbitrary_precision`), used for editing agent
  configuration during `init` and for JSON-RPC messages in `jig mcp`; its
  license texts live in `licenses/`.

## Acceptance Checks

- A new registry numbers issues from `B1` (or the configured prefix);
  subtasks show their root's number; closed issues keep their numbers; a
  changed prefix changes every handle; no output outside paths contains a
  key of an issue whose number is readable; a duplicated number is warned,
  refused when used, reported by `check`, and fixed with `update --number`.
- Initialize and use a registry without Git. Discover it from a subdirectory
  without silently picking a different checkout.
- Race two real `claim` processes against one registry and verify exactly
  one winner. Verify that forced claims bypass only the agreed blockers.
- Exercise multiple review rounds, closure without a code change, reopening,
  and recovery after interruption. Verify that neither content nor history
  is lost and that dependencies still control readiness.
- Check one-line note bounds, both `--body` input modes, compact JSON, and
  selective memory loading. Verify that read commands never dump unrelated
  bodies into agent context.
- Walk the full lifecycle: `add` creates `proposed`, which neither `ready`
  nor `claim` (also with `--force`) accepts; `approve` makes it ready;
  `revise` returns it to `proposed`; `add --approved` starts at `open`;
  `rework` returns `review` to `claimed`; a rejected proposal closes.
- Run `init` against existing `.claude/settings.json` and
  `.codex/hooks.json` with custom key order and formatting, and verify that
  every byte outside the inserted hook group is unchanged.
- Seed a missing blocker and a missing parent: `ready` lists the other
  eligible issues, warns on stderr, and exits 0; `claim` of the affected
  issue fails. Add a file with merge conflict markers: `ready`, `list`, and
  `tree` still list every other issue, warn, and exit 0.
- Run `init` on a project with an older unmodified block, a customized
  block, a newer block, and no block: the first is upgraded, the second and
  third are left byte-for-byte, the fourth is appended; `init` and `check`
  report exactly the cases described in Agent Instructions Block.
- The full test suite builds and passes on the declared minimum Rust
  version as well as on current stable.
- `add` without `-p` stores priority 2. An issue file with
  `priority: null` or a value outside 0–4 is reported by `check` as
  invalid with a hint naming the field, is skipped with a warning by
  `ready`, `list`, and `tree`, and is refused by `show` and every write.
- `release 1.0` stamps every closed issue with `release: null` and prints
  their IDs; a second `release 1.0` stamps nothing. After closing one more
  issue, `release 1.1` stamps only it. `update ID --release 1.0` moves it
  to `1.0` with a history entry; on an active issue it is refused.
  `reopen` clears `release` with the former value in history. `list
  --release 1.0` lists exactly the stamped issues. `check` reports
  `release` on an active issue and a file without the `release` key.
- A parent with an active subtask is absent from `ready` and refused by
  `claim`; `claim --force` takes it and names the bypassed subtasks in
  history. Once every subtask is closed, the parent is ready. A closed
  blocker keeps its `depends_on` link; reopening it blocks the dependent
  again. The refusal, the forced-claim history entry, and `show` list
  blockers and subtasks separately.
- A parent whose subtask file is unusable (an invalid field, and
  separately a file that cannot be parsed) is skipped by `ready` with a
  warning naming the file, refused by `claim` also with `--force`, and
  shown by `show` as `Ready: no — unresolved …`.
- `show` gives `Ready: yes` for a ready issue and, for one issue that is
  deferred, blocked, and waiting for a subtask at once, `Ready: no` with all
  three reasons in the agreed order; JSON `readiness` carries the same.
- `show` of an issue without documents prints `Documents: .jig/docs/KEY/
  (none)` and JSON `docs: []` with `docs_dir`. Put files, including one in
  a subdirectory, in `.jig/docs/KEY/`: `show` lists their paths in text and
  in JSON `docs`; `close` and `reopen` leave
  them in place. A directory in `.jig/docs/` named after no issue and a
  plain file directly in `.jig/docs/` are reported by `check`; a registry
  without `docs/` is clean. With `.jig/docs/KEY` replaced by a file, `show`
  prints the issue, warns with the relative path, and exits 0.
- With issues in every status, `list` prints the non-empty groups in the
  order `review`, `proposed`, `claimed`, `open` with counts and rows in the
  flat-list format; `list --status all`, `--status open`, `--release`, and
  `--sort` print flat lists; `--status active` is rejected; JSON output is
  the same flat array as before.
- `jig --help` names every command, has no line longer than 80 columns,
  and indents every command line.
- Seed one instance of every `check` problem: a single `check` run reports
  all of them with hints and changes no byte in the registry.
- Remove write access from a freshly cloned registry without `.lock`: all
  read commands succeed.
- `jig mcp` started in a subdirectory of a project completes `initialize`
  with `instructions` equal to the `jig start` text plus the MCP paragraph;
  the `AGENTS.md` block and `jig start` contain no mention of MCP. `tools/list` names exactly
  the commands of the dispatcher (minus `mcp`, plus `dep_add`/`dep_rm`),
  read tools carry `readOnlyHint`, and the decision tools' descriptions
  start with "A person's decision:". For every tool, a scripted call gives
  the same result text and the same registry bytes as the CLI on an
  identical registry, including a refusal (`isError`) and a warning. A
  title `--force` stays a title. A CLI change made between two tool calls
  is seen by the second. stdout carries only JSON-RPC; `ping` is answered
  and an unknown method gets `-32601`.
