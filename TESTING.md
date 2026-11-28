# Testing Jig

Tests protect the contract in `SPEC.md`. Each test names the contract it
protects, not the code it touches or the round in which a bug was found.
`cargo test --locked` must pass on stable and on the MSRV (`rust-version` in
`Cargo.toml`), on Linux and macOS.

## Levels

| Level | Where | Protects |
| --- | --- | --- |
| 1. Domain | `#[cfg(test)]` in `src/` | Pure rules: issue codec, IDs, readiness reasons, argument parsing, history formatting. No process, and no file system unless the rule is about files. |
| 2. Command contract | `tests/` through the binary | What a command does to a registry and what it prints: lifecycle, readiness, dependencies, text fields, damaged registries, `init`, memory, documents. |
| 3. Process | `tests/` through the binary | Behavior that needs a real process: exit codes, stdout/stderr split, closed pipes, stdin, `.lock`, concurrent claims, permissions, symlinks, discovery boundaries. |
| 4. Interface parity | `tests/` through the binary | MCP and CLI give the same result for every tool (`jig mcp` against `jig`), and the MCP transport follows the protocol. |

Put each check at the lowest level that can express it. A rule that holds
for any input belongs at level 1. Levels 2 to 4 run the real binary,
because the CLI is the product. The whole suite costs seconds, so we do
not trade that realism away for speed.

## Families

A contract shared by a family of members has **one** test. That test gets
the members from the same declaration the product uses. A new member is
then covered without editing the test, and a member without a case fails
it.

| Family | Source of members | One test checks |
| --- | --- | --- |
| Commands | `COMMANDS` | help line, refusal without a registry, `--json` accepted exactly for read commands, usage errors name the command |
| Arguments and text fields | argument declarations in `COMMANDS` | every text argument accepts and rejects inputs according to its declared kind (single line, multiline, exact body, ID, release name), and a value that looks like an option stays text |
| Lifecycle | statuses × commands | allowed and refused transitions, the history line each one writes, and the files it moves |
| Readiness | readiness reasons | one table of registry shapes. For every row: the `Ready:` line, JSON `readiness`, `ready` membership and warning, and the `claim` refusal agree. |
| Damaged registry | `check` problem kinds | each seeded problem × each command class: lists skip it once with a warning, `show` reports it, writes refuse it, and `check` names it without changing a byte |
| Codec | seeded generator | write → read round trip, or refusal, for generated fields and bodies |
| MCP parity | `tools/list` | for every tool, the same result text and registry bytes as the CLI on an identical registry, including a refusal, a warning, and values that look like options |

A failing assertion names the member and the case (command, field, status,
problem kind, seed), so the failure points to the declaration to fix.

## When to add a test

- **A new contract** gets a test only when no existing test covers it.
  If you cannot name the contract, do not add the test.
- **A new member of a family** (command, argument, status, problem kind,
  tool) needs no new test. If the family test cannot find the member, fix
  the test so it reads the declaration.
- **A bug found in review** becomes a case of the existing contract test.
  It is never a test of its own. Test names carry no round or issue
  numbers. If no test covers that contract, add a contract test, not a
  test for the fix.
- **A stronger test replaces weaker checks** of the same contract. It does
  not sit next to them.
- Tests come from the contract in `SPEC.md`. Review findings are cases of
  that contract, not a separate list of tests.

## Assertions

- Where the contract defines an output, compare the whole output: stdout,
  stderr, JSON, or file bytes. Use `contains` only for the part of a
  message that the contract fixes.
- Normalize timestamps, `claim_id`, and generated IDs with one shared
  helper before comparing registries. Never weaken a comparison to avoid
  normalizing.
- A check that something does not happen must be able to fail: first show
  that the same assertion catches the opposite.

## Harness

- Each test gets its own directory under `jig-tests/` in the system
  temporary directory, and its own `XDG_STATE_HOME`. The directory is
  outside the checkout because this repository tracks its own work in Jig,
  and a test must never discover that registry. Tests run in parallel and
  do not depend on each other's order.
- Child processes: read stdout and stderr while the process runs, assert
  the exit code, and treat a timeout as detecting a hang (seconds, not
  milliseconds), never as measuring speed. No fixed sleeps.
- Seeded generators print the seed and the case index on failure.
- Tests that only make sense on one platform are gated with `cfg`, and
  their names say so.

## Speed

The suite stays fast: `cargo test --locked` on an already built tree takes
seconds, and a change must not make it noticeably slower. Speed comes from
where a check lives, not from dropping coverage:

- **Wide input spaces go to level 1.** Generated values, every character
  class, and long seeded runs exercise the pure rule in process, where a
  thousand cases cost milliseconds.
- **Levels 2 to 4 prove the wiring.** A family test at these levels runs
  each member through the binary with one representative per class of the
  rule (accepted, each kind of refusal). It does not run the full
  generator per member, because every case there costs a process.
- **Share the setup.** Seed a registry once per family test and copy it,
  instead of building it with a chain of commands for every case.
- **No fixed sleeps.** Timeouts only detect hangs.

The added time of `cargo test --locked` is part of the cost of a change.
When you hand in a change, report it together with the test count.
