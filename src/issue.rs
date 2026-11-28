use crate::clock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Proposed,
    Open,
    Claimed,
    Review,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Proposed => "proposed",
            Status::Open => "open",
            Status::Claimed => "claimed",
            Status::Review => "review",
            Status::Closed => "closed",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "proposed" => Some(Status::Proposed),
            "open" => Some(Status::Open),
            "claimed" => Some(Status::Claimed),
            "review" => Some(Status::Review),
            "closed" => Some(Status::Closed),
            _ => None,
        }
    }

    pub fn active(self) -> bool {
        self != Status::Closed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub priority: u8,
    pub parent: Option<String>,
    pub depends_on: Vec<String>,
    pub assignee: Option<String>,
    pub defer_until: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub closed_at: Option<String>,
    pub release: Option<String>,
    pub claim_id: Option<String>,
    pub body: String,
    pub history: String,
    pub notes: String,
    pub closing_summary: Option<String>,
}

pub(crate) const BAD_PRIORITY: &str = "field priority must be an integer from 0 to 4";

const KEYS: [&str; 13] = [
    "id",
    "title",
    "status",
    "priority",
    "parent",
    "depends_on",
    "assignee",
    "defer_until",
    "created_at",
    "updated_at",
    "closed_at",
    "release",
    "claim_id",
];

pub fn valid_label(text: &str) -> bool {
    !text.trim().is_empty()
        && !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

pub fn valid_release(text: &str) -> bool {
    valid_label(text) && text.trim() == text
}

pub fn valid_id(id: &str) -> bool {
    let (stem, suffix) = match id.split_once('.') {
        Some((s, t)) => (s, Some(t)),
        None => (id, None),
    };
    let stem_ok = if let Some(hex) = stem.strip_prefix("B-") {
        hex.len() == 6
            && hex
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    } else if let Some(digits) = stem.strip_prefix('B') {
        !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())
    } else {
        false
    };
    if !stem_ok {
        return false;
    }
    match suffix {
        None => true,
        Some(rest) => rest.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()) && !part.starts_with('0')
        }),
    }
}

pub fn id_alias(id: &str) -> Option<String> {
    if valid_id(id) {
        return Some(id.to_string());
    }
    let (stem, suffix) = match id.split_once('.') {
        Some((s, t)) => (s, format!(".{t}")),
        None => (id, String::new()),
    };
    if stem.len() == 6
        && stem
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        let candidate = format!("B-{stem}{suffix}");
        if valid_id(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn field_str(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&esc(value));
    out.push('\n');
}

fn field_opt(out: &mut String, key: &str, value: &Option<String>) {
    match value {
        Some(v) => field_str(out, key, v),
        None => {
            out.push_str(key);
            out.push_str(": null\n");
        }
    }
}

pub fn encode(issue: &Issue) -> Result<String, String> {
    validate(issue)?;
    if let Some((problem, _)) = crate::domain::field_problems(issue).first() {
        return Err((*problem).to_string());
    }
    let mut out = String::from("---\n");
    field_str(&mut out, "id", &issue.id);
    field_str(&mut out, "title", &issue.title);
    out.push_str("status: ");
    out.push_str(issue.status.as_str());
    out.push('\n');
    out.push_str(&format!("priority: {}\n", issue.priority));
    field_opt(&mut out, "parent", &issue.parent);
    if issue.depends_on.is_empty() {
        out.push_str("depends_on: []\n");
    } else {
        out.push_str("depends_on: [");
        let parts: Vec<String> = issue.depends_on.iter().map(|d| esc(d)).collect();
        out.push_str(&parts.join(", "));
        out.push_str("]\n");
    }
    field_opt(&mut out, "assignee", &issue.assignee);
    field_opt(&mut out, "defer_until", &issue.defer_until);
    field_opt(&mut out, "created_at", &issue.created_at);
    field_opt(&mut out, "updated_at", &issue.updated_at);
    field_opt(&mut out, "closed_at", &issue.closed_at);
    field_opt(&mut out, "release", &issue.release);
    field_opt(&mut out, "claim_id", &issue.claim_id);
    out.push_str("---\n\n");
    if let Some(summary) = &issue.closing_summary {
        out.push_str("## Closing Summary\n\n");
        for line in summary.split('\n') {
            if line.is_empty() {
                out.push_str(">\n");
            } else {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    if !issue.body.is_empty() {
        out.push_str(&issue.body);
        out.push_str("\n\n");
    }
    out.push_str("## History\n\n");
    if !issue.history.is_empty() {
        out.push_str(&issue.history);
        out.push_str("\n\n");
    }
    out.push_str("## Notes\n\n");
    if !issue.notes.is_empty() {
        out.push_str(&issue.notes);
        out.push_str("\n\n");
    }
    merge_markers(&out)?;
    Ok(out)
}

fn validate(issue: &Issue) -> Result<(), String> {
    if !valid_id(&issue.id) {
        return Err(format!("invalid issue id {:?}", issue.id));
    }
    if !valid_label(&issue.title) {
        return Err("title must be a non-empty single line".to_string());
    }
    if issue.priority > 4 {
        return Err(BAD_PRIORITY.to_string());
    }
    if let Some(a) = &issue.assignee
        && !valid_label(a)
    {
        return Err("assignee must be a non-empty single line".to_string());
    }
    if let Some(release) = &issue.release
        && !valid_release(release)
    {
        return Err(
            "release must be a non-empty single line without surrounding whitespace".to_string(),
        );
    }
    if issue.parent.as_deref() != issue.id.rsplit_once('.').map(|(parent, _)| parent) {
        return Err("parent must match the dotted issue id".to_string());
    }
    if let Some(parent) = &issue.parent
        && !valid_id(parent)
    {
        return Err(format!("invalid parent id {parent:?}"));
    }
    let mut seen_deps = std::collections::HashSet::new();
    for dep in &issue.depends_on {
        if !valid_id(dep) {
            return Err(format!("invalid dependency id {dep:?}"));
        }
        if dep == &issue.id {
            return Err("issue cannot depend on itself".to_string());
        }
        if !seen_deps.insert(dep) {
            return Err(format!("duplicate dependency id {dep:?}"));
        }
    }
    if let Some(d) = &issue.defer_until
        && !clock::valid_date(d)
    {
        return Err(format!("invalid defer_until date {d:?}"));
    }
    for (key, value) in [
        ("created_at", &issue.created_at),
        ("updated_at", &issue.updated_at),
        ("closed_at", &issue.closed_at),
    ] {
        if let Some(v) = value
            && !clock::valid_timestamp(v)
        {
            return Err(format!("invalid {key} timestamp {v:?}"));
        }
    }
    for (what, text) in [
        ("body", &issue.body),
        ("history", &issue.history),
        ("notes", &issue.notes),
    ] {
        structural_check(text, what)?;
    }
    Ok(())
}

fn unescape(s: &str, pos: &mut usize) -> Result<String, String> {
    let b = s.as_bytes();
    if b.get(*pos) != Some(&b'"') {
        return Err(format!("expected '\"' at byte {pos}"));
    }
    *pos += 1;
    let mut out = String::new();
    loop {
        let Some(&c) = b.get(*pos) else {
            return Err("unterminated string".to_string());
        };
        match c {
            b'"' => {
                *pos += 1;
                return Ok(out);
            }
            b'\\' => {
                *pos += 1;
                let Some(&e) = b.get(*pos) else {
                    return Err("unterminated escape".to_string());
                };
                *pos += 1;
                match e {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hi = hex4(s, pos)?;
                        let cp = if (0xd800..0xdc00).contains(&hi) {
                            if s.as_bytes().get(*pos) == Some(&b'\\')
                                && s.as_bytes().get(*pos + 1) == Some(&b'u')
                            {
                                *pos += 2;
                                let lo = hex4(s, pos)?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return Err("invalid surrogate pair".to_string());
                                }
                                0x1_0000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                return Err("lone surrogate escape".to_string());
                            }
                        } else if (0xdc00..0xe000).contains(&hi) {
                            return Err("lone surrogate escape".to_string());
                        } else {
                            hi
                        };
                        out.push(char::from_u32(cp).ok_or("invalid unicode escape")?);
                    }
                    _ => return Err(format!("invalid escape \\{}", e as char)),
                }
            }
            0x00..=0x1f => return Err("unescaped control character".to_string()),
            _ => {
                let rest = &s[*pos..];
                let ch = rest.chars().next().ok_or("unterminated string")?;
                out.push(ch);
                *pos += ch.len_utf8();
            }
        }
    }
}

fn hex4(s: &str, pos: &mut usize) -> Result<u32, String> {
    let b = s.as_bytes();
    if *pos + 4 > b.len() {
        return Err("short unicode escape".to_string());
    }
    let mut v: u32 = 0;
    for &c in &b[*pos..*pos + 4] {
        let d = (c as char).to_digit(16).ok_or("invalid unicode escape")?;
        v = v * 16 + d;
    }
    *pos += 4;
    Ok(v)
}

#[derive(Clone)]
enum Value {
    Null,
    Str(String),
    List(Vec<String>),
    Priority(u8),
}

fn parse_value(raw: &str, key: &str) -> Result<Value, String> {
    let s = raw.trim_matches([' ', '\t']);
    if key == "priority" {
        return match s.as_bytes() {
            [p @ b'0'..=b'4'] => Ok(Value::Priority(p - b'0')),
            _ => Err(format!("{BAD_PRIORITY}; got {s:?}")),
        };
    }
    if s == "null" {
        return Ok(Value::Null);
    }
    if s.starts_with('"') {
        if key == "status" {
            return Err(format!("field {key} must be a plain scalar"));
        }
        let mut pos = 0usize;
        let value = unescape(s, &mut pos).map_err(|e| format!("{key}: {e}"))?;
        let tail = s[pos..].trim_end_matches([' ', '\t']);
        if !tail.is_empty() {
            return Err(format!("{key}: trailing characters after value"));
        }
        return Ok(Value::Str(value));
    }
    if let Some(inner) = s.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| format!("{key}: unterminated list"))?;
        let mut items = Vec::new();
        if !inner.trim_matches([' ', '\t']).is_empty() {
            for part in inner.split(',') {
                let p = part.trim_matches([' ', '\t']);
                if !p.starts_with('"') {
                    return Err(format!("{key}: list elements must be double-quoted"));
                }
                let mut pos = 0usize;
                let v = unescape(p, &mut pos).map_err(|e| format!("{key}: {e}"))?;
                if !p[pos..].trim_matches([' ', '\t']).is_empty() {
                    return Err(format!("{key}: trailing characters in list element"));
                }
                items.push(v);
            }
        }
        return Ok(Value::List(items));
    }
    match key {
        "status" => {
            if Status::parse(s).is_none() {
                return Err(format!("invalid status for field {key}: {s:?}"));
            }
            Ok(Value::Str(s.to_string()))
        }
        _ => Err(format!(
            "field {key} must be null or a double-quoted string"
        )),
    }
}

fn want_str(v: Value, key: &str) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s),
        _ => Err(format!("field {key} must be a string")),
    }
}

fn want_opt(v: Value, key: &str) -> Result<Option<String>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Str(s) => Ok(Some(s)),
        _ => Err(format!("field {key} must be a string or null")),
    }
}

fn conflict_marker(line: &str, marker: &str) -> bool {
    line.strip_prefix(marker)
        .is_some_and(|tail| tail.is_empty() || tail.starts_with(' '))
}

fn merge_markers(text: &str) -> Result<(), String> {
    let mut stage = 0u8;
    let mut fence: Option<Fence> = None;
    for line in text.lines() {
        match &fence {
            Some(f) => {
                if fence_closes(line, f) {
                    fence = None;
                }
            }
            None => {
                if let Some(f) = fence_opens(line) {
                    stage = 0;
                    fence = Some(f);
                } else if conflict_marker(line, "<<<<<<<") {
                    stage = 1;
                } else if line == "=======" {
                    if stage == 1 {
                        stage = 2;
                    }
                } else if conflict_marker(line, ">>>>>>>") {
                    if stage == 2 {
                        return Err("merge conflict markers present".to_string());
                    }
                    stage = 0;
                }
            }
        }
    }
    Ok(())
}

pub struct Fence {
    marker: u8,
    width: usize,
}

fn fence_run(line: &str) -> Option<(u8, usize, &str)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i > 3 {
        return None;
    }
    let m = *b.get(i)?;
    if m != b'`' && m != b'~' {
        return None;
    }
    let mut w = 0;
    while b.get(i + w) == Some(&m) {
        w += 1;
    }
    if w < 3 {
        return None;
    }
    Some((m, w, &line[i + w..]))
}

pub fn fence_opens(line: &str) -> Option<Fence> {
    let (marker, width, rest) = fence_run(line)?;
    if marker == b'`' && rest.contains('`') {
        return None;
    }
    Some(Fence { marker, width })
}

pub fn fence_closes(line: &str, fence: &Fence) -> bool {
    match fence_run(line) {
        Some((m, w, rest)) => m == fence.marker && w >= fence.width && rest.trim().is_empty(),
        None => false,
    }
}

fn managed(line: &str) -> Option<&'static str> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i > 3 {
        return None;
    }
    let after = line[i..].strip_prefix("##")?;
    if !matches!(after.as_bytes().first(), Some(b' ' | b'\t')) {
        return None;
    }
    let mut body = after.trim_end_matches([' ', '\t', '\r']);
    let text_end = body.trim_end_matches('#').len();
    if text_end < body.len() && matches!(body.as_bytes()[..text_end].last(), Some(b' ' | b'\t')) {
        body = body[..text_end].trim_end_matches([' ', '\t']);
    }
    match body.trim_start_matches([' ', '\t']) {
        "Closing Summary" => Some("Closing Summary"),
        "History" => Some("History"),
        "Notes" => Some("Notes"),
        _ => None,
    }
}

fn next_line(text: &str, pos: usize) -> &str {
    let end = text[pos.min(text.len())..]
        .find('\n')
        .map(|i| pos.min(text.len()) + i)
        .unwrap_or(text.len());
    &text[pos.min(text.len())..end]
}

fn line_end(text: &str, pos: usize) -> usize {
    text[pos.min(text.len())..]
        .find('\n')
        .map(|i| pos.min(text.len()) + i + 1)
        .unwrap_or(text.len())
}

fn scan_boundary(text: &str, from: usize, want: &str) -> Result<usize, String> {
    let mut fence: Option<Fence> = None;
    let mut pos = from;
    while pos < text.len() {
        let eol = text[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(text.len());
        let line = &text[pos..eol];
        match &fence {
            Some(f) => {
                if fence_closes(line, f) {
                    fence = None;
                }
            }
            None => {
                if let Some(name) = managed(line) {
                    if name == want {
                        return Ok(pos);
                    }
                    return Err(format!("reserved heading ## {name} inside issue text"));
                }
                fence = fence_opens(line);
            }
        }
        pos = eol + 1;
    }
    if fence.is_some() {
        return Err("unclosed code fence".to_string());
    }
    Err(format!("missing ## {want} section"))
}

pub(crate) fn structural_check(text: &str, what: &str) -> Result<(), String> {
    merge_markers(text).map_err(|e| format!("{what}: {e}"))?;
    let mut fence: Option<Fence> = None;
    let mut pos = 0usize;
    while pos < text.len() {
        let eol = text[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(text.len());
        let line = &text[pos..eol];
        match &fence {
            Some(f) => {
                if fence_closes(line, f) {
                    fence = None;
                }
            }
            None => {
                if let Some(name) = managed(line) {
                    return Err(format!("{what}: reserved heading ## {name}"));
                }
                fence = fence_opens(line);
            }
        }
        pos = eol + 1;
    }
    if fence.is_some() {
        return Err(format!("{what}: unclosed code fence"));
    }
    Ok(())
}

fn strip_section_gap(s: &str) -> &str {
    s.strip_suffix("\n\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

pub fn decode(markdown: &str) -> Result<Issue, String> {
    merge_markers(markdown)?;
    let text = markdown
        .strip_prefix("---\n")
        .ok_or("missing frontmatter fence")?;
    let end = text.find("\n---\n").ok_or("missing frontmatter fence")?;
    let fm = &text[..end];
    let rest = &text[end + 5..];

    let mut fields: Vec<(String, Value)> = Vec::new();
    for line in fm.split('\n') {
        if line.trim().is_empty() {
            continue;
        }
        let (key, raw) = line
            .split_once(':')
            .ok_or_else(|| format!("malformed frontmatter line {line:?}"))?;
        let key = key.trim();
        if !KEYS.contains(&key) {
            return Err(format!("unknown frontmatter key {key:?}"));
        }
        if fields.iter().any(|(k, _)| k == key) {
            return Err(format!("duplicate frontmatter key {key:?}"));
        }
        fields.push((key.to_string(), parse_value(raw, key)?));
    }
    for want in KEYS {
        if !fields.iter().any(|(k, _)| k == want) {
            return Err(format!("missing frontmatter key {want:?}"));
        }
    }
    let get = |name: &str| fields.iter().find(|(k, _)| k == name).unwrap().1.clone();

    let id = want_str(get("id"), "id")?;
    let title = want_str(get("title"), "title")?;
    let status_raw = match get("status") {
        Value::Str(s) => s,
        _ => return Err("field status must be a string".to_string()),
    };
    let status = Status::parse(&status_raw).ok_or("invalid status value")?;
    let Value::Priority(priority) = get("priority") else {
        return Err(BAD_PRIORITY.to_string());
    };
    let parent = want_opt(get("parent"), "parent")?;
    let depends_on = match get("depends_on") {
        Value::List(items) => items,
        _ => return Err("field depends_on must be a list".to_string()),
    };
    let assignee = want_opt(get("assignee"), "assignee")?;
    let defer_until = want_opt(get("defer_until"), "defer_until")?;
    let created_at = want_opt(get("created_at"), "created_at")?;
    let updated_at = want_opt(get("updated_at"), "updated_at")?;
    let closed_at = want_opt(get("closed_at"), "closed_at")?;
    let release = want_opt(get("release"), "release")?;
    let claim_id = want_opt(get("claim_id"), "claim_id")?;

    let mut pos = usize::from(rest.starts_with('\n'));
    let mut closing_summary = None;
    if managed(next_line(rest, pos)) == Some("Closing Summary") {
        pos = line_end(rest, pos);
        if rest[pos..].starts_with('\n') {
            pos += 1;
        }
        let mut parts = Vec::new();
        loop {
            let line = next_line(rest, pos);
            if !line.starts_with('>') {
                break;
            }
            parts.push(
                line.strip_prefix("> ")
                    .or_else(|| line.strip_prefix('>'))
                    .unwrap_or(line),
            );
            pos = line_end(rest, pos);
        }
        if parts.is_empty() {
            return Err("closing summary without blockquote".to_string());
        }
        closing_summary = Some(parts.join("\n"));
        if rest[pos..].starts_with('\n') {
            pos += 1;
        }
    }

    let hist_at = scan_boundary(rest, pos, "History")?;
    let body = strip_section_gap(&rest[pos..hist_at]).to_string();
    let after_hist = line_end(rest, hist_at);
    let notes_at = scan_boundary(rest, after_hist, "Notes")?;
    let history_raw = &rest[after_hist..notes_at];
    let history =
        strip_section_gap(history_raw.strip_prefix('\n').unwrap_or(history_raw)).to_string();
    let after_notes = line_end(rest, notes_at);
    structural_check(&rest[after_notes..], "notes")?;
    let notes_raw = &rest[after_notes..];
    let notes = strip_section_gap(notes_raw.strip_prefix('\n').unwrap_or(notes_raw)).to_string();

    let issue = Issue {
        id,
        title,
        status,
        priority,
        parent,
        depends_on,
        assignee,
        defer_until,
        created_at,
        updated_at,
        closed_at,
        release,
        claim_id,
        body,
        history,
        notes,
        closing_summary,
    };
    validate(&issue)?;
    Ok(issue)
}

#[cfg(test)]
mod tests {
    use super::{Issue, Status, decode, encode, id_alias, valid_id};
    use crate::text::TextKind;

    fn open_issue(body: &str) -> Issue {
        Issue {
            id: "B-abc123".into(),
            title: "t".into(),
            status: Status::Open,
            priority: 2,
            parent: None,
            depends_on: Vec::new(),
            assignee: None,
            defer_until: None,
            created_at: Some("2026-01-02T03:04:05Z".into()),
            updated_at: None,
            closed_at: None,
            release: None,
            claim_id: None,
            body: body.into(),
            history: String::new(),
            notes: String::new(),
            closing_summary: None,
        }
    }

    #[test]
    fn frontmatter_scalars_are_strict_and_name_the_field() {
        let text = encode(&open_issue(
            "para\n\n```\n## History\n## Notes\n```\n\n> ## Notes\n\ntail",
        ))
        .unwrap();
        assert_eq!(
            decode(&text).unwrap().body,
            "para\n\n```\n## History\n## Notes\n```\n\n> ## Notes\n\ntail",
            "fenced and blockquoted reserved headings stay body"
        );
        for (field, value) in [
            ("title", "plain #tag"),
            ("title", "'x'"),
            ("title", "x # comment"),
            ("title", "|"),
            ("title", "\"a\\zb\""),
            ("title", "\"a\" # c"),
            ("title", "[B1]"),
            ("parent", "B440"),
            ("created_at", "~"),
            ("created_at", "2020-05-04"),
            ("claim_id", "'x'"),
            ("depends_on", "[B1]"),
        ] {
            let line = text
                .lines()
                .find(|l| l.starts_with(&format!("{field}:")))
                .unwrap();
            let broken = text.replacen(line, &format!("{field}: {value}"), 1);
            let error = decode(&broken).unwrap_err();
            assert!(
                error.contains(field),
                "{field}: {value} must name the field, got {error}"
            );
        }
        let escaped = text.replace("title: \"t\"", "title: \"a\\u0142b\"");
        assert_eq!(decode(&escaped).unwrap().title, "a\u{0142}b");
        let legacy = text
            .replace("id: \"B-abc123\"", "id: \"B440.2\"")
            .replace("parent: null", "parent: \"B440\"")
            .replace(
                "created_at: \"2026-01-02T03:04:05Z\"",
                "created_at: \"2020-05-04\"",
            );
        let issue = decode(&legacy).unwrap();
        assert_eq!((issue.id.as_str(), issue.priority), ("B440.2", 2));
        assert_eq!(issue.created_at.as_deref(), Some("2020-05-04"));
    }

    #[test]
    fn fields_are_read_in_any_order_and_written_canonically() {
        let canonical = encode(&open_issue("body")).unwrap();
        let mut lines: Vec<&str> = canonical.lines().collect();
        lines.swap(1, 2);
        lines.swap(4, 12);
        let reordered = format!("{}\n", lines.join("\n"));
        assert_ne!(reordered, canonical);
        let issue = decode(&reordered).unwrap();
        assert_eq!(issue, open_issue("body"), "a reordered file is valid");
        assert_eq!(
            encode(&issue).unwrap(),
            canonical,
            "the next write restores the canonical order"
        );
    }

    #[test]
    fn ids_and_aliases() {
        for id in ["B-abcdef", "B-abcdef.2.10", "B440"] {
            assert!(valid_id(id), "{id}");
        }
        for id in ["B-abcdef.0", "B-abcdef.02", "abcdef", "B-ABC123"] {
            assert!(!valid_id(id), "{id}");
        }
        assert_eq!(id_alias("abcdef").as_deref(), Some("B-abcdef"));
        assert_eq!(id_alias("abcdef.2").as_deref(), Some("B-abcdef.2"));
    }

    #[test]
    fn structural_markers_round_trip_or_refuse() {
        let accepted = [
            "conflict quoted in fence\n\n```\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> branch\n```\n",
            "tilde fence\n~~~\n<<<<<<< h\n=======\n>>>>>>> j\n~~~\n",
            "setext\n=======\n\n>>>>>>> stray end token\n",
            "eight\n<<<<<<<<\n========\n>>>>>>>>\n",
            "crlf\ta\r\n\r\nb\r\n",
            "unicode “quotes” 'single' \"double\" żółć\n\nblank\n\n---\n",
            "fenced reserved\n```\n  ## History ###\n```\n",
            "indented code\n    ## Notes\n\n\t## Closing Summary\n",
            "quoted reserved\n> ## Notes\n",
            "conflict spanning fence\n<<<<<<< x\n=======\n```\ninside\n```\n>>>>>>> y\n",
            "\n````md\n```\n## History\n```\n````\n\ntrailing\n\n",
        ];
        for (n, body) in accepted.iter().enumerate() {
            assert_eq!(
                TextKind::Body.check("body", false, body),
                Ok(()),
                "case {n}"
            );
            let issue = open_issue(body);
            assert_eq!(
                decode(&encode(&issue).unwrap()).unwrap(),
                issue,
                "case {n} round trip"
            );
        }
        let refused = [
            "real conflict\n\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> branch\n",
            "indented reserved\n  ## History\n",
            "closing hashes\n## Notes ###\n",
            "premature end then triplet\n>>>>>>> z\n\n<<<<<<< x\n=======\n>>>>>>> y\n",
            "text\n\n## History\n",
            "ok\n\n```\nunclosed",
        ];
        for (n, body) in refused.iter().enumerate() {
            assert!(
                TextKind::Body.check("body", false, body).is_err(),
                "case {n}"
            );
            assert!(
                encode(&open_issue(body)).is_err(),
                "case {n}: the codec refuses too"
            );
        }
    }
}
