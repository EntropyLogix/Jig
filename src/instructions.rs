use crate::store;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

const VERSION: u32 = 1;
const BEGIN: &str = "<!-- BEGIN BLT v:";
const END: &str = "<!-- END BLT -->";

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
const BLOCK: &str = text!("## Issue tracking with Billet");

fn content_hash(text: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in text.replace("\r\n", "\n").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

struct ManagedBlock {
    start: usize,
    end: usize,
    version: u32,
    customized: bool,
    crlf: bool,
}

fn parse_opener(line: &str) -> Option<(u32, u64)> {
    let inner = line.strip_prefix(BEGIN)?.strip_suffix(" -->")?;
    let (version, hash) = inner.split_once(" hash:")?;
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let version = version.parse::<u32>().ok()?;
    if hash.len() != 16
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some((version, u64::from_str_radix(hash, 16).ok()?))
}

fn marker_candidate(line: &str) -> bool {
    for prefix in ["<!-- BEGIN BLT", "<!-- END BLT"] {
        if let Some(rest) = line.trim().strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with(|c: char| !c.is_alphanumeric()))
        {
            return true;
        }
    }
    false
}

fn locate(text: &str) -> Result<Option<ManagedBlock>, String> {
    let malformed = || "malformed or duplicated Billet instruction markers".to_string();
    let mut opener: Option<(usize, usize, u32, u64, bool)> = None;
    let mut closer: Option<(usize, usize)> = None;
    let mut offset = 0usize;
    for piece in text.split_inclusive('\n') {
        let line = piece.strip_suffix('\n').unwrap_or(piece);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some((version, digest)) = parse_opener(line) {
            if opener.is_some() || closer.is_some() {
                return Err(malformed());
            }
            opener = Some((
                offset,
                offset + piece.len(),
                version,
                digest,
                piece.ends_with("\r\n"),
            ));
        } else if line == END {
            if opener.is_none() || closer.is_some() {
                return Err(malformed());
            }
            closer = Some((offset, offset + line.len()));
        } else if marker_candidate(line) {
            return Err(malformed());
        }
        offset += piece.len();
    }
    match (opener, closer) {
        (None, None) => Ok(None),
        (Some((start, body_start, version, digest, crlf)), Some((end_start, end))) => {
            let customized = digest != content_hash(&text[body_start..end_start]);
            Ok(Some(ManagedBlock {
                start,
                end,
                version,
                customized,
                crlf,
            }))
        }
        _ => Err(malformed()),
    }
}

fn block_text(newline: &str) -> String {
    format!(
        "{BEGIN}{VERSION} hash:{:016x} -->\n{BLOCK}{END}",
        content_hash(BLOCK)
    )
    .replace('\n', newline)
}

fn append(text: &str) -> String {
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
    out.push_str(&block_text(newline));
    out.push_str(newline);
    out
}

pub(crate) enum Update {
    Write(String),
    Unchanged,
    Customized,
    NeedsAttention { problem: String, hint: &'static str },
}

fn existing_problem(block: &ManagedBlock) -> Option<(String, &'static str)> {
    match (block.version.cmp(&VERSION), block.customized) {
        (Ordering::Greater, _) => Some((
            format!(
                "Billet instructions block version {} is newer than supported version {VERSION}",
                block.version
            ),
            "use a version of blt that understands that block version",
        )),
        (Ordering::Less, true) => Some((
            "customized Billet instructions are outdated".to_string(),
            "remove the managed block, run `blt init`, and reapply your local edits",
        )),
        (Ordering::Less, false) => Some((
            "Billet instructions are outdated".to_string(),
            "run `blt init` to update the instructions",
        )),
        (Ordering::Equal, _) => None,
    }
}

pub(crate) fn update(text: &str) -> Result<Update, String> {
    let Some(b) = locate(text)? else {
        return Ok(Update::Write(append(text)));
    };
    if b.version < VERSION && !b.customized {
        let newline = if b.crlf { "\r\n" } else { "\n" };
        let block = block_text(newline);
        return Ok(Update::Write(format!(
            "{}{block}{}",
            &text[..b.start],
            &text[b.end..]
        )));
    }
    if let Some((problem, hint)) = existing_problem(&b) {
        return Ok(Update::NeedsAttention { problem, hint });
    }
    Ok(if b.customized {
        Update::Customized
    } else {
        Update::Unchanged
    })
}

pub(crate) fn problem(root: &Path) -> Option<(String, &'static str)> {
    let path = root.join("AGENTS.md");
    let text = match store::metadata_cause(&path, false) {
        Err(e) => return Some((e, "fix AGENTS.md manually")),
        Ok(None) => {
            return Some((
                "Billet instructions are missing".to_string(),
                "run `blt init` to add the instructions",
            ));
        }
        Ok(Some(_)) => match fs::read_to_string(&path) {
            Err(e) => return Some((format!("cannot read: {e}"), "fix AGENTS.md manually")),
            Ok(text) => text,
        },
    };
    match locate(&text) {
        Err(e) => Some((e, "resolve AGENTS.md manually")),
        Ok(None) => Some((
            "Billet instructions are missing".to_string(),
            "run `blt init` to add the instructions",
        )),
        Ok(Some(b)) => existing_problem(&b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_block_fingerprint_requires_version_bump() {
        let tag = concat!("--tag v", env!("CARGO_PKG_VERSION"));
        assert_eq!(
            BLOCK.matches(tag).count(),
            1,
            "expected one generated installation tag"
        );
        let stable = BLOCK.replace(tag, "--tag v<PACKAGE_VERSION>");
        let expected = match VERSION {
            1 => 0x27b4_c308_7532_9de1,
            v => panic!("Record the block fingerprint for VERSION {v}"),
        };
        assert_eq!(
            content_hash(&stable),
            expected,
            "Changing the Billet instructions requires bumping VERSION and recording the new fingerprint"
        );
    }
}
