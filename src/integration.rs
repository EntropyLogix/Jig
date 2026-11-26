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
- Run `jig claim ID` before starting work. `--by NAME` is optional. Use `--force` only when a human explicitly authorizes it for that issue.
- Submit finished work with `jig review ID [--summary TEXT]`. A human closes issues with `jig close ID --reason TEXT`.
- Make issue changes through `jig`. Use `jig recall [TOPIC]` to read project memory on demand.
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

fn hook_update(text: &str) -> Result<Option<String>, String> {
    let mut value: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?
    };
    let cfg = value
        .as_object_mut()
        .ok_or("expected an object at the root")?;
    let hooks = cfg
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("expected hooks to be an object")?;
    let groups = hooks
        .entry("SessionStart")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("expected hooks.SessionStart to be an array")?;
    let handler = json!({"type":"command", "command": COMMAND, "timeout":5});
    let desired = json!({"hooks":[handler]});
    let mut found = false;
    for group in groups.iter() {
        let list = group
            .get("hooks")
            .and_then(Value::as_array)
            .ok_or("expected each SessionStart group to contain a hooks array")?;
        for item in list {
            if !item.is_object() {
                return Err("expected each SessionStart hook to be an object".into());
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
    groups.push(desired);
    let pretty = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(Some(format!("{pretty}\n")))
}
