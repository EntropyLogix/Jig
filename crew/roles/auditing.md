# Auditing rules

These rules apply to every audit and to the lead's own audit.

- **Read only.** Do not change code, tests, documentation, configuration,
  git state, or Billet state in the working tree. Other auditors work on the
  same tree at the same time, and the developer's fingerprint must keep
  matching. Do not run formatters or other tools that rewrite files. Build
  and test output in ignored directories is fine.
- **Code state.** The `Code:` line above is computed by crew itself: it
  recomputes the fingerprint of the working tree and compares it with the
  one in the handoff. Rely on it and do not recompute the fingerprint by
  other means. If it reports a change or a missing fingerprint, put that at
  the top of your report and tell the person, then audit the code as it is
  now unless the person tells you to stop.
- **Against the contract.** Audit against the issue description, the
  architecture documents listed above, the documents the description
  names, and the repository's AGENTS.md and guidelines. The handoff is a
  list of claims: verify each one yourself by running the named test,
  reading the assertion line, and running the gate. A claim you could not
  confirm is a finding.
- **Previous decision.** If a previous decision is listed above, check that
  each of its items is done.
- **Beyond the handoff.** Look for behavior the contract requires that
  nobody claimed, regressions, and edge cases. Prefer probes (small scripts
  run in a temporary directory) over reading alone.
- **Mutants, for every contract point.** Passing tests prove only what
  they assert. For each contract point, change the product code so that the
  point is violated, in a plausible way that is not the old bug (for example
  a constant result, a dropped condition, a missing case), and run the
  tests. Each point needs at least one mutant. A mutant that survives is a
  finding: the tests do not protect that point. Record each mutant and its
  result under "Verified claims" or as a finding. Do this only in a copy of
  the repository in a temporary directory, never in the shared working
  tree, and delete the copy afterwards.
- **Findings.** Number them `{{LABEL}}-1`, `{{LABEL}}-2`, … Each finding has
  a priority, a location (file:line or command), what is wrong, how to
  reproduce it, and the expected result.
  - P1: wrong behavior, data loss, a crash, or a broken gate.
  - P2: a contract violation or a missing required test.
  - P3: a minor inconsistency.

  Priority orders the work. It does not make a finding optional.
- **Only what you reproduced.** Put suspicions you could not confirm under
  "Not verified", with what would confirm them.
- **No fixes.** Describe the expected result. Do not write patches or
  replacement code.
