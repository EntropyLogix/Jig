use crate::check;
use crate::clock;
use crate::config;
use crate::domain::{self, Action};
use crate::ident;
use crate::instructions;
use crate::integration;
use crate::issue::{self, Issue, Status};
use crate::memory;
use crate::names::{Names, Resolved};
use crate::store::{self, Loaded, Place};
use crate::text::TextKind;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct Fail {
    pub code: i32,
    pub msg: String,
}

type R<T> = Result<T, Fail>;

fn fail(msg: impl Into<String>) -> Fail {
    Fail {
        code: 1,
        msg: msg.into(),
    }
}

fn usage(msg: impl Into<String>) -> Fail {
    Fail {
        code: 2,
        msg: msg.into(),
    }
}

#[derive(Clone, Copy)]
pub enum ArgType {
    Text(TextKind),
    Boolean,
    Integer,
    TextList(TextKind),
    Enum(&'static [&'static str]),
}

#[derive(Clone, Copy)]
pub struct ArgSpec {
    pub name: &'static str,
    pub kind: ArgType,
    pub required: bool,
    pub positional: bool,
    pub cli: Option<&'static str>,
    pub alias: Option<&'static str>,
    pub mcp: bool,
}

impl ArgSpec {
    pub fn field(&self) -> &'static str {
        self.cli.unwrap_or(self.name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HelpGroup {
    Browse,
    CreateAndEdit,
    Work,
    Decide,
    Memory,
    Setup,
}

impl HelpGroup {
    const ALL: [HelpGroup; 6] = [
        HelpGroup::Browse,
        HelpGroup::CreateAndEdit,
        HelpGroup::Work,
        HelpGroup::Decide,
        HelpGroup::Memory,
        HelpGroup::Setup,
    ];

    fn title(self) -> &'static str {
        match self {
            HelpGroup::Browse => "Browse",
            HelpGroup::CreateAndEdit => "Create and edit",
            HelpGroup::Work => "Work (agents)",
            HelpGroup::Decide => "Decide (people)",
            HelpGroup::Memory => "Memory",
            HelpGroup::Setup => "Setup",
        }
    }
}

pub struct CommandSpec {
    pub name: &'static str,
    pub group: HelpGroup,
    pub syntax: &'static str,
    pub description: &'static str,
    pub args: &'static [ArgSpec],
    pub handler: Handler,
    pub decision: bool,
    pub mcp: bool,
}

pub type Handler = fn(&[String], &Path, bool, bool) -> R<CommandOutput>;

const EMPTY_ARGS: &[ArgSpec] = &[];
const STATUS_VALUES: &[&str] = &["proposed", "open", "claimed", "review", "closed", "all"];
const SORT_VALUES: &[&str] = &["oldest", "newest"];
const DEP_ACTIONS: &[&str] = &["add", "rm"];

const ARGS_INIT: &[ArgSpec] = &[
    ArgSpec {
        name: "no_hooks",
        kind: ArgType::Boolean,
        required: false,
        positional: false,
        cli: Some("--no-hooks"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "prefix",
        kind: ArgType::Text(TextKind::Prefix),
        required: false,
        positional: false,
        cli: Some("--prefix"),
        alias: None,
        mcp: true,
    },
];
const ARGS_START: &[ArgSpec] = &[ArgSpec {
    name: "hook_json",
    kind: ArgType::Boolean,
    required: false,
    positional: false,
    cli: Some("--hook-json"),
    alias: None,
    mcp: false,
}];
const ARGS_ADD: &[ArgSpec] = &[
    ArgSpec {
        name: "title",
        kind: ArgType::Text(TextKind::Label),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "approved",
        kind: ArgType::Boolean,
        required: false,
        positional: false,
        cli: Some("--approved"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "priority",
        kind: ArgType::Integer,
        required: false,
        positional: false,
        cli: Some("--priority"),
        alias: Some("-p"),
        mcp: true,
    },
    ArgSpec {
        name: "body",
        kind: ArgType::Text(TextKind::Body),
        required: false,
        positional: false,
        cli: Some("--body"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "parent",
        kind: ArgType::Text(TextKind::Id),
        required: false,
        positional: false,
        cli: Some("--parent"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "dep",
        kind: ArgType::TextList(TextKind::Id),
        required: false,
        positional: false,
        cli: Some("--dep"),
        alias: None,
        mcp: true,
    },
];
const ARGS_SHOW: &[ArgSpec] = &[ArgSpec {
    name: "id",
    kind: ArgType::Text(TextKind::Id),
    required: true,
    positional: true,
    cli: None,
    alias: None,
    mcp: true,
}];
const ARGS_RELEASE: &[ArgSpec] = &[ArgSpec {
    name: "name",
    kind: ArgType::Text(TextKind::Release),
    required: true,
    positional: true,
    cli: None,
    alias: None,
    mcp: true,
}];
const ARGS_LIST: &[ArgSpec] = &[
    ArgSpec {
        name: "status",
        kind: ArgType::Enum(STATUS_VALUES),
        required: false,
        positional: false,
        cli: Some("--status"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "release",
        kind: ArgType::Text(TextKind::Release),
        required: false,
        positional: false,
        cli: Some("--release"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "sort",
        kind: ArgType::Enum(SORT_VALUES),
        required: false,
        positional: false,
        cli: Some("--sort"),
        alias: None,
        mcp: true,
    },
];
const ARGS_CLAIM: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "by",
        kind: ArgType::Text(TextKind::Label),
        required: false,
        positional: false,
        cli: Some("--by"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "force",
        kind: ArgType::Boolean,
        required: false,
        positional: false,
        cli: Some("--force"),
        alias: None,
        mcp: true,
    },
];
const ARGS_UPDATE: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "title",
        kind: ArgType::Text(TextKind::Label),
        required: false,
        positional: false,
        cli: Some("--title"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "priority",
        kind: ArgType::Integer,
        required: false,
        positional: false,
        cli: Some("--priority"),
        alias: Some("-p"),
        mcp: true,
    },
    ArgSpec {
        name: "body",
        kind: ArgType::Text(TextKind::Body),
        required: false,
        positional: false,
        cli: Some("--body"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "release",
        kind: ArgType::Text(TextKind::Release),
        required: false,
        positional: false,
        cli: Some("--release"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "number",
        kind: ArgType::Text(TextKind::Number),
        required: false,
        positional: false,
        cli: Some("--number"),
        alias: None,
        mcp: true,
    },
];
const ARGS_REVIEW: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "summary",
        kind: ArgType::Text(TextKind::Multiline),
        required: false,
        positional: false,
        cli: Some("--summary"),
        alias: None,
        mcp: true,
    },
];
const ARGS_REASON: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "reason",
        kind: ArgType::Text(TextKind::Multiline),
        required: true,
        positional: false,
        cli: Some("--reason"),
        alias: None,
        mcp: true,
    },
];
const ARGS_CLOSE: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "reason",
        kind: ArgType::Text(TextKind::Multiline),
        required: true,
        positional: false,
        cli: Some("--reason"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "summary",
        kind: ArgType::Text(TextKind::Multiline),
        required: false,
        positional: false,
        cli: Some("--summary"),
        alias: None,
        mcp: true,
    },
];
const ARGS_DEP: &[ArgSpec] = &[
    ArgSpec {
        name: "action",
        kind: ArgType::Enum(DEP_ACTIONS),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "blocker",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
];
const ARGS_NOTE: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "text",
        kind: ArgType::Text(TextKind::Note),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "by",
        kind: ArgType::Text(TextKind::Label),
        required: false,
        positional: false,
        cli: Some("--by"),
        alias: None,
        mcp: true,
    },
];
const ARGS_DEFER: &[ArgSpec] = &[
    ArgSpec {
        name: "id",
        kind: ArgType::Text(TextKind::Id),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "until",
        kind: ArgType::Text(TextKind::Date),
        required: false,
        positional: false,
        cli: Some("--until"),
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "clear",
        kind: ArgType::Boolean,
        required: false,
        positional: false,
        cli: Some("--clear"),
        alias: None,
        mcp: true,
    },
];
const ARGS_REMEMBER: &[ArgSpec] = &[
    ArgSpec {
        name: "topic",
        kind: ArgType::Text(TextKind::Topic),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
    ArgSpec {
        name: "text",
        kind: ArgType::Text(TextKind::Free),
        required: true,
        positional: true,
        cli: None,
        alias: None,
        mcp: true,
    },
];
const ARGS_RECALL: &[ArgSpec] = &[ArgSpec {
    name: "topic",
    kind: ArgType::Text(TextKind::Free),
    required: false,
    positional: true,
    cli: None,
    alias: None,
    mcp: true,
}];

struct Parsed {
    pos: Vec<String>,
    opts: HashMap<String, Vec<String>>,
    flags: HashSet<String>,
}

#[derive(Clone, Copy)]
enum Arity {
    Switch,
    One,
    Many,
}

fn parse_tail(args: &[String], spec: &[(&str, Arity, &str)], repeat: &[&str]) -> R<Parsed> {
    let mut p = Parsed {
        pos: Vec::new(),
        opts: HashMap::new(),
        flags: HashSet::new(),
    };
    let mut i = 0;
    let mut literal = false;
    while i < args.len() {
        let a = &args[i];
        if literal {
            p.pos.push(a.clone());
            i += 1;
            continue;
        }
        if a == "--" {
            literal = true;
            i += 1;
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            let Some((_, arity, canon)) = spec.iter().find(|(s, _, _)| *s == a.as_str()) else {
                return Err(usage(format!("unknown option {a}")));
            };
            match arity {
                Arity::Switch => {
                    if !p.flags.insert(canon.to_string()) {
                        return Err(usage(format!("duplicate option {a}")));
                    }
                }
                Arity::One => {
                    i += 1;
                    if i >= args.len() {
                        return Err(usage(format!("missing value for {a}")));
                    }
                    let entry = p.opts.entry(canon.to_string()).or_default();
                    if !repeat.contains(canon) && !entry.is_empty() {
                        return Err(usage(format!("duplicate option {a}")));
                    }
                    entry.push(args[i].clone());
                }
                Arity::Many => {
                    i += 1;
                    let entry = p.opts.entry(canon.to_string()).or_default();
                    if !repeat.contains(canon) && !entry.is_empty() {
                        return Err(usage(format!("duplicate option {a}")));
                    }
                    let mut taken = 0usize;
                    while i < args.len()
                        && (taken == 0
                            || (args[i] != "--"
                                && !(args[i].len() > 1 && args[i].starts_with('-'))))
                    {
                        entry.push(args[i].clone());
                        taken += 1;
                        i += 1;
                    }
                    if taken == 0 {
                        return Err(usage(format!("missing value for {a}")));
                    }
                    continue;
                }
            }
            i += 1;
            continue;
        }
        p.pos.push(a.clone());
        i += 1;
    }
    Ok(p)
}

fn parse_command_tail(args: &[String], name: &str) -> R<Parsed> {
    let command = COMMANDS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| usage(format!("unknown command {name}")))?;
    let mut spec = Vec::new();
    let mut repeat = Vec::new();
    if JSON_CMDS.contains(&name) {
        spec.push(("--json", Arity::Switch, "json"));
    }
    for arg in command.args {
        let Some(option) = arg.cli else {
            continue;
        };
        let arity = match arg.kind {
            ArgType::Boolean => Arity::Switch,
            ArgType::TextList(_) => {
                repeat.push(arg.name);
                Arity::Many
            }
            ArgType::Text(_) | ArgType::Integer | ArgType::Enum(_) => Arity::One,
        };
        spec.push((option, arity, arg.name));
        if let Some(alias) = arg.alias {
            spec.push((alias, arity, arg.name));
        }
    }
    let parsed = parse_tail(args, &spec, &repeat)?;
    check_text_args(command, &parsed)?;
    Ok(parsed)
}

fn check_text_args(command: &CommandSpec, p: &Parsed) -> R<()> {
    let positional: Vec<&ArgSpec> = command.args.iter().filter(|arg| arg.positional).collect();
    let needed = positional.iter().filter(|arg| arg.required).count();
    let fits = (needed..=positional.len()).contains(&p.pos.len());
    for arg in command.args {
        let (ArgType::Text(kind) | ArgType::TextList(kind)) = arg.kind else {
            continue;
        };
        let values: Vec<&String> = if arg.positional {
            let index = positional.iter().position(|other| other.name == arg.name);
            match index.and_then(|index| p.pos.get(index)) {
                Some(value) if fits => vec![value],
                _ => Vec::new(),
            }
        } else {
            p.opts.get(arg.name).into_iter().flatten().collect()
        };
        for value in values {
            kind.check(arg.field(), arg.required, value)
                .map_err(usage)?;
        }
    }
    Ok(())
}

fn opt<'a>(p: &'a Parsed, name: &str) -> Option<&'a str> {
    p.opts.get(name).and_then(|v| v.first()).map(String::as_str)
}

fn flag(p: &Parsed, name: &str) -> bool {
    p.flags.contains(name)
}

fn pos_n(p: &Parsed, n: usize, what: &str) -> R<()> {
    if p.pos.len() != n {
        return Err(usage(format!("expected {what}")));
    }
    Ok(())
}

fn registry(cwd: &Path) -> R<store::Lock> {
    let billet = store::discover(cwd).map_err(fail)?;
    store::lock(&billet).map_err(fail)
}

fn with_registry<T>(cwd: &Path, mut read: impl FnMut(&Path) -> R<T>) -> R<T> {
    let billet = store::discover(cwd).map_err(fail)?;
    store::with_read(&billet, store::Recovery::Allow, || read(&billet)).map_err(fail)?
}

fn names(billet: &Path) -> R<Names> {
    snapshot_names(billet, &check::scan(billet))
}

fn snapshot_names(billet: &Path, snap: &check::Snapshot) -> R<Names> {
    let config = config::load(billet).map_err(fail)?;
    Ok(snap.names(&config.prefix))
}

fn joined(names: &Names, keys: &[String]) -> String {
    keys.iter()
        .map(|key| names.name(key))
        .collect::<Vec<_>>()
        .join(", ")
}

fn duplicate_warnings(names: &Names) -> Vec<String> {
    names
        .duplicates()
        .map(|(number, roots)| {
            let paths = roots
                .iter()
                .map(|root| root.path.display().to_string())
                .collect::<Vec<_>>()
                .join(" and ");
            format!(
                "number {}{number} is used by {paths}; run `blt check`",
                names.prefix
            )
        })
        .collect()
}

fn resolve(names: &Names, raw: &str) -> R<String> {
    let reference = issue::parse_reference(raw, &names.prefix)
        .ok_or_else(|| usage(format!("invalid issue reference {raw:?}")))?;
    match names.resolve(&reference) {
        Resolved::Key(key) => Ok(key),
        Resolved::Missing(shown) => Err(fail(format!("issue {shown} not found"))),
        Resolved::Ambiguous(shown, roots) => {
            let matches = roots
                .iter()
                .map(|root| format!("{} {}", root.path.display(), issue::esc(&root.title)))
                .collect::<Vec<_>>()
                .join(", ");
            Err(fail(format!(
                "{shown} is ambiguous: {matches}; run `blt check`"
            )))
        }
    }
}

fn load(billet: &Path, names: &Names, raw: &str) -> R<Loaded> {
    let key = resolve(names, raw)?;
    store::find(billet, &key)
        .map_err(fail)?
        .ok_or_else(|| fail(format!("issue {} not found", names.name(&key))))
}

fn read_stdin() -> R<String> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| fail(e.to_string()))?;
    Ok(s)
}

fn body_arg(p: &Parsed, allow_stdin: bool) -> R<Option<String>> {
    match opt(p, "body") {
        Some("-") if allow_stdin => {
            let body = read_stdin()?;
            TextKind::Body.check("body", false, &body).map_err(usage)?;
            Ok(Some(body))
        }
        Some(t) => Ok(Some(t.to_string())),
        None => Ok(None),
    }
}

fn state_dir() -> R<PathBuf> {
    if let Ok(x) = std::env::var("XDG_STATE_HOME")
        && !x.is_empty()
    {
        return Ok(PathBuf::from(x));
    }
    if let Ok(h) = std::env::var("HOME")
        && !h.is_empty()
    {
        return Ok(Path::new(&h).join(".local/state"));
    }
    Err(fail("cannot locate a state directory"))
}

fn hist_push(issue: &mut Issue, entry: String) {
    if !issue.history.is_empty() {
        issue.history.push('\n');
    }
    issue.history.push_str(&entry);
}

fn hist_entry(what: &str) -> String {
    format!("- {} {}", clock::now_stamp(), what)
}

fn history_text(label: &str, text: &str) -> String {
    if !text.contains('\n') {
        return format!("{label} {text}");
    }
    let mut out = label.to_string();
    for line in text.split('\n') {
        out.push_str("\n  > ");
        out.push_str(line);
    }
    out
}

fn parse_priority(v: &str) -> R<u8> {
    match v.as_bytes() {
        [p @ b'0'..=b'4'] => Ok(p - b'0'),
        _ => Err(usage(format!("invalid priority {v:?}; expected 0..=4"))),
    }
}

fn jopt(v: &Option<String>) -> String {
    match v {
        Some(s) => issue::esc(s),
        None => "null".to_string(),
    }
}

fn row_json(names: &Names, i: &Issue) -> String {
    format!(
        "{{\"id\":{},\"key\":{},\"title\":{},\"status\":{},\"priority\":{},\"assignee\":{}}}",
        issue::esc(&names.name(&i.id)),
        issue::esc(&i.id),
        issue::esc(&i.title),
        issue::esc(i.status.as_str()),
        i.priority,
        jopt(&i.assignee)
    )
}

fn sort_rows(rows: &mut [Issue]) {
    rows.sort_by(|a, b| {
        (
            a.priority,
            a.created_at.is_none(),
            a.created_at.clone().unwrap_or_default(),
            &a.id,
        )
            .cmp(&(
                b.priority,
                b.created_at.is_none(),
                b.created_at.clone().unwrap_or_default(),
                &b.id,
            ))
    });
}

#[derive(Clone, Copy)]
enum CreatedOrder {
    Oldest,
    Newest,
}

fn sort_chronologically(rows: &mut [Issue], order: CreatedOrder) {
    rows.sort_by(|a, b| {
        let by_date = match (a.created_at.as_deref(), b.created_at.as_deref()) {
            (Some(a), Some(b)) => match order {
                CreatedOrder::Oldest => a.cmp(b),
                CreatedOrder::Newest => b.cmp(a),
            },
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_date.then_with(|| a.id.cmp(&b.id))
    });
}

pub struct CommandOutput {
    pub stdout: String,
    pub warnings: Vec<String>,
    pub code: i32,
}

impl CommandOutput {
    fn plain(stdout: String) -> CommandOutput {
        CommandOutput {
            stdout,
            warnings: Vec::new(),
            code: 0,
        }
    }
}

pub const JSON_CMDS: [&str; 7] = ["start", "ready", "show", "list", "tree", "recall", "check"];

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "list",
        group: HelpGroup::Browse,
        syntax: "list [--status S] [--release NAME] [--sort oldest|newest]",
        description: "issues grouped by status (default)",
        args: ARGS_LIST,
        handler: handle_list,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "show",
        group: HelpGroup::Browse,
        syntax: "show ID",
        description: "one full issue and its documents",
        args: ARGS_SHOW,
        handler: handle_show,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "tree",
        group: HelpGroup::Browse,
        syntax: "tree ID",
        description: "issue with its subtasks",
        args: ARGS_SHOW,
        handler: handle_tree,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "ready",
        group: HelpGroup::Browse,
        syntax: "ready",
        description: "open, unblocked work for agents",
        args: EMPTY_ARGS,
        handler: handle_ready,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "add",
        group: HelpGroup::CreateAndEdit,
        syntax: "add TITLE [--approved] [-p N] [--body TEXT|-] [--parent ID] [--dep ID...]",
        description: "new issue (proposed unless --approved)",
        args: ARGS_ADD,
        handler: handle_add,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "update",
        group: HelpGroup::CreateAndEdit,
        syntax: "update ID [--title T] [-p N] [--body TEXT|-] [--release NAME] [--number N]",
        description: "change the given fields",
        args: ARGS_UPDATE,
        handler: handle_update,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "note",
        group: HelpGroup::CreateAndEdit,
        syntax: "note ID TEXT [--by NAME]",
        description: "one-line note",
        args: ARGS_NOTE,
        handler: handle_note,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "dep",
        group: HelpGroup::CreateAndEdit,
        syntax: "dep add|rm ID BLOCKER",
        description: "manage blockers",
        args: ARGS_DEP,
        handler: handle_dep,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "defer",
        group: HelpGroup::CreateAndEdit,
        syntax: "defer ID --until YYYY-MM-DD | --clear",
        description: "hide an issue until a date",
        args: ARGS_DEFER,
        handler: handle_defer,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "claim",
        group: HelpGroup::Work,
        syntax: "claim ID [--by NAME] [--force]",
        description: "start work on an open issue",
        args: ARGS_CLAIM,
        handler: handle_claim,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "unclaim",
        group: HelpGroup::Work,
        syntax: "unclaim ID",
        description: "give a claimed issue back",
        args: ARGS_SHOW,
        handler: handle_unclaim,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "review",
        group: HelpGroup::Work,
        syntax: "review ID [--summary TEXT]",
        description: "submit for review",
        args: ARGS_REVIEW,
        handler: handle_review,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "approve",
        group: HelpGroup::Decide,
        syntax: "approve ID",
        description: "proposed -> open",
        args: ARGS_SHOW,
        handler: handle_approve,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "revise",
        group: HelpGroup::Decide,
        syntax: "revise ID --reason TEXT",
        description: "open -> proposed",
        args: ARGS_REASON,
        handler: handle_revise,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "rework",
        group: HelpGroup::Decide,
        syntax: "rework ID --reason TEXT",
        description: "review -> claimed",
        args: ARGS_REASON,
        handler: handle_rework,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "close",
        group: HelpGroup::Decide,
        syntax: "close ID --reason TEXT [--summary TEXT]",
        description: "archive an issue",
        args: ARGS_CLOSE,
        handler: handle_close,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "reopen",
        group: HelpGroup::Decide,
        syntax: "reopen ID --reason TEXT",
        description: "reactivate a closed issue",
        args: ARGS_REASON,
        handler: handle_reopen,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "release",
        group: HelpGroup::Decide,
        syntax: "release NAME",
        description: "seal closed issues with a release name",
        args: ARGS_RELEASE,
        handler: handle_release,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "remember",
        group: HelpGroup::Memory,
        syntax: "remember TOPIC TEXT",
        description: "record a project fact",
        args: ARGS_REMEMBER,
        handler: handle_remember,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "recall",
        group: HelpGroup::Memory,
        syntax: "recall [TOPIC]",
        description: "read project facts",
        args: ARGS_RECALL,
        handler: handle_recall,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "init",
        group: HelpGroup::Setup,
        syntax: "init [--no-hooks] [--prefix P]",
        description: "create .billet/, AGENTS.md and project hooks",
        args: ARGS_INIT,
        handler: handle_init,
        decision: true,
        mcp: true,
    },
    CommandSpec {
        name: "start",
        group: HelpGroup::Setup,
        syntax: "start",
        description: "workflow summary for agents",
        args: ARGS_START,
        handler: handle_start,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "check",
        group: HelpGroup::Setup,
        syntax: "check",
        description: "report registry problems",
        args: EMPTY_ARGS,
        handler: handle_check,
        decision: false,
        mcp: true,
    },
    CommandSpec {
        name: "mcp",
        group: HelpGroup::Setup,
        syntax: "mcp",
        description: "MCP server over stdio (started by the client)",
        args: EMPTY_ARGS,
        handler: cmd_mcp,
        decision: false,
        mcp: false,
    },
];

pub(crate) struct McpToolSpec {
    pub(crate) name: &'static str,
    pub(crate) command: &'static CommandSpec,
    pub(crate) action: Option<&'static str>,
    pub(crate) description: &'static str,
}

pub(crate) fn mcp_tools() -> Vec<McpToolSpec> {
    COMMANDS
        .iter()
        .filter(|spec| spec.mcp)
        .flat_map(|spec| {
            if spec.name == "dep" {
                vec![
                    McpToolSpec {
                        name: "dep_add",
                        command: spec,
                        action: Some("add"),
                        description: "add a blocker",
                    },
                    McpToolSpec {
                        name: "dep_rm",
                        command: spec,
                        action: Some("rm"),
                        description: "remove a blocker",
                    },
                ]
            } else {
                vec![McpToolSpec {
                    name: spec.name,
                    command: spec,
                    action: None,
                    description: spec.description,
                }]
            }
        })
        .collect()
}

pub fn command_names() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|spec| spec.name)
}

fn help_text() -> String {
    let mut out = String::from(
        "Billet - file-based issue tracker and project memory\n\nUsage: blt [--json] COMMAND [ARGS]\n\n",
    );
    for (index, group) in HelpGroup::ALL.iter().enumerate() {
        out.push_str(group.title());
        out.push('\n');
        for spec in COMMANDS.iter().filter(|spec| spec.group == *group) {
            let line = format!("  {}", spec.syntax);
            if line.len() <= 34 && line.len() + 2 + spec.description.len() <= 80 {
                out.push_str(&format!("{line:<34}{}\n", spec.description));
            } else {
                out.push_str(&line);
                out.push('\n');
                out.push_str(&format!("{:34}{}\n", "", spec.description));
            }
        }
        if index + 1 < HelpGroup::ALL.len() {
            out.push('\n');
        }
    }
    out.push_str("\nOptions\n  --json             machine output for read commands\n  -p, --priority N   0 (highest) to 4, default 2\n  S                  proposed|open|claimed|review|closed|all\n  --                 end of options; what follows is text\n  --help, --version");
    out
}

pub fn run(args: &[String], cwd: &Path) -> Result<CommandOutput, Fail> {
    run_inner(args, cwd, true)
}

pub(crate) fn run_mcp(args: &[String], cwd: &Path) -> Result<CommandOutput, Fail> {
    run_inner(args, cwd, false)
}

fn run_inner(args: &[String], cwd: &Path, allow_stdin: bool) -> Result<CommandOutput, Fail> {
    if args.is_empty() {
        return Ok(CommandOutput::plain(format!("{}\n", help_text())));
    }
    let mut json = false;
    let mut i = 0usize;
    let cmd = loop {
        let Some(a) = args.get(i) else {
            return Err(usage("missing command; try `blt --help`"));
        };
        match a.as_str() {
            "--json" if json => return Err(usage("duplicate option --json")),
            "--json" => {
                json = true;
                i += 1;
            }
            "--" => {
                i += 1;
                let Some(a) = args.get(i) else {
                    return Err(usage("missing command; try `blt --help`"));
                };
                break a.to_string();
            }
            "--help" | "-h" | "help" => {
                return Ok(CommandOutput::plain(format!("{}\n", help_text())));
            }
            "--version" => {
                return Ok(CommandOutput::plain(format!(
                    "blt {}\n",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(usage(format!("unknown option {s}")));
            }
            s => break s.to_string(),
        };
    };
    let tail: Vec<String> = args[i + 1..].to_vec();
    if json && !JSON_CMDS.contains(&cmd.as_str()) {
        return Err(usage(format!("--json is not valid for `{cmd}`")));
    }
    let Some(spec) = COMMANDS.iter().find(|spec| spec.name == cmd) else {
        return Err(usage(format!("unknown command {cmd}; try `blt --help`")));
    };
    (spec.handler)(&tail, cwd, json, allow_stdin)
}

fn handle_init(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_init(args, cwd).map(CommandOutput::plain)
}

fn handle_ready(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_ready(args, cwd, json)
}

fn handle_check(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_check(args, cwd, json)
}

fn handle_start(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_start(args, cwd, json).map(CommandOutput::plain)
}

fn handle_show(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_show(args, cwd, json)
}

fn handle_list(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_list(args, cwd, json)
}

fn handle_tree(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_tree(args, cwd, json)
}

fn handle_add(args: &[String], cwd: &Path, _: bool, allow_stdin: bool) -> R<CommandOutput> {
    cmd_add(args, cwd, allow_stdin).map(CommandOutput::plain)
}

fn handle_approve(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_approve(args, cwd).map(CommandOutput::plain)
}

fn handle_revise(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_revise(args, cwd).map(CommandOutput::plain)
}

fn handle_claim(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_claim(args, cwd).map(CommandOutput::plain)
}

fn handle_unclaim(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_unclaim(args, cwd).map(CommandOutput::plain)
}

fn handle_update(args: &[String], cwd: &Path, _: bool, allow_stdin: bool) -> R<CommandOutput> {
    cmd_update(args, cwd, allow_stdin).map(CommandOutput::plain)
}

fn handle_review(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_review(args, cwd).map(CommandOutput::plain)
}

fn handle_rework(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_rework(args, cwd).map(CommandOutput::plain)
}

fn handle_close(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_close(args, cwd).map(CommandOutput::plain)
}

fn handle_reopen(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_reopen(args, cwd).map(CommandOutput::plain)
}

fn handle_release(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_release(args, cwd)
}

fn handle_dep(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_dep(args, cwd).map(CommandOutput::plain)
}

fn handle_note(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_note(args, cwd).map(CommandOutput::plain)
}

fn handle_defer(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_defer(args, cwd).map(CommandOutput::plain)
}

fn handle_remember(args: &[String], cwd: &Path, _: bool, _: bool) -> R<CommandOutput> {
    cmd_remember(args, cwd).map(CommandOutput::plain)
}

fn handle_recall(args: &[String], cwd: &Path, json: bool, _: bool) -> R<CommandOutput> {
    cmd_recall(args, cwd, json).map(CommandOutput::plain)
}

fn cmd_mcp(args: &[String], _: &Path, _: bool, _: bool) -> R<CommandOutput> {
    if args.is_empty() {
        return Err(usage("mcp is started by the client"));
    }
    Err(usage("mcp does not accept arguments"))
}

fn cmd_init(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "init")?;
    pos_n(&p, 0, "no positional arguments")?;
    let created = store::init(cwd, opt(&p, "prefix")).map_err(fail)?;
    let _lock = registry(cwd)?;
    let notice = integration::install(cwd, !flag(&p, "no_hooks")).map_err(fail)?;
    let mut out = if created {
        "initialized .billet/\n".to_string()
    } else {
        ".billet/ already initialized\n".to_string()
    };
    if let Some(notice) = notice {
        out.push_str(&notice);
        out.push('\n');
    }
    Ok(out)
}

fn cmd_start(args: &[String], cwd: &Path, json: bool) -> R<String> {
    let p = parse_command_tail(args, "start")?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    if flag(&p, "hook_json") {
        if json {
            return Err(usage("--json and --hook-json are mutually exclusive"));
        }
        store::discover(cwd).map_err(fail)?;
        return Ok(format!(
            "{{\"hookSpecificOutput\":{{\"hookEventName\":\"SessionStart\",\"additionalContext\":{}}}}}\n",
            issue::esc(instructions::WORKFLOW.trim_end_matches('\n'))
        ));
    }
    with_registry(cwd, |_billet| {
        let workflow = instructions::WORKFLOW.trim_end_matches('\n');
        if json {
            Ok(format!("{{\"workflow\":{}}}\n", issue::esc(workflow)))
        } else {
            Ok(format!("{workflow}\n"))
        }
    })
}

fn cmd_add(args: &[String], cwd: &Path, allow_stdin: bool) -> R<String> {
    let p = parse_command_tail(args, "add")?;
    pos_n(&p, 1, "add TITLE")?;
    let title = &p.pos[0];
    let priority = opt(&p, "priority")
        .map(parse_priority)
        .transpose()?
        .unwrap_or(2);
    let body = body_arg(&p, allow_stdin)?.unwrap_or_default();
    let lock = registry(cwd)?;
    let billet = lock.billet.clone();
    let names = names(&billet)?;
    let parent = match opt(&p, "parent") {
        Some(raw) => Some(load(&billet, &names, raw)?.issue.id),
        None => None,
    };
    let mut depends_on = Vec::new();
    for raw in p.opts.get("dep").into_iter().flatten() {
        let id = load(&billet, &names, raw)?.issue.id;
        if depends_on.contains(&id) {
            return Err(usage(format!("duplicate dependency {}", names.name(&id))));
        }
        depends_on.push(id);
    }
    let (id, number) = match &parent {
        Some(p) => (next_child(&billet, p)?, None),
        None => {
            let salt = ident::install_salt(&state_dir()?).map_err(fail)?;
            let key = ident::new_issue_id(&salt, |id| store::exists(&billet, id)).map_err(fail)?;
            (key, Some(names.next_number()))
        }
    };
    let now = clock::now_iso();
    let new = Issue {
        id: id.clone(),
        number,
        title: title.clone(),
        status: if flag(&p, "approved") {
            Status::Open
        } else {
            Status::Proposed
        },
        priority,
        parent,
        depends_on,
        assignee: None,
        defer_until: None,
        created_at: Some(now.clone()),
        updated_at: Some(now),
        closed_at: None,
        release: None,
        claim_id: None,
        body,
        history: String::new(),
        notes: String::new(),
        closing_summary: None,
    };
    store::write_new(&billet, &new).map_err(fail)?;
    let mut names = names;
    names.add(&new, PathBuf::new());
    Ok(format!("{}\n", names.name(&id)))
}

fn next_child(billet: &Path, parent: &str) -> R<String> {
    let prefix = format!("{parent}.");
    let mut max: u64 = 0;
    for place in [Place::Open, Place::Closed] {
        let dir = billet.join(place.dir());
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(fail(format!(
                    "read {}: {e}",
                    store::diagnostic_path(&dir).display()
                )));
            }
        };
        for ent in rd {
            let ent = ent.map_err(|e| fail(e.to_string()))?;
            let Some(name) = ent.file_name().to_str().map(|s| s.to_string()) else {
                continue;
            };
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Some(idx) = rest.strip_suffix(".md") else {
                continue;
            };
            if idx.contains('.') || idx.is_empty() || !idx.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if idx.starts_with('0') {
                return Err(fail(format!("non-canonical child index in {name}")));
            }
            let n: u64 = idx
                .parse()
                .map_err(|_| fail(format!("malformed child index in {name}")))?;
            max = max.max(n);
        }
    }
    let next = max
        .checked_add(1)
        .ok_or_else(|| fail("child index overflow"))?;
    Ok(format!("{parent}.{next}"))
}

fn file_warnings(
    snap: &check::Snapshot,
    include: impl Fn(&Path) -> bool,
    skip_inconsistent: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut warned: HashSet<PathBuf> = HashSet::new();
    for problem in &snap.problems {
        if !include(&problem.path) || !warned.insert(problem.path.clone()) {
            continue;
        }
        warnings.push(format!(
            "skipping {}: {}; run `blt check`",
            problem.path.display(),
            problem.problem
        ));
    }
    let prefix = if skip_inconsistent { "skipping " } else { "" };
    for rec in &snap.records {
        if !include(&rec.path) || warned.contains(&rec.path) {
            continue;
        }
        if let Some((problem, _)) = domain::field_problems(&rec.issue).first() {
            warned.insert(rec.path.clone());
            warnings.push(format!(
                "{prefix}{}: {problem}; run `blt check`",
                rec.path.display()
            ));
        }
    }
    warnings
}

fn cmd_ready(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_command_tail(args, "ready")?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |billet| {
        let snap = check::scan(billet);
        let names = snapshot_names(billet, &snap)?;
        let today = clock::today();
        let mut warnings = file_warnings(&snap, |p| p.starts_with(".billet/open"), true);
        warnings.extend(duplicate_warnings(&names));
        let mut rows = Vec::new();
        for i in snap.issues() {
            if !domain::field_problems(i).is_empty() {
                continue;
            }
            let eligibility = domain::eligibility(
                i,
                &today,
                |id| snap.resolve(id),
                |id| snap.descendants(id),
                |id| snap.unresolved_descendants(id),
            );
            if eligibility.ready() {
                rows.push(i.clone());
            } else if !eligibility.unresolved.is_empty() {
                let unresolved: Vec<String> = eligibility
                    .unresolved
                    .iter()
                    .map(|reference| reference.text(&names))
                    .collect();
                warnings.push(format!(
                    "skipping {}: unresolved {}; run `blt check`",
                    names.name(&i.id),
                    unresolved.join(", ")
                ));
            }
        }
        sort_rows(&mut rows);
        let stdout = if json {
            let items: Vec<String> = rows.iter().map(|i| row_json(&names, i)).collect();
            format!("[{}]\n", items.join(","))
        } else {
            let mut out = String::new();
            for i in &rows {
                out.push_str(&format!(
                    "{} {} {}\n",
                    names.name(&i.id),
                    i.priority,
                    i.title
                ));
            }
            out
        };
        Ok(CommandOutput {
            stdout,
            warnings,
            code: 0,
        })
    })
}

fn cmd_check(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_command_tail(args, "check")?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    let billet = store::discover_raw(cwd).map_err(fail)?;
    let problems = store::with_read(&billet, store::Recovery::ReportOnly, || {
        check::collect(&billet)
    })
    .map_err(fail)?;
    if problems.is_empty() {
        return Ok(CommandOutput::plain(if json {
            "[]\n".to_string()
        } else {
            "registry is clean\n".to_string()
        }));
    }
    let stdout = if json {
        let items: Vec<String> = problems
            .iter()
            .map(|p| {
                format!(
                    "{{\"path\":{},\"problem\":{},\"hint\":{}}}",
                    issue::esc(&p.path.display().to_string()),
                    issue::esc(&p.problem),
                    issue::esc(&p.hint)
                )
            })
            .collect();
        format!("[{}]\n", items.join(","))
    } else {
        let mut out = String::new();
        for p in &problems {
            out.push_str(&format!(
                "{}: {}\n  hint: {}\n",
                p.path.display(),
                p.problem,
                p.hint
            ));
        }
        out
    };
    Ok(CommandOutput {
        stdout,
        warnings: Vec::new(),
        code: 1,
    })
}

fn issue_docs(billet: &Path, id: &str) -> Result<(Vec<String>, Vec<String>), Fail> {
    let docs_root = billet.join("docs");
    match std::fs::symlink_metadata(&docs_root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Ok((
                Vec::new(),
                vec![
                    ".billet/docs: must be a real directory (not a symbolic link); run `blt check`"
                        .to_string(),
                ],
            ));
        }
        Ok(_) => {
            return Ok((
                Vec::new(),
                vec![".billet/docs: must be a real directory; run `blt check`".to_string()],
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => {
            return Ok((
                Vec::new(),
                vec![format!(
                    ".billet/docs: cannot inspect: {error}; run `blt check`"
                )],
            ));
        }
    }
    let docs = billet.join("docs").join(id);
    let metadata = match std::fs::symlink_metadata(&docs) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(error) => {
            return Ok((
                Vec::new(),
                vec![format!(".billet/docs/{id}: cannot inspect: {error}")],
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok((
            Vec::new(),
            vec![format!(
                ".billet/docs/{id}: must be a real directory (not a symbolic link); run `blt check`"
            )],
        ));
    }
    if !metadata.is_dir() {
        return Ok((
            Vec::new(),
            vec![format!(
                ".billet/docs/{id}: must be a real directory; run `blt check`"
            )],
        ));
    }

    fn walk(root: &Path, dir: &Path, paths: &mut Vec<String>, warnings: &mut Vec<String>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                let rel = dir
                    .strip_prefix(root)
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| dir.to_string_lossy().replace('\\', "/"));
                warnings.push(format!("{rel}: cannot read: {error}; run `blt check`"));
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let rel = dir
                        .strip_prefix(root)
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| dir.to_string_lossy().replace('\\', "/"));
                    warnings.push(format!(
                        "{rel}: cannot read entry: {error}; run `blt check`"
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    let rel = path
                        .strip_prefix(root)
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
                    warnings.push(format!("{rel}: cannot inspect: {error}; run `blt check`"));
                    continue;
                }
            };
            if metadata.is_dir() {
                walk(root, &path, paths, warnings);
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root).unwrap_or(path.as_path());
                paths.push(relative.to_string_lossy().replace('\\', "/"));
            } else {
                let rel = path
                    .strip_prefix(root)
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
                warnings.push(format!(
                    "{rel}: must be a regular file or directory; run `blt check`"
                ));
            }
        }
    }

    let project_root = billet
        .parent()
        .ok_or_else(|| fail("cannot determine project root"))?;
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    walk(project_root, &docs, &mut paths, &mut warnings);
    paths.sort();
    Ok((paths, warnings))
}

fn named_text(names: &Names, issue: &Issue, text: &str) -> String {
    let text = text.trim_end_matches('\n');
    let Some(rest) = text.strip_prefix("---\n") else {
        return text.to_string();
    };
    let Some(end) = rest.find("\n---\n") else {
        return text.to_string();
    };
    let mut out = String::from("---\n");
    for line in rest[..end].split('\n') {
        let key = line.split_once(':').map(|(key, _)| key.trim());
        match key {
            Some("number") => continue,
            Some("id") => out.push_str(&format!("id: {}", names.name(&issue.id))),
            Some("parent") => match &issue.parent {
                Some(parent) => out.push_str(&format!("parent: {}", names.name(parent))),
                None => out.push_str("parent: null"),
            },
            Some("depends_on") => out.push_str(&format!(
                "depends_on: [{}]",
                issue
                    .depends_on
                    .iter()
                    .map(|key| names.name(key))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out.push_str(&rest[end + 1..]);
    out
}

fn cmd_show(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_command_tail(args, "show")?;
    pos_n(&p, 1, "show ID")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |billet| {
        let snap = check::scan(billet);
        let names = snapshot_names(billet, &snap)?;
        let l = load(billet, &names, &p.pos[0])?;
        snap.resolve(&l.issue.id)
            .map_err(|e| fail(e.text(&names)))?;
        let i = &l.issue;
        let named = |keys: &[String]| {
            format!(
                "[{}]",
                keys.iter()
                    .map(|key| issue::esc(&names.name(key)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let (docs, doc_warnings) = issue_docs(billet, &i.id)?;
        let docs_dir = format!(".billet/docs/{}/", i.id);
        let mut readiness = domain::eligibility(
            i,
            &clock::today(),
            |id| snap.resolve(id),
            |id| snap.descendants(id),
            |id| snap.unresolved_descendants(id),
        );
        if !domain::field_problems(i).is_empty() {
            readiness.unresolved.push(domain::Unresolved::Own(
                store::diagnostic_path(&l.path).to_path_buf(),
            ));
        }
        if json {
            let deps = named(&i.depends_on);
            let unresolved = format!(
                "[{}]",
                readiness
                    .unresolved
                    .iter()
                    .map(|s| issue::esc(&s.text(&names)))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let blockers = named(&readiness.blockers);
            let subtasks = named(&readiness.subtasks);
            let readiness_json = format!(
                "{{\"ready\":{},\"deferred_until\":{},\"deferred_by\":{},\"unresolved\":{},\"blockers\":{},\"subtasks\":{}}}",
                readiness.ready(),
                jopt(&readiness.deferred_until),
                jopt(&readiness.deferred_by.as_ref().map(|key| names.name(key))),
                unresolved,
                blockers,
                subtasks
            );
            Ok(CommandOutput {
                stdout: format!(
                    "{{\"id\":{},\"key\":{},\"title\":{},\"status\":{},\"priority\":{},\"parent\":{},\
                 \"depends_on\":{},\"assignee\":{},\"defer_until\":{},\"created_at\":{},\
                 \"updated_at\":{},\"closed_at\":{},\"release\":{},\"body\":{},\"history\":{},\"notes\":{},\
                 \"closing_summary\":{},\"readiness\":{},\"docs_dir\":{},\"docs\":[{}]}}\n",
                    issue::esc(&names.name(&i.id)),
                    issue::esc(&i.id),
                    issue::esc(&i.title),
                    issue::esc(i.status.as_str()),
                    i.priority,
                    jopt(&i.parent.as_ref().map(|key| names.name(key))),
                    deps,
                    jopt(&i.assignee),
                    jopt(&i.defer_until),
                    jopt(&i.created_at),
                    jopt(&i.updated_at),
                    jopt(&i.closed_at),
                    jopt(&i.release),
                    issue::esc(&i.body),
                    issue::esc(&i.history),
                    issue::esc(&i.notes),
                    jopt(&i.closing_summary),
                    readiness_json,
                    issue::esc(&docs_dir),
                    docs.iter()
                        .map(|path| issue::esc(path))
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                warnings: doc_warnings,
                code: 0,
            })
        } else {
            let mut output = named_text(&names, i, &l.text);
            output.push_str("\n\nReady: ");
            if readiness.ready() {
                output.push_str("yes\n");
            } else {
                output.push_str("no — ");
                output.push_str(&readiness.reason_text(&names, true));
                output.push('\n');
            }
            output.push_str("Documents: ");
            output.push_str(&docs_dir);
            if docs.is_empty() {
                output.push_str(" (none)\n");
            } else {
                output.push('\n');
                for path in docs {
                    output.push_str("  ");
                    output.push_str(&path);
                    output.push('\n');
                }
            }
            Ok(CommandOutput {
                stdout: output,
                warnings: doc_warnings,
                code: 0,
            })
        }
    })
}

fn cmd_list(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_command_tail(args, "list")?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    let release = opt(&p, "release");
    if release.is_some()
        && let Some(status) = opt(&p, "status")
        && !matches!(status, "closed" | "all")
    {
        return Err(usage("--release requires --status closed or all"));
    }
    let filter = if release.is_some() {
        opt(&p, "status").unwrap_or("closed")
    } else {
        opt(&p, "status").unwrap_or("active")
    }
    .to_string();
    if let Some(explicit) = opt(&p, "status")
        && !["proposed", "open", "claimed", "review", "closed", "all"].contains(&explicit)
    {
        return Err(usage(format!(
            "invalid status filter {explicit:?}; expected proposed|open|claimed|review|closed|all"
        )));
    }
    let order = match opt(&p, "sort") {
        None => None,
        Some("oldest") => Some(CreatedOrder::Oldest),
        Some("newest") => Some(CreatedOrder::Newest),
        Some(other) => {
            return Err(usage(format!(
                "invalid sort {other:?}; expected oldest or newest"
            )));
        }
    };
    with_registry(cwd, |billet| {
        let snap = check::scan(billet);
        let names = snapshot_names(billet, &snap)?;
        let want = |s: Status| {
            filter == "all"
                || (filter == "active" && s.active())
                || Status::parse(&filter) == Some(s)
        };
        let mut rows: Vec<Issue> = snap
            .issues()
            .filter(|i| want(i.status))
            .filter(|i| {
                release.is_none_or(|name| {
                    i.status == Status::Closed && i.release.as_deref() == Some(name)
                })
            })
            .cloned()
            .collect();
        match order {
            Some(order) => sort_chronologically(&mut rows, order),
            None => sort_rows(&mut rows),
        }
        let grouped = !json && order.is_none() && release.is_none() && opt(&p, "status").is_none();
        let stdout = if json {
            let items: Vec<String> = rows.iter().map(|i| row_json(&names, i)).collect();
            format!("[{}]\n", items.join(","))
        } else if grouped {
            let groups = [
                (Status::Review, "REVIEW"),
                (Status::Proposed, "PROPOSED"),
                (Status::Claimed, "CLAIMED"),
                (Status::Open, "OPEN"),
            ];
            let mut out = String::new();
            for (status, label) in groups {
                let group: Vec<&Issue> = rows.iter().filter(|i| i.status == status).collect();
                if group.is_empty() {
                    continue;
                }
                out.push_str(&format!("{label} ({})\n", group.len()));
                for i in group {
                    let who = i.assignee.as_deref().unwrap_or("-");
                    out.push_str(&format!(
                        "  {} {} {} {who} {}\n",
                        names.name(&i.id),
                        i.status.as_str(),
                        i.priority,
                        i.title
                    ));
                }
            }
            out
        } else {
            let mut out = String::new();
            for i in &rows {
                let pri = i.priority;
                let who = i.assignee.clone().unwrap_or_else(|| "-".into());
                let created = order
                    .map(|_| format!(" {}", i.created_at.as_deref().unwrap_or("-")))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{} {} {pri} {who}{created} {}\n",
                    names.name(&i.id),
                    i.status.as_str(),
                    i.title
                ));
            }
            out
        };
        let mut warnings = file_warnings(&snap, |_| true, false);
        warnings.extend(duplicate_warnings(&names));
        Ok(CommandOutput {
            stdout,
            warnings,
            code: 0,
        })
    })
}

fn child_key(issue: &Issue) -> (u8, u64, String) {
    match issue.id.rsplit_once('.') {
        Some((_, n)) => n
            .parse::<u64>()
            .map(|n| (0, n, issue.id.clone()))
            .unwrap_or((1, 0, issue.id.clone())),
        None => (1, 0, issue.id.clone()),
    }
}

fn cmd_tree(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_command_tail(args, "tree")?;
    pos_n(&p, 1, "tree ID")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |billet| {
        let snap = check::scan(billet);
        let names = snapshot_names(billet, &snap)?;
        let id = resolve(&names, &p.pos[0])?;
        let root = snap.resolve(&id).map_err(|e| fail(e.text(&names)))?;
        let available: HashSet<&str> = snap.issues().map(|i| i.id.as_str()).collect();
        let mut kids: HashMap<String, Vec<Issue>> = HashMap::new();
        for i in snap.issues() {
            let mut anc = i.parent.as_deref();
            while let Some(a) = anc {
                if available.contains(a) {
                    break;
                }
                anc = a.rsplit_once('.').map(|(id, _)| id);
            }
            if let Some(a) = anc {
                kids.entry(a.to_string()).or_default().push(i.clone());
            }
        }
        for v in kids.values_mut() {
            v.sort_by_key(child_key);
        }
        let stdout = if json {
            format!("{}\n", tree_json(&names, &root, &kids, &mut HashSet::new()))
        } else {
            let mut out = String::new();
            tree_text(&names, &mut out, &root, &kids, 0, &mut HashSet::new());
            out
        };
        let mut warnings = file_warnings(&snap, |_| true, false);
        warnings.extend(duplicate_warnings(&names));
        Ok(CommandOutput {
            stdout,
            warnings,
            code: 0,
        })
    })
}

fn tree_json(
    names: &Names,
    i: &Issue,
    kids: &HashMap<String, Vec<Issue>>,
    seen: &mut HashSet<String>,
) -> String {
    let mut out = format!(
        "{{\"id\":{},\"key\":{},\"title\":{},\"status\":{},\"children\":[",
        issue::esc(&names.name(&i.id)),
        issue::esc(&i.id),
        issue::esc(&i.title),
        issue::esc(i.status.as_str())
    );
    if seen.insert(i.id.clone())
        && let Some(list) = kids.get(&i.id)
    {
        let parts: Vec<String> = list
            .iter()
            .map(|c| tree_json(names, c, kids, seen))
            .collect();
        out.push_str(&parts.join(","));
    }
    out.push_str("]}");
    out
}

fn tree_text(
    names: &Names,
    out: &mut String,
    i: &Issue,
    kids: &HashMap<String, Vec<Issue>>,
    depth: usize,
    seen: &mut HashSet<String>,
) {
    out.push_str(&"  ".repeat(depth));
    out.push_str(&format!(
        "{} {} {}\n",
        names.name(&i.id),
        i.status.as_str(),
        i.title
    ));
    if !seen.insert(i.id.clone()) {
        return;
    }
    if let Some(list) = kids.get(&i.id) {
        for c in list {
            tree_text(names, out, c, kids, depth + 1, seen);
        }
    }
}

fn cmd_claim(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "claim")?;
    pos_n(&p, 1, "claim ID")?;
    let force = flag(&p, "force");
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let names = names(billet)?;
    let l = load(billet, &names, &p.pos[0])?;
    let snap = check::scan(billet);
    if let Err(e) = snap.resolve(&l.issue.id) {
        return Err(fail(e.text(&names)));
    }
    if let Some((problem, _)) = domain::field_problems(&l.issue).first() {
        return Err(fail(format!(
            "issue {}: {problem}",
            names.name(&l.issue.id)
        )));
    }
    let today = clock::today();
    let eligibility = domain::eligibility(
        &l.issue,
        &today,
        |id| snap.resolve(id),
        |id| snap.descendants(id),
        |id| snap.unresolved_descendants(id),
    );
    let blocked = !eligibility.blockers.is_empty() || !eligibility.subtasks.is_empty();
    if !eligibility.ready()
        && (!force
            || !blocked
            || eligibility.status.is_some()
            || eligibility.deferred_until.is_some()
            || !eligibility.unresolved.is_empty())
    {
        let prefix = if eligibility.status.as_deref() == Some("proposed") {
            " awaits approval;"
        } else {
            ""
        };
        return Err(fail(format!(
            "issue {}{} is not ready: {}",
            names.name(&l.issue.id),
            prefix,
            eligibility.reason_text(&names, true)
        )));
    }
    let bypassed = eligibility.bypassed();
    let by = opt(&p, "by").map(|s| s.to_string());
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Claim).map_err(fail)?;
    i.assignee = by.clone();
    i.claim_id = Some(ident::new_claim_id().map_err(fail)?);
    i.updated_at = Some(clock::now_iso());
    if force && !bypassed.is_empty() {
        let who = by
            .as_deref()
            .map(|b| format!(" by {b}"))
            .unwrap_or_default();
        let mut parts = Vec::new();
        if !eligibility.blockers.is_empty() {
            parts.push(format!(
                "blockers {}",
                joined(&names, &eligibility.blockers)
            ));
        }
        if !eligibility.subtasks.is_empty() {
            parts.push(format!(
                "subtasks {}",
                joined(&names, &eligibility.subtasks)
            ));
        }
        hist_push(
            &mut i,
            hist_entry(&format!(
                "claimed --force{who} bypassing {}",
                parts.join("; ")
            )),
        );
    }
    store::write_issue(&l, &i).map_err(fail)?;
    let who = by.map(|b| format!(" as {b}")).unwrap_or_default();
    Ok(format!("claimed {}{who}\n", names.name(&i.id)))
}

fn cmd_approve(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "approve")?;
    pos_n(&p, 1, "approve ID")?;
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Approve).map_err(fail)?;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, hist_entry("approved"));
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("approved {}\n", names.name(&i.id)))
}

fn cmd_revise(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "revise")?;
    pos_n(&p, 1, "revise ID")?;
    let reason = opt(&p, "reason").ok_or_else(|| usage("--reason is required"))?;
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Revise).map_err(fail)?;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, hist_entry(&history_text("revised:", reason)));
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("returned {} to proposed\n", names.name(&i.id)))
}

fn cmd_unclaim(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "unclaim")?;
    pos_n(&p, 1, "unclaim ID")?;
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Unclaim).map_err(fail)?;
    i.assignee = None;
    i.claim_id = None;
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("unclaimed {}\n", names.name(&i.id)))
}

fn cmd_update(args: &[String], cwd: &Path, allow_stdin: bool) -> R<String> {
    let p = parse_command_tail(args, "update")?;
    pos_n(&p, 1, "update ID")?;
    if opt(&p, "title").is_none()
        && opt(&p, "priority").is_none()
        && opt(&p, "body").is_none()
        && opt(&p, "release").is_none()
        && opt(&p, "number").is_none()
    {
        return Err(usage("update requires at least one change"));
    }
    let body = body_arg(&p, allow_stdin)?;
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    if opt(&p, "release").is_some() && l.issue.status != Status::Closed {
        return Err(fail(format!(
            "issue {} is not closed",
            names.name(&l.issue.id)
        )));
    }
    let mut i = l.issue.clone();
    if let Some(t) = opt(&p, "title") {
        i.title = t.to_string();
    }
    if let Some(v) = opt(&p, "priority") {
        i.priority = parse_priority(v)?;
    }
    if let Some(b) = body {
        i.body = b;
    }
    let mut names = names;
    if let Some(raw) = opt(&p, "number") {
        let number: u64 = raw
            .parse()
            .map_err(|_| usage(format!("invalid issue number {raw:?}")))?;
        if i.number.is_none() {
            return Err(fail(format!(
                "{} is a subtask; its number comes from its root",
                names.name(&i.id)
            )));
        }
        if i.number != Some(number) {
            if names.taken(number) {
                return Err(fail(format!(
                    "number {}{number} is already used",
                    names.prefix
                )));
            }
            let old = names.name(&i.id);
            i.number = Some(number);
            names.add(&i, l.path.clone());
            let new = names.name(&i.id);
            hist_push(&mut i, hist_entry(&format!("number: {old} -> {new}")));
        } else if opt(&p, "title").is_none()
            && opt(&p, "priority").is_none()
            && opt(&p, "body").is_none()
            && opt(&p, "release").is_none()
        {
            return Ok(format!("updated {}\n", names.name(&i.id)));
        }
    }
    if let Some(release) = opt(&p, "release") {
        if i.release.as_deref() != Some(release) {
            let old = i.release.as_deref().unwrap_or("null").to_string();
            hist_push(&mut i, hist_entry(&format!("release: {old} -> {release}")));
            i.release = Some(release.to_string());
        } else if opt(&p, "title").is_none()
            && opt(&p, "priority").is_none()
            && opt(&p, "body").is_none()
            && i.number == l.issue.number
        {
            return Ok(format!("updated {}\n", names.name(&i.id)));
        }
    }
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("updated {}\n", names.name(&i.id)))
}

fn cmd_review(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "review")?;
    pos_n(&p, 1, "review ID")?;
    let summary = opt(&p, "summary");
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Review).map_err(fail)?;
    i.updated_at = Some(clock::now_iso());
    let who = i
        .assignee
        .as_deref()
        .map(|a| format!(" by {a}"))
        .unwrap_or_default();
    let mut entry = hist_entry(&format!("submitted for review{who}"));
    if let Some(s) = summary {
        entry.push_str("\n  ");
        entry.push_str(&history_text("summary:", s));
    }
    hist_push(&mut i, entry);
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("submitted {} for review\n", names.name(&i.id)))
}

fn cmd_rework(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "rework")?;
    pos_n(&p, 1, "rework ID")?;
    let reason = opt(&p, "reason").ok_or_else(|| usage("--reason is required"))?;
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Rework).map_err(fail)?;
    if i.claim_id.is_none() {
        i.claim_id = Some(ident::new_claim_id().map_err(fail)?);
    }
    i.updated_at = Some(clock::now_iso());
    hist_push(
        &mut i,
        hist_entry(&history_text("rework requested:", reason)),
    );
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("returned {} for rework\n", names.name(&i.id)))
}

fn cmd_close(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "close")?;
    pos_n(&p, 1, "close ID")?;
    let reason = opt(&p, "reason").ok_or_else(|| usage("--reason is required"))?;
    let summary = opt(&p, "summary");
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let names = names(billet)?;
    let l = load(billet, &names, &p.pos[0])?;
    let now = clock::now_iso();
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Close).map_err(fail)?;
    i.closed_at = Some(now.clone());
    i.updated_at = Some(now);
    i.claim_id = None;
    if let Some(s) = summary {
        i.closing_summary = Some(s.to_string());
    }
    hist_push(&mut i, hist_entry(&history_text("closed:", reason)));
    store::move_issue(billet, &l, &i).map_err(fail)?;
    Ok(format!("closed {}\n", names.name(&i.id)))
}

fn cmd_reopen(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "reopen")?;
    pos_n(&p, 1, "reopen ID")?;
    let reason = opt(&p, "reason").ok_or_else(|| usage("--reason is required"))?;
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let names = names(billet)?;
    let l = load(billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, &names.name(&i.id), Action::Reopen).map_err(fail)?;
    let mut entry = hist_entry(&history_text("reopened:", reason));
    if let Some(summary) = i.closing_summary.take() {
        entry.push_str("\n  ");
        entry.push_str(&history_text("closing summary:", &summary));
    }
    if let Some(release) = i.release.take() {
        entry.push_str("\n  ");
        entry.push_str(&history_text("release:", &release));
    }
    i.assignee = None;
    i.claim_id = None;
    i.closed_at = None;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, entry);
    store::move_issue(billet, &l, &i).map_err(fail)?;
    Ok(format!("reopened {}\n", names.name(&i.id)))
}

fn cmd_release(args: &[String], cwd: &Path) -> R<CommandOutput> {
    let p = parse_command_tail(args, "release")?;
    pos_n(&p, 1, "release NAME")?;
    let name = &p.pos[0];
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let snap = check::scan(billet);
    let names = snapshot_names(billet, &snap)?;
    let warnings = file_warnings(
        &snap,
        |path| {
            path.starts_with(".billet/closed")
                && snap
                    .records
                    .iter()
                    .find(|rec| rec.path == path)
                    .is_none_or(|rec| rec.issue.release.is_none())
        },
        true,
    );
    let mut ids = Vec::new();
    let now = clock::now_iso();
    let mut candidates: Vec<_> = snap
        .records
        .iter()
        .filter(|rec| {
            rec.usable
                && rec.place == Place::Closed
                && rec.issue.release.is_none()
                && domain::field_problems(&rec.issue).is_empty()
        })
        .collect();
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    for rec in candidates {
        let loaded = store::find(billet, &rec.issue.id)
            .map_err(fail)?
            .ok_or_else(|| fail(format!("issue {} not found", names.name(&rec.issue.id))))?;
        let mut issue = loaded.issue.clone();
        issue.release = Some(name.clone());
        issue.updated_at = Some(now.clone());
        store::write_issue(&loaded, &issue).map_err(fail)?;
        ids.push(names.name(&issue.id));
    }
    if ids.is_empty() {
        return Ok(CommandOutput {
            stdout: "nothing sealed\n".to_string(),
            warnings,
            code: 0,
        });
    }
    Ok(CommandOutput {
        stdout: format!("{}\n", ids.join("\n")),
        warnings,
        code: 0,
    })
}

fn dep_graph(billet: &Path) -> R<HashMap<String, Vec<String>>> {
    let mut g = HashMap::new();
    for l in store::load_all(billet).map_err(fail)? {
        g.insert(l.issue.id.clone(), l.issue.depends_on.clone());
    }
    Ok(g)
}

fn reaches(g: &HashMap<String, Vec<String>>, from: &str, to: &str) -> bool {
    let mut stack = vec![from.to_string()];
    let mut seen = HashSet::new();
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(edges) = g.get(&n) {
            stack.extend(edges.iter().cloned());
        }
    }
    false
}

fn cmd_dep(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "dep")?;
    pos_n(&p, 3, "dep add|rm ID BLOCKER")?;
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let names = names(billet)?;
    let id = resolve(&names, &p.pos[1])?;
    let blocker = resolve(&names, &p.pos[2])?;
    if id == blocker {
        return Err(fail("issue cannot depend on itself"));
    }
    let l = store::find(billet, &id)
        .map_err(fail)?
        .ok_or_else(|| fail(format!("issue {} not found", names.name(&id))))?;
    let shown = names.name(&id);
    let shown_blocker = names.name(&blocker);
    let mut i = l.issue.clone();
    match p.pos[0].as_str() {
        "add" => {
            store::find(billet, &blocker)
                .map_err(fail)?
                .ok_or_else(|| fail(format!("issue {shown_blocker} not found")))?;
            if i.depends_on.contains(&blocker) {
                return Err(fail(format!("{shown} already depends on {shown_blocker}")));
            }
            let g = dep_graph(billet)?;
            if reaches(&g, &blocker, &id) {
                return Err(fail(format!("dependency cycle via {shown_blocker}")));
            }
            i.depends_on.push(blocker.clone());
            i.updated_at = Some(clock::now_iso());
            store::write_issue(&l, &i).map_err(fail)?;
            Ok(format!("{shown} now blocked by {shown_blocker}\n"))
        }
        "rm" => {
            if !i.depends_on.contains(&blocker) {
                return Err(fail(format!("{shown} does not depend on {shown_blocker}")));
            }
            i.depends_on.retain(|d| d != &blocker);
            i.updated_at = Some(clock::now_iso());
            store::write_issue(&l, &i).map_err(fail)?;
            Ok(format!("{shown} no longer blocked by {shown_blocker}\n"))
        }
        other => Err(usage(format!("unknown dep action {other}; use add or rm"))),
    }
}

fn cmd_note(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "note")?;
    pos_n(&p, 2, "note ID TEXT")?;
    let text = &p.pos[1];
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    let mut i = l.issue.clone();
    let who = opt(&p, "by").map(|b| format!("{b}: ")).unwrap_or_default();
    if !i.notes.is_empty() {
        i.notes.push('\n');
    }
    i.notes
        .push_str(&format!("- {} {who}{text}", clock::now_stamp()));
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("noted {}\n", names.name(&i.id)))
}

fn cmd_defer(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "defer")?;
    pos_n(&p, 1, "defer ID")?;
    let until = opt(&p, "until");
    let clear = flag(&p, "clear");
    if until.is_some() == clear {
        return Err(usage("defer requires exactly one of --until or --clear"));
    }
    let lock = registry(cwd)?;
    let names = names(&lock.billet)?;
    let l = load(&lock.billet, &names, &p.pos[0])?;
    if l.place == Place::Closed && until.is_some() {
        return Err(fail(format!("issue {} is closed", names.name(&l.issue.id))));
    }
    let mut i = l.issue.clone();
    i.defer_until = until.map(|d| d.to_string());
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    match until {
        Some(d) => Ok(format!("deferred {} until {d}\n", names.name(&i.id))),
        None => Ok(format!("cleared deferral on {}\n", names.name(&i.id))),
    }
}

fn load_memory(billet: &Path) -> R<memory::Memory> {
    let text = std::fs::read_to_string(billet.join("memory.md"))
        .map_err(|e| fail(format!("read memory.md: {e}")))?;
    memory::parse(&text).map_err(|e| fail(format!("memory.md: {e}")))
}

fn cmd_remember(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_command_tail(args, "remember")?;
    pos_n(&p, 2, "remember TOPIC TEXT")?;
    let topic = &p.pos[0];
    let lock = registry(cwd)?;
    let billet = &lock.billet;
    let mut mem = load_memory(billet)?;
    let value = p.pos[1].clone();
    match mem.entries.iter_mut().find(|(t, _)| t == topic) {
        Some((_, v)) => *v = value,
        None => mem.entries.push((topic.clone(), value)),
    }
    let text = memory::render(&mem);
    store::atomic_write(&billet.join("memory.md"), text.as_bytes()).map_err(fail)?;
    Ok(format!("remembered {topic}\n"))
}

fn cmd_recall(args: &[String], cwd: &Path, json: bool) -> R<String> {
    let p = parse_command_tail(args, "recall")?;
    if p.pos.len() > 1 {
        return Err(usage("expected at most one topic"));
    }
    let json = json || flag(&p, "json");
    with_registry(cwd, |billet| {
        let mem = load_memory(billet)?;
        match p.pos.first() {
            None => {
                let mut names: Vec<String> = mem.entries.iter().map(|(t, _)| t.clone()).collect();
                names.sort();
                if json {
                    let items: Vec<String> = names.iter().map(|n| issue::esc(n)).collect();
                    Ok(format!("[{}]\n", items.join(",")))
                } else {
                    Ok(names.iter().map(|n| format!("{n}\n")).collect())
                }
            }
            Some(topic) => {
                let Some((_, v)) = mem.entries.iter().find(|(t, _)| t == topic) else {
                    return Err(fail(format!("unknown topic {topic}")));
                };
                if json {
                    Ok(format!(
                        "{{\"topic\":{},\"value\":{}}}\n",
                        issue::esc(topic),
                        issue::esc(v)
                    ))
                } else {
                    Ok(format!("{v}\n"))
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{hist_entry, history_text};
    use crate::issue::{self, Issue, Status};
    use crate::memory;
    use crate::text::TextKind;
    use crate::text::generated::{self, Rng};

    fn issue(status: Status) -> Issue {
        Issue {
            id: "1abcde".into(),
            number: Some(1),
            title: "t".into(),
            status,
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
            body: String::new(),
            history: String::new(),
            notes: String::new(),
            closing_summary: None,
        }
    }

    fn round_trip(issue: &Issue, what: &str, case: usize) {
        let encoded = issue::encode(issue)
            .unwrap_or_else(|e| panic!("case {case} {what}: accepted value refused by codec: {e}"));
        let decoded = issue::decode(&encoded).unwrap();
        assert_eq!(
            &decoded, issue,
            "case {case} {what}: bytes changed through the file"
        );
    }

    #[test]
    fn accepted_text_survives_the_file_codec() {
        let mut rng = Rng(0xc0de_c0de);
        let mut stored = 0;
        for case in 0..3000 {
            let (value, _) = generated::text(&mut rng);
            if TextKind::Label.check("title", true, &value).is_ok() {
                let mut open = issue(Status::Open);
                open.title = value.clone();
                round_trip(&open, "title", case);
                let mut claimed = issue(Status::Claimed);
                claimed.claim_id = Some("0123456789abcdef0123456789abcdef".into());
                claimed.assignee = Some(value.clone());
                round_trip(&claimed, "assignee", case);
                stored += 1;
            }
            if TextKind::Body.check("body", false, &value).is_ok() {
                let mut open = issue(Status::Open);
                open.body = value.clone();
                round_trip(&open, "body", case);
                stored += 1;
            }
            if TextKind::Multiline
                .check("--summary", false, &value)
                .is_ok()
            {
                let mut closed = issue(Status::Closed);
                closed.closed_at = Some("2026-01-02T03:04:05Z".into());
                closed.history = hist_entry(&history_text("closed:", &value));
                closed.closing_summary = Some(value.clone());
                round_trip(&closed, "history and closing summary", case);
                stored += 1;
            }
            if TextKind::Release.check("release", true, &value).is_ok() {
                let mut closed = issue(Status::Closed);
                closed.release = Some(value.clone());
                round_trip(&closed, "release", case);
                stored += 1;
            }
            if TextKind::Note.check("text", true, &value).is_ok() {
                let mut open = issue(Status::Open);
                open.notes = format!("- 2026-01-02 03:04:05 UTC -me: {value}");
                round_trip(&open, "note", case);
                stored += 1;
            }
            let memory = memory::Memory {
                entries: vec![("topic".into(), value.clone())],
            };
            let parsed = memory::parse(&memory::render(&memory)).unwrap();
            assert_eq!(parsed.entries, memory.entries, "case {case} memory value");
        }
        assert!(stored > 3000, "only {stored} stored values");
    }
}
