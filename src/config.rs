use crate::issue;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub const FILE: &str = "config.json";
pub const DEFAULT_PREFIX: &str = "B";
pub const PREFIX_RULE: &str = "use 1 to 8 uppercase letters, optionally followed by '-'";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub prefix: String,
}

pub fn parse(text: &str) -> Result<Config, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let Value::Object(map) = value else {
        return Err("must be a JSON object".to_string());
    };
    if let Some(key) = map.keys().find(|key| key.as_str() != "prefix") {
        return Err(format!("unknown key {key:?}"));
    }
    let prefix = match map.get("prefix") {
        Some(Value::String(prefix)) => prefix.clone(),
        Some(_) => return Err("prefix must be a string".to_string()),
        None => return Err("no prefix".to_string()),
    };
    if !issue::valid_prefix(&prefix) {
        return Err(format!("invalid prefix {prefix:?}; {PREFIX_RULE}"));
    }
    Ok(Config { prefix })
}

pub fn render(prefix: &str) -> String {
    format!("{{\n  \"prefix\": {}\n}}\n", issue::esc(prefix))
}

pub fn load(billet: &Path) -> Result<Config, String> {
    read(billet).map_err(|e| format!(".blt/config.json: {e}"))
}

pub fn read(billet: &Path) -> Result<Config, String> {
    let path = billet.join(FILE);
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err("missing; run `blt init`".to_string())
        }
        Err(e) => Err(format!("cannot read: {e}")),
    }
}
