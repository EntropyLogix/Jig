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
- Keep existing identifiers such as `B440`. New top-level issues use `B-`
  followed by exactly six lowercase hex digits. There is no hidden longer ID
  or variable abbreviation. Generate the hex from a hash of a random local
  installation identifier and fresh system randomness. The installation
  identifier stays out of the project; do not use the IP, MAC, or hostname.
  Check both directories under the lock and retry on a local collision.
- Subtasks use dotted IDs (`B-a3f8c1.1`, `B-a3f8c1.1.1`). Allocate the next
  number after the largest direct child under the lock, including closed
  children. A closed child does not free its number.
- Writes use a local lock and atomic file replacement. Processes using the
  same `.jig/` cannot both claim the same issue. Independently edited copies
  may conflict after synchronization; we accept that rather than partition
  the queue. Do not silently overwrite an issue, renumber one side, or pick
  the newest claim by timestamp. When a conflict is present in the files,
  surface it for explicit resolution.
- Humans may read and edit the Markdown directly, provided no one else is
  writing the same issue at the time. Agents make changes through `jig`.

### Issue Workflow

- The lifecycle is `proposed` → `open` → `claimed` → `review` → `closed`.
  `proposed` means the contract is not approved yet and the issue is not
  work. `add` creates a `proposed` issue, including subtasks created with
  `--parent`; `add --approved` creates it directly as `open`. Agents create
  issues without `--approved`; approving is a human action.
- `approve ID` moves a `proposed` issue to `open` and records a history
  entry. `revise ID --reason TEXT` returns an `open` issue to `proposed`
  when its approved contract has to be reworked before anyone starts on
  it. Both are workflow rules for humans, not an access-control boundary.
  The status applies to one issue only: a `proposed` parent does not hide
  its subtree, and approving a parent does not approve its children.
- `ready` shows only `open` issues that are not deferred and whose blockers
  are all closed. A missing blocker is not a closed blocker. `depends_on`
  records blockers on the waiting issue; `dep add` rejects cycles. Parenthood
  alone is not a dependency. Deferring a parent hides the entire subtree
  without editing the children; their own blockers and deferrals still apply.
- An issue whose blocker or ancestor is missing or unreadable is not ready.
  `ready` skips it, still lists every other eligible issue, prints one
  warning per skipped issue to stderr naming the unresolved reference and
  pointing to `jig check`, and exits 0. JSON on stdout keeps its normal
  shape. `claim` of such an issue fails as before.
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
- `claim ID --force` bypasses only existing unfinished blockers (any active
  status: `proposed`, `open`, `claimed`, or `review`), not the issue's own
  `proposed` status, deferral, malformed or missing dependencies, or a
  claim held by someone else. It
  does not change dependencies or make the issue appear in `ready`. It
  records one short history entry with the time, supplied assignee (if any),
  and bypassed blocker IDs. No `--reason` is required. Agents use `--force`
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
- `update` is non-interactive and changes only supplied fields: title,
  priority, or description. It does not change lifecycle state or overwrite
  the history or notes. There is no interactive `edit`. For both `add` and
  `update`, `--body TEXT` takes literal text and `--body -` reads stdin.
  Omission does not read stdin; `update` then leaves the description alone.
  There is no `--body-file`.

### History, Notes, And Manual Summaries

- `## History` records approvals, revisions, review submissions, rework
  requests, closure, reopening, and forced claims. Append entries with a UTC
  date and time down to seconds. Keep existing entries even when the same transition occurs
  several times. Timestamps are not used to pick winners after synchronization.
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
    Markdown or TODO files;
  - `jig` comes from `https://github.com/EntropyLogix/Jig`; if it is not
    installed, ask a person to install it with
    `cargo install --locked --git https://github.com/EntropyLogix/Jig --tag vX.Y.Z`,
    where `X.Y.Z` is the version of `jig` that wrote the block;
  - run `jig start`; `jig ready` lists approved, unblocked work; `jig show ID`
    reads one issue; an issue authorizes only its described scope;
  - claim before working with `jig claim ID --by NAME` using the agent's
    name; `--force` only on a person's explicit instruction for that issue;
  - submit with `jig review ID --summary TEXT`; approving, revising, and
    closing are for people unless a person explicitly says otherwise for
    that issue;
  - work found along the way becomes `jig add TITLE --body TEXT` (a
    proposal, not work) plus a `jig note` on the issue where it was found;
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
  `claim_id`, or `closed_at` or a closing summary on an active issue), a
  pending or mismatched move record, an unreadable
  `memory.md`, unexpected files in `open/` or `closed/`, and the legacy
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

## File And CLI Details

- **Reopening a summarized issue.** Preserve the former closing summary
  verbatim in `## History` and remove the top section on `reopen`. That way
  the active issue does not present an old outcome as its current summary.
- **On-disk format.** Fixed, one-line YAML fields: `id`, `title`, `status`,
  `priority`, `parent`, `depends_on`, `assignee`, `defer_until`, `created_at`,
  `updated_at`, and `closed_at`. Add a nullable `claim_id` to distinguish
  independent claims, even with identical or absent assignee labels. Use
  `null` for unset scalar fields and `[]` for no dependencies. Reserve
  top-level `## Closing Summary`, `## History`, and `## Notes` for the
  managed sections.
  No project `config.json` until a real per-project setting is needed.
- **Dates and readiness.** New timestamps use UTC. `defer_until` is a UTC
  calendar date; the issue is eligible on that day. Sort `ready` by priority
  (0 highest; new issues default to 2), then creation date, then ID. `list`
  without a filter shows all active statuses. Allow an exact six-hex ID
  without `B-` as in the source specification, but no shorter prefix match.
- **Read JSON.** `ready` and `list` return arrays of `{id,title,status,
  priority,assignee}`. `tree` returns a recursive `{id,title,status,children}`
  object. `show` returns the issue's metadata (excluding internal `claim_id`)
  and `body`, `history`, `notes`, and `closing_summary` as separate Markdown
  values (`closing_summary` is `null` when absent). `start` returns
  `{workflow}`; `recall` returns an array of topic names or `{topic,value}`
  for one topic. Successful output goes to stdout; errors go to stderr
  with a nonzero exit code and no partial JSON.
- **Read-only access.** Read commands (`start`, `ready`, `show`, `list`,
  `tree`, `recall`, `check`) work in a checkout without write access to
  `.jig/`; they do not need to create or write `.jig/.lock`. Where they can
  write, the non-`check` read commands may finish an interrupted move as
  before; where they cannot, a pending move is reported as an error.
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
  hours. Use `null` for unknown timestamps or priorities, preserving the
  original wording in the body. Sort unknown priority or date after known
  values on its respective sort key.
- **Platform and dependencies.** Start with macOS and Linux. Use Rust's
  standard file locking, with no extra locking dependency. Choose and
  approve any external argument, serialization, or hashing libraries before
  adding them. The only approved dependency is pinned `serde_json`
  (`=1.0.154`, `arbitrary_precision`), used solely for editing agent
  configuration during `init`; its license texts live in `licenses/`.

## Acceptance Checks

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
- Seed one instance of every `check` problem: a single `check` run reports
  all of them with hints and changes no byte in the registry.
- Remove write access from a freshly cloned registry without `.lock`: all
  read commands succeed.
