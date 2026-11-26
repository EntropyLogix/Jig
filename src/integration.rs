use crate::store;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const BEGIN: &str = "<!-- BEGIN ENTROPYJIG -->";
const END: &str = "<!-- END ENTROPYJIG -->";
const COMMAND: &str = "jig start --hook-json";
const CONFIGS: [(&str, &str); 2] = [(".claude", "settings.json"), (".codex", "hooks.json")];

const BLOCK: &str = "<!-- BEGIN ENTROPYJIG -->
## Issue tracking with EntropyJig

- Run `jig start` for the workflow. Use `jig ready` to find eligible work and `jig show ID` to read the full issue. Readiness is not approval to change a project contract.
- Create proposals with `jig add` without `--approved`. A proposal is not work. Humans approve with `jig approve`, revise with `jig revise`, or close with `jig close`.
- Run `jig claim ID` before starting work. `--by NAME` is optional. Use `--force` only when a human explicitly authorizes it for that issue.
- Submit finished work with `jig review ID [--summary TEXT]`. A human closes issues with `jig close ID --reason TEXT`.
- Make issue changes through `jig`. Use `jig recall [TOPIC]` to read project memory on demand.
- If `jig ready` warns, run `jig check` and report the result to a human; do not remove dependencies to get unblocked.
<!-- END ENTROPYJIG -->
";

struct Change {
    path: PathBuf,
    text: String,
}

pub(crate) fn install(root: &Path, with_hooks: bool) -> Result<(), String> {
    let mut changes = Vec::new();
    let instructions = root.join("AGENTS.md");
    if let Some(text) = instructions_update(&read(&instructions)?)? {
        changes.push(Change {
            path: instructions,
            text,
        });
    }
    if with_hooks {
        for (directory, filename) in CONFIGS {
            let dir = root.join(directory);
            store::metadata_of(&dir, true)?;
            let path = dir.join(filename);
            if let Some(text) =
                hook_update(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?
            {
                changes.push(Change { path, text });
            }
        }
    }
    for Change { path, text } in changes {
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        store::atomic_write(&path, text.as_bytes())?;
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    if store::metadata_of(path, false)?.is_none() {
        return Ok(String::new());
    }
    fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn instructions_update(text: &str) -> Result<Option<String>, String> {
    let malformed =
        || "AGENTS.md has malformed EntropyJig marker pairs; refusing to edit".to_string();
    let mut begin = None;
    let mut end = None;
    for (i, line) in text.lines().enumerate() {
        if line == BEGIN {
            if begin.is_some() {
                return Err(malformed());
            }
            begin = Some(i);
        } else if line == END {
            if end.is_some() {
                return Err(malformed());
            }
            end = Some(i);
        }
    }
    match (begin, end) {
        (Some(b), Some(e)) if b < e => return Ok(None),
        (None, None) => {}
        _ => return Err(malformed()),
    }
    let newline = if text.as_bytes().windows(2).any(|b| b == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let block = BLOCK.replace('\n', newline);
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(newline);
        }
        if !out.ends_with(format!("{newline}{newline}").as_str()) {
            out.push_str(newline);
        }
    }
    out.push_str(&block);
    Ok(Some(out))
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while matches!(bytes.get(i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        i += 1;
    }
    i
}

fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    loop {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
}

fn value_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    match bytes[start] {
        b'"' => string_end(bytes, start),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut i = start;
            loop {
                match bytes[i] {
                    b'"' => i = string_end(bytes, i),
                    b'{' | b'[' => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' | b']' => {
                        depth -= 1;
                        i += 1;
                        if depth == 0 {
                            return i;
                        }
                    }
                    _ => i += 1,
                }
            }
        }
        _ => {
            let mut i = start;
            while !matches!(
                bytes.get(i),
                None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')
            ) {
                i += 1;
            }
            i
        }
    }
}

struct Scan {
    found: Option<usize>,
    found_key: Option<usize>,
    close: usize,
    insert_at: usize,
    has: bool,
    last_pos: Option<usize>,
    standalone_pos: Option<usize>,
}

fn scan_object(text: &str, start: usize, key: &str) -> Result<Scan, String> {
    let bytes = text.as_bytes();
    let mut out = Scan {
        found: None,
        found_key: None,
        close: 0,
        insert_at: start + 1,
        has: false,
        last_pos: None,
        standalone_pos: None,
    };
    let mut i = start + 1;
    loop {
        i = skip_ws(bytes, i);
        if bytes[i] == b'}' {
            out.close = i;
            return Ok(out);
        }
        let key_pos = i;
        if line_indent(text, key_pos).is_some() {
            out.standalone_pos = Some(key_pos);
        }
        let key_end = string_end(bytes, i);
        let decoded: String = serde_json::from_str(&text[key_pos..key_end])
            .map_err(|e| format!("cannot read object key: {e}"))?;
        i = skip_ws(bytes, key_end);
        i = skip_ws(bytes, i + 1);
        let end = value_end(text, i);
        if decoded == key {
            if out.found.is_some() {
                return Err(format!("duplicate key {key:?}; refusing to edit"));
            }
            out.found = Some(i);
            out.found_key = Some(key_pos);
        }
        out.has = true;
        out.insert_at = end;
        out.last_pos = Some(key_pos);
        i = skip_ws(bytes, end);
        if bytes[i] == b',' {
            i += 1;
        }
    }
}

fn scan_array(text: &str, start: usize) -> Scan {
    let bytes = text.as_bytes();
    let mut out = Scan {
        found: None,
        found_key: None,
        close: 0,
        insert_at: start + 1,
        has: false,
        last_pos: None,
        standalone_pos: None,
    };
    let mut i = start + 1;
    loop {
        i = skip_ws(bytes, i);
        if bytes[i] == b']' {
            out.close = i;
            return out;
        }
        if line_indent(text, i).is_some() {
            out.standalone_pos = Some(i);
        }
        let end = value_end(text, i);
        out.has = true;
        out.insert_at = end;
        out.last_pos = Some(i);
        i = skip_ws(bytes, end);
        if bytes[i] == b',' {
            i += 1;
        }
    }
}

fn line_indent(text: &str, pos: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = pos;
    while i > 0 {
        match bytes[i - 1] {
            b' ' | b'\t' => i -= 1,
            b'\n' => break,
            _ => return None,
        }
    }
    Some(text[i..pos].to_string())
}

fn member_indent(text: &str, scan: &Scan, parent_indent: &str, unit: &str) -> String {
    if scan.has {
        return scan
            .standalone_pos
            .and_then(|p| line_indent(text, p))
            .or_else(|| line_indent(text, scan.close).map(|c| format!("{c}{unit}")))
            .unwrap_or_else(|| format!("{parent_indent}{unit}"));
    }
    match line_indent(text, scan.close) {
        Some(close) => format!("{close}{unit}"),
        None => format!("{parent_indent}{unit}"),
    }
}

fn measured_unit(text: &str, scan: &Scan, parent_anchor: Option<usize>) -> Option<String> {
    let child = scan.standalone_pos.and_then(|p| line_indent(text, p))?;
    let parent = parent_anchor.and_then(|p| line_indent(text, p))?;
    child
        .strip_prefix(&parent)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
}

fn unit_for(text: &str, scans: &[(&Scan, Option<usize>)]) -> String {
    for (scan, parent_anchor) in scans {
        if let Some(u) = measured_unit(text, scan, *parent_anchor) {
            return u;
        }
    }
    "  ".to_string()
}

fn splice(
    text: &str,
    scan: &Scan,
    nl: &str,
    member: &str,
    indent: &str,
    parent_indent: &str,
) -> String {
    let mut fragment = String::new();
    if scan.has {
        fragment.push(',');
    }
    fragment.push_str(nl);
    fragment.push_str(indent);
    fragment.push_str(member);
    let tail = &text[scan.insert_at..scan.close];
    if !tail.contains('\n') {
        fragment.push_str(nl);
        fragment
            .push_str(&line_indent(text, scan.close).unwrap_or_else(|| parent_indent.to_string()));
    }
    let mut out = String::with_capacity(text.len() + fragment.len());
    out.push_str(&text[..scan.insert_at]);
    out.push_str(&fragment);
    out.push_str(&text[scan.insert_at..]);
    out
}

const COMPACT_GROUP: &str =
    "{\"hooks\":[{\"type\":\"command\",\"command\":\"jig start --hook-json\",\"timeout\":5}]}";

fn hook_group(nl: &str, unit: &str, indent: &str, compact: bool) -> String {
    if compact {
        return COMPACT_GROUP.to_string();
    }
    format!(
        "{{{nl}{indent}{unit}\"hooks\": [{nl}{indent}{unit}{unit}{{{nl}{indent}{unit}{unit}{unit}\"type\": \"command\",{nl}{indent}{unit}{unit}{unit}\"command\": \"{COMMAND}\",{nl}{indent}{unit}{unit}{unit}\"timeout\": 5{nl}{indent}{unit}{unit}}}{nl}{indent}{unit}]{nl}{indent}}}"
    )
}

fn hook_update(text: &str) -> Result<Option<String>, String> {
    const UNIT: &str = "  ";
    let file_nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let desired = json!({"hooks":[{"type":"command","command":COMMAND,"timeout":5}]});

    if text.trim().is_empty() {
        let group = hook_group(file_nl, UNIT, &UNIT.repeat(3), false);
        let result = format!(
            "{text}{{{file_nl}{UNIT}\"hooks\": {{{file_nl}{UNIT}{UNIT}\"SessionStart\": [{file_nl}{UNIT}{UNIT}{UNIT}{group}{file_nl}{UNIT}{UNIT}]{file_nl}{UNIT}}}{file_nl}}}{file_nl}"
        );
        let mut expected = json!({});
        expected
            .as_object_mut()
            .unwrap()
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .entry("SessionStart")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(desired);
        let parsed: Value =
            serde_json::from_str(&result).map_err(|e| format!("internal splice error: {e}"))?;
        if parsed != expected {
            return Err("internal splice verification failed".to_string());
        }
        return Ok(Some(result));
    }

    let orig: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    if !orig.is_object() {
        return Err("expected an object at the root".to_string());
    }
    let compact = !text.trim().contains('\n');
    let nl = if compact { "" } else { file_nl };
    let bytes = text.as_bytes();
    let root_open = skip_ws(bytes, 0);
    let root = scan_object(text, root_open, "hooks")?;

    let (target, parent_indent, member, indent);
    match root.found {
        None => {
            let unit = if compact {
                String::new()
            } else {
                unit_for(text, &[(&root, Some(root_open))])
            };
            let ind = member_indent(text, &root, "", &unit);
            let inner = format!("{ind}{unit}");
            member = if compact {
                format!("\"hooks\":{{\"SessionStart\":[{COMPACT_GROUP}]}}")
            } else {
                format!(
                    "\"hooks\": {{{nl}{inner}\"SessionStart\": [{nl}{inner}{unit}{}{nl}{inner}]{nl}{ind}}}",
                    hook_group(nl, &unit, &format!("{inner}{unit}"), false)
                )
            };
            target = root;
            parent_indent = String::new();
            indent = ind;
        }
        Some(hooks_at) => {
            if !orig["hooks"].is_object() {
                return Err("expected hooks to be an object".to_string());
            }
            let hooks = scan_object(text, hooks_at, "SessionStart")?;
            match hooks.found {
                None => {
                    let unit = if compact {
                        String::new()
                    } else {
                        unit_for(text, &[(&hooks, root.found_key), (&root, Some(root_open))])
                    };
                    let hooks_indent = root
                        .found_key
                        .and_then(|p| line_indent(text, p))
                        .unwrap_or_else(|| unit.clone());
                    let ind = member_indent(text, &hooks, &hooks_indent, &unit);
                    member = if compact {
                        format!("\"SessionStart\":[{COMPACT_GROUP}]")
                    } else {
                        format!(
                            "\"SessionStart\": [{nl}{ind}{unit}{}{nl}{ind}]",
                            hook_group(nl, &unit, &format!("{ind}{unit}"), false)
                        )
                    };
                    target = hooks;
                    parent_indent = hooks_indent;
                    indent = ind;
                }
                Some(array_at) => {
                    let groups = orig["hooks"]["SessionStart"]
                        .as_array()
                        .ok_or("expected hooks.SessionStart to be an array")?;
                    let mut found = false;
                    for group in groups {
                        let list = group
                            .get("hooks")
                            .and_then(Value::as_array)
                            .ok_or("expected each SessionStart group to contain a hooks array")?;
                        for item in list {
                            if !item.is_object() {
                                return Err(
                                    "expected each SessionStart hook to be an object".into()
                                );
                            }
                            if item.get("command").and_then(Value::as_str) == Some(COMMAND) {
                                if group != &desired || found {
                                    return Err(
                                        "existing EntropyJig SessionStart handler differs or is duplicated; keep just one standard handler"
                                            .into(),
                                    );
                                }
                                found = true;
                            }
                        }
                    }
                    if found {
                        return Ok(None);
                    }
                    let array = scan_array(text, array_at);
                    let unit = if compact {
                        String::new()
                    } else {
                        unit_for(
                            text,
                            &[
                                (&array, hooks.found_key),
                                (&hooks, root.found_key),
                                (&root, Some(root_open)),
                            ],
                        )
                    };
                    let hooks_indent = root
                        .found_key
                        .and_then(|p| line_indent(text, p))
                        .unwrap_or_else(|| unit.clone());
                    let start_indent = hooks
                        .found_key
                        .and_then(|p| line_indent(text, p))
                        .unwrap_or_else(|| format!("{hooks_indent}{unit}"));
                    let ind = member_indent(text, &array, &start_indent, &unit);
                    member = hook_group(nl, &unit, &ind, compact);
                    target = array;
                    parent_indent = start_indent;
                    indent = ind;
                }
            }
        }
    }
    let result = splice(text, &target, nl, &member, &indent, &parent_indent);

    let mut expected = orig.clone();
    expected
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap()
        .entry("SessionStart")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap()
        .push(desired);
    let parsed: Value =
        serde_json::from_str(&result).map_err(|e| format!("splice produced invalid JSON: {e}"))?;
    if parsed != expected {
        return Err("internal splice verification failed".to_string());
    }
    Ok(Some(result))
}
