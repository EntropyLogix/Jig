use crate::store;
use std::fs;
use std::path::Path;

macro_rules! text {
    ($heading:literal) => {
        concat!(
            $heading,
            "\n\n",
            "This project tracks work with `blt`. Do not keep task lists in other Markdown or TODO files. Your built-in todo or plan tool is only for the steps of the current task; record anything that must outlive this session in `blt`.\n",
            "If `blt` is not installed, ask a person to install it: `cargo install --locked --git https://github.com/EntropyLogix/Billet --tag v",
            env!("CARGO_PKG_VERSION"),
            "` (https://github.com/EntropyLogix/Billet).\n\n",
            "- Run `blt start` for the workflow. `blt ready` lists approved, unblocked work; `blt show ID` reads one issue. An issue authorizes only its described scope. Refer to issues by the ID `blt` prints (for example `B12`, subtask `B12.3`), in commands and in messages; a person may give only the number (`12`).\n",
            "- Claim before working: `blt claim ID --by NAME` with your agent name. Use `--force` only when a person explicitly authorizes it for that issue.\n",
            "- Submit finished work with `blt review ID --summary TEXT`. Approving, revising, returning for rework and closing issues is for people unless a person explicitly tells you otherwise for that issue.\n",
            "- Work found along the way: `blt add TITLE --body TEXT` (it starts as a proposal, not work) and `blt note ID TEXT` on the issue where you found it.\n",
            "- The issue description is the current contract: rewrite it with `blt update ID --body -` as decisions change instead of appending; write long analyses and reports directly in the `Documents:` directory from `blt show ID`, not in other directories.\n",
            "- Choose the relationship: a part of this issue's work is a subtask (`blt add TITLE --parent ID --body TEXT`); separate work that must happen beforehand is a blocker (`blt dep add ID BLOCKER`). A parent is ready only after its subtasks close; a closed blocker stops blocking without removing the link. If your issue needs other work beforehand, add it as a subtask or blocker, leave a `blt note`, and run `blt unclaim ID`; use `blt dep rm` only to undo your own mistake, with a note.\n",
            "- Stopping before the work is done: leave a `blt note`; run `blt unclaim ID` if you will not continue.\n",
            "- Read project facts on demand with `blt recall [TOPIC]`; record durable project facts with `blt remember TOPIC TEXT`. Keep personal preferences in your own memory.\n",
            "- If `blt ready` warns or lists nothing, run `blt check` and report to a person; never remove dependencies or approve proposals to get unblocked.\n"
        )
    };
}

pub(crate) const WORKFLOW: &str = text!("Billet workflow");
pub(crate) const MCP_INSTRUCTIONS: &str = "Use these MCP tools instead of running `blt` in the shell. Each tool mirrors the command of the same name. A person's refusal of a tool call is final and is not retried through the shell.";
const LINE: &str = "This project tracks work with `blt` (https://github.com/EntropyLogix/Billet): run `blt start` before you begin and follow its rules; if `blt` is missing, ask a person to install it.";

fn present(text: &str) -> bool {
    text.lines()
        .any(|line| line.strip_suffix('\r').unwrap_or(line) == LINE)
}

pub(crate) fn update(text: &str) -> Option<String> {
    if present(text) {
        return None;
    }
    let newline = if text.as_bytes().windows(2).any(|b| b == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(newline);
        }
        if !out.ends_with(format!("{newline}{newline}").as_str()) {
            out.push_str(newline);
        }
    }
    out.push_str(LINE);
    out.push_str(newline);
    Some(out)
}

pub(crate) fn problem(root: &Path) -> Option<(String, &'static str)> {
    let path = root.join("AGENTS.md");
    let missing = || {
        Some((
            "Billet instructions are missing".to_string(),
            "run `blt init` to add the instructions",
        ))
    };
    match store::metadata_cause(&path, false) {
        Err(e) => Some((e, "fix AGENTS.md manually")),
        Ok(None) => missing(),
        Ok(Some(_)) => match fs::read_to_string(&path) {
            Err(e) => Some((format!("cannot read: {e}"), "fix AGENTS.md manually")),
            Ok(text) if present(&text) => None,
            Ok(_) => missing(),
        },
    }
}
