<!-- BEGIN BLT v:1 hash:6182831f357bfb1d -->
## Issue tracking with Billet

This project tracks work with `blt`. Do not keep task lists in other Markdown or TODO files. Your built-in todo or plan tool is only for the steps of the current task; record anything that must outlive this session in `blt`.
If `blt` is not installed, ask a person to install it: `cargo install --locked --git https://github.com/EntropyLogix/Billet --tag v1.0.0` (https://github.com/EntropyLogix/Billet).

- Run `blt start` for the workflow. `blt ready` lists approved, unblocked work; `blt show ID` reads one issue. An issue authorizes only its described scope. Refer to issues by the ID `blt` prints (for example `B12`, subtask `B12.3`), in commands and in messages; a person may give only the number (`12`).
- Claim before working: `blt claim ID --by NAME` with your agent name. Use `--force` only when a person explicitly authorizes it for that issue.
- Submit finished work with `blt review ID --summary TEXT`. Approving, revising, returning for rework and closing issues is for people unless a person explicitly tells you otherwise for that issue.
- Work found along the way: `blt add TITLE --body TEXT` (it starts as a proposal, not work) and `blt note ID TEXT` on the issue where you found it.
- The issue description is the current contract: rewrite it with `blt update ID --body -` as decisions change instead of appending; write long analyses and reports directly in the `Documents:` directory from `blt show ID`, not in other directories.
- Choose the relationship: a part of this issue's work is a subtask (`blt add TITLE --parent ID --body TEXT`); separate work that must happen beforehand is a blocker (`blt dep add ID BLOCKER`). A parent is ready only after its subtasks close; a closed blocker stops blocking without removing the link. If your issue needs other work beforehand, add it as a subtask or blocker, leave a `blt note`, and run `blt unclaim ID`; use `blt dep rm` only to undo your own mistake, with a note.
- Stopping before the work is done: leave a `blt note`; run `blt unclaim ID` if you will not continue.
- Read project facts on demand with `blt recall [TOPIC]`; record durable project facts with `blt remember TOPIC TEXT`. Keep personal preferences in your own memory.
- If `blt ready` warns or lists nothing, run `blt check` and report to a person; never remove dependencies or approve proposals to get unblocked.
<!-- END BLT -->
