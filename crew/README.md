# jig-crew

An optional review loop for coding agents on top of Jig. A developer
implements an issue, one or more auditors check it independently, and a
review lead merges their reports into the single decision the developer
works from. Jig itself is unchanged: crew uses statuses, the history, and
the issue's documents directory.

## The loop

| Step | The person runs | Result |
| --- | --- | --- |
| 1 | `/jig-crew dev ID claude-dev` | the issue is claimed, implemented, handed over with `jig review` |
| 2 | `/jig-crew audit ID codex-1` (any number, in parallel) | `audit-codex-1.md` |
| 3 | `/jig-crew lead ID` (interactive) | `audit-lead.md`, `decision.md` |
| 3a | optional: `/jig-crew audit ID claude-2 --cross-check` | a report on the lead's decision |
| 4 | the lead runs `jig rework ID` | back to step 1; the developer reads only `decision.md` |
| 5 | the person runs `jig close ID --reason ...` | done |

In Codex, start a role with `$jig-crew dev ID claude-dev` and so on.

A round is one `jig review`; its number is the count of review submissions
in the issue's history. Documents live in
`.jig/docs/ID/review/round-NN/`. While an issue is in `review` its code is
frozen: the developer records a fingerprint of the working tree in the
handoff, and auditors and the lead are warned when the code no longer
matches it.

Labels name agents (`claude-dev`, `codex-1`). They are required by crew
and map to Jig's `claim --by`; `lead` is reserved for the review lead.
Invoking the lead for an issue is the person's permission for that one
`jig rework`; closing an issue is always the person's decision.

Instructions are in English. Agents talk to the person, and write their
documents, in the person's language.

## Install

Requirements: `jig`, `git`, `jq`, and a POSIX shell. The installer checks
them and says how to get a missing one; it installs nothing else.

```sh
crew/install.sh              # Claude Code and Codex
crew/install.sh --claude     # only Claude Code
crew/install.sh --codex      # only Codex
crew/install.sh --uninstall  # remove (combine with --claude or --codex)
```

The skill is copied to `${CLAUDE_CONFIG_DIR:-~/.claude}/skills/jig-crew`
and `~/.agents/skills/jig-crew`; rerun the installer after updating the
checkout. It touches no settings or hooks. The skill runs only when invoked
explicitly (`disable-model-invocation` for Claude Code,
`agents/openai.yaml` for Codex).

## Layout

- `SKILL.md`: entry point for both clients.
- `bin/crew`: checks the issue's status and the label, computes the round,
  creates the round's files from `templates/`, verifies the code
  fingerprint, and prints the instructions of exactly one role.
- `roles/`: `common.md` for every role, `auditing.md` for auditors and the
  lead, and one file per role.
- `templates/`: `handoff.md`, `audit.md`, `decision.md`.

`bin/crew` can also be run directly: `crew dev ID LABEL`,
`crew audit ID LABEL [--cross-check]`, `crew lead ID`, `crew fingerprint`.
