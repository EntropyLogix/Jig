---
name: billet-crew
description: Runs one role of the billet-crew loop (architect, dev, audit) for a Billet issue. Use only when the person invokes billet-crew explicitly.
argument-hint: "architect ID | dev ID LABEL | audit ID LABEL [--cross-check] | audit ID --lead"
disable-model-invocation: true
allowed-tools: Bash(~/.claude/skills/billet-crew/bin/crew:*)
---

!`~/.claude/skills/billet-crew/bin/crew $ARGUMENTS 2>&1 || true`

The block above is the output of crew for the arguments the person gave.

If there is no crew output above (Codex does not run it), run crew yourself
first, with the person's arguments exactly as given, and use its output.
In Codex the command is `~/.agents/skills/billet-crew/bin/crew`, for example
`~/.agents/skills/billet-crew/bin/crew audit B12 codex-1`. Do not use the
`~/.claude` path from the line above.

Follow the instructions in the crew output. They define your role, the
files you read, and the only files you write. If crew printed a `crew:`
refusal instead, tell the person that message and stop. If the person gave
no arguments, show them the usage crew printed and stop.

Reply in the language of the issue description, unless the person writes
to you in another language; English here does not change that.
