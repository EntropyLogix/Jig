# blt-crew

An optional review loop for coding agents on top of Billet. An architect turns
a proposal into a contract, a developer implements it, one or more auditors
check it independently, and a lead auditor merges their reports into the
single decision the developer works from. Billet itself is unchanged: crew
uses statuses, the history, notes, and the issue's documents directory.

## The loop

| Step | The person runs | Result |
| --- | --- | --- |
| 0 | optional: `/blt-crew architect ID` on a proposal, then `blt approve ID` | the contract, `architecture.md`, subtasks |
| 1 | `/blt-crew dev ID claude-dev` | the issue is claimed, implemented, handed over with `blt review` |
| 2 | `/blt-crew audit ID codex-1` (any number, in parallel) | `audit-codex-1.md` |
| 3 | `/blt-crew audit ID --lead` (interactive) | `audit-lead.md`, `decision.md` |
| 3a | optional: `/blt-crew audit ID claude-2 --cross-check` | a report on the lead's decision |
| 4 | the lead runs `blt rework ID` | back to step 1; the developer reads only `decision.md` |
| 5 | the person runs `blt close ID --reason ...` | done |

In Codex, start a role with `$blt-crew dev ID claude-dev` and so on.

`/blt-crew status ID` shows every round so far: whether the developer
handed it over, each audit report with its verdict, and the decision. A
report or decision counts as finished once its `## Verdict` section is
filled in; until then it shows as in progress. Running a role again with a
label that already has a report reopens that report and prints its verdict,
so the same command continues the same audit.

The architect works only on `proposed` issues and writes no code: it
rewrites the description as the contract, keeps the analysis in
`.blt/docs/KEY/architecture.md`, may split the work into subtasks, and
leaves a `blt note --by architect`. The developer, auditors, and lead see
the issue's and its ancestors' `architecture.md` under `Read:`; the
developer also sees whether the architect reviewed the issue, and on large
unreviewed work asks the person whether to hand it to the architect first.

A round is one `blt review`; its number is the count of review submissions
in the issue's history. Documents live in
`.blt/docs/KEY/review/round-NN/`. While an issue is in `review` its code is
frozen: the developer records a fingerprint of the working tree in the
handoff, and auditors and the lead are warned when the code no longer
matches it.

Labels name agents (`claude-dev`, `codex-1`). They are required by crew and
map to Billet's `claim --by`; `lead` and `architect` are fixed labels of
their roles. Invoking the lead for an issue is the person's permission for
that one `blt rework`; closing an issue is always the person's decision.

Instructions are in English. Agents talk to the person, and write their
documents, in the person's language.

## Install

Requirements: `blt`, `git`, `jq`, and a POSIX shell. The installer checks
them and says how to get a missing one; it installs nothing else.

```sh
crew/install.sh              # Claude Code and Codex
crew/install.sh --claude     # only Claude Code
crew/install.sh --codex      # only Codex
crew/install.sh --uninstall  # remove (combine with --claude or --codex)
```

The skill is copied to `${CLAUDE_CONFIG_DIR:-~/.claude}/skills/blt-crew`
and `~/.agents/skills/blt-crew`; rerun the installer after updating the
checkout. It touches no settings or hooks. The skill runs only when invoked
explicitly (`disable-model-invocation` for Claude Code,
`agents/openai.yaml` for Codex).

## Layout

- `SKILL.md`: entry point for both clients.
- `bin/crew`: checks the issue's status and the label, computes the round,
  creates the round's files from `templates/`, verifies the code
  fingerprint, and prints the instructions of exactly one role, or the
  status of the rounds.
- `roles/`: `common.md` for every role, `auditing.md` for auditors and the
  lead, and one file per role.
- `templates/`: `architecture.md`, `handoff.md`, `audit.md`, `decision.md`.

`bin/crew` can also be run directly: `crew architect ID`, `crew dev ID LABEL`,
`crew audit ID LABEL [--cross-check]`, `crew audit ID --lead`,
`crew status ID`, `crew fingerprint`.
