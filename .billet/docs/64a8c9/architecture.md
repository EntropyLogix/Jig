# Architecture: B4

## Goal

People and agents refer to issues the way they did with `bugs.md`: a short,
sequential, memorable handle such as `B494`, where a higher number is a
newer issue. Copies of a registry edited independently must still never
produce two issues with the same identity.

## Decisions

- **D1. Two identifiers with separate jobs.**
  - The **key** is the internal, random identity: file names, `parent`,
    `depends_on`, documents directories. It is never shown to people.
  - The **handle** is what people and agents read and write:
    `<prefix><number>`, and `<prefix><number>.<n>` for subtasks.
- **D2. The key is opaque.** Six lowercase hex digits that start with a
  digit and contain at least one letter (the generator retries other
  values), without a prefix: `5a3f35`, subtasks `5a3f35.2`. Files become
  `.jig/open/5a3f35.md`, documents `.jig/docs/5a3f35/`. A handle starts
  with a letter of the prefix, a bare number has no letter, and a key
  starts with a digit and has a letter, so an input token is never
  ambiguous, whatever the prefix.
- **D3. The number** is stored in a new required frontmatter field
  `number` on top-level issues (integer ≥ 1; `null` on subtasks). `add`
  assigns the largest number in the registry, open and closed, plus one,
  under the lock. A closed issue keeps its number; numbers are not reused.
  A subtask's handle is its root's handle plus the dotted part of its key;
  the subtask stores no number, so renumbering a root rewrites one file.
- **D4. The prefix is a project setting.** `.jig/config.json`
  `{"prefix": "B"}`, written by `jig init [--prefix P]`; this is the first
  per-project setting, so the file now exists. Default prefix `B`. A prefix
  is 1–8 ASCII uppercase letters, optionally followed by `-` (`B`, `AN`,
  `AN-`). Handles are derived, so editing the prefix changes every handle
  at once; `check` validates the file. Default `B` was confirmed by the
  person (as in EntropyFX).
- **D5. One way in conversation.** All human output uses handles: `list`,
  `show`, `ready`, `tree`, history and notes written by Jig, refusals,
  readiness reasons, `check` (except file paths). JSON `id` is the handle;
  JSON adds `key` and keeps `docs_dir` as a path. The MCP server follows
  the CLI. The only place a key appears is a path.
- **D6. Input.** Commands accept a handle, case-insensitively
  (`B494`, `b494`, `B494.2`); a bare number (`494`) as shorthand; and a key
  (`5a3f35`), for tools that read paths. The hex shorthand and the `B-`
  form are removed.
- **D7. Duplicate numbers.** Two copies can assign the same number between
  synchronizations. Identity stays intact (keys differ), so Jig detects
  the duplicate where it matters, without new checks on every command:
  - a handle that matches two issues is refused, naming both by title;
  - `list`, `ready`, and `tree`, which read every file anyway, warn once
    per duplicate;
  - `check` reports it with a hint;
  - a person fixes it with `jig update HANDLE --number N` (N unused, on a
    top-level issue; recorded in history). Agents never renumber.
- **D8. No legacy formats.** Jig drops `B440` and `B-xxxxxx` IDs. Existing
  registries are recreated or fixed by hand; the legacy migrator maps old
  numbers to `number` and generates keys. The profile stays `v1`.
- **D9. Agent block.** One rule: refer to issues only by the handle Jig
  prints; never write a key in messages or documents.
- **D10. Release.** Tag `v1.0.0` is removed now and set again on the
  commit that completes this issue. Afterwards the registries are
  recreated or fixed by hand in Jig, Forge, and AnomalyProtocol; Jig is
  removed from EntropyForge, which is being retired.

## Approach

- `issue.rs`: key validation and generation (D2), the `number` field (D3),
  handle formatting and parsing (D1, D6).
- `store.rs`: number assignment under the lock, key-based paths.
- New project config read with the registry (D4); `init --prefix`.
- `commands.rs`, `mcp.rs`, `check.rs`, `domain.rs`: every human-facing
  string renders handles (D5); input resolution with the ambiguity refusal
  (D6, D7); `update --number` (D7); duplicate-number problem kind (D7).
- `instructions.rs`: the block rule (D9); block hash changes, version
  stays `v:1`.
- SPEC, README, TESTING families: `ArgSpec` gets a handle text kind;
  the damaged-registry family gains the duplicate-number and bad-config
  kinds; parity, every-command and lifecycle families cover handles.
- crew: prints handles; documents paths come from `docs_dir`.

## Affected code

`src/issue.rs`, `src/store.rs`, `src/commands.rs`, `src/text.rs`,
`src/check.rs`, `src/domain.rs`, `src/mcp.rs`, `src/instructions.rs`,
`src/integration.rs` (init), `tests/cli.rs`, `SPEC.md`, `README.md`,
`crew/bin/crew`; migrator vendor and profile.

## Subtasks

None. The change is one contract across the codebase; splitting it would
leave intermediate states where some output shows keys.

## Risks and open questions

- Renumbering after a duplicate changes a handle, so handles quoted in
  free text (notes, commit messages, other documents) can go stale. This is
  rare and accepted.
- Paths still show keys (`.jig/docs/5a3f35/`). Accepted: a path is not an
  identifier people type.
