pub struct Memory {
    pub entries: Vec<(String, String)>,
}

pub const HEADER: &str = "# Project Memory";

pub fn valid_topic(t: &str) -> bool {
    let mut it = t.bytes();
    match it.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    it.all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

pub fn render(m: &Memory) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for (topic, value) in &m.entries {
        out.push('\n');
        out.push_str("## ");
        out.push_str(topic);
        out.push('\n');
        for line in value.split('\n') {
            if line.is_empty() {
                out.push_str(">\n");
            } else {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

pub fn parse(text: &str) -> Result<Memory, String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    let mut header_seen = false;
    let finish = |entries: &mut Vec<(String, String)>, cur: &mut Option<(String, Vec<String>)>| {
        if let Some((name, lines)) = cur.take() {
            if entries.iter().any(|(t, _)| t == &name) {
                return Err(format!("duplicate memory topic {name:?}"));
            }
            entries.push((name, lines.join("\n")));
        }
        Ok(())
    };
    for line in text.split('\n') {
        if !header_seen {
            if line.trim().is_empty() {
                continue;
            }
            if line != HEADER {
                return Err("memory.md missing header".to_string());
            }
            header_seen = true;
            continue;
        }
        if line.starts_with('>') {
            let v = line
                .strip_prefix("> ")
                .or_else(|| line.strip_prefix('>'))
                .unwrap();
            match cur.as_mut() {
                Some((_, lines)) => lines.push(v.to_string()),
                None => return Err("memory value before any topic".to_string()),
            }
            continue;
        }
        if let Some(name) = line.strip_prefix("## ") {
            let topic = name.trim_end();
            if !valid_topic(topic) {
                return Err(format!("invalid memory topic {topic:?}"));
            }
            finish(&mut entries, &mut cur)?;
            cur = Some((topic.to_string(), Vec::new()));
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        return Err(format!("unexpected memory content {line:?}"));
    }
    if !header_seen {
        return Err("memory.md missing header".to_string());
    }
    finish(&mut entries, &mut cur)?;
    Ok(Memory { entries })
}
