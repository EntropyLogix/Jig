use crate::check;
use crate::clock;
use crate::domain::{self, Action};
use crate::ident;
use crate::instructions;
use crate::integration;
use crate::issue::{self, Issue, Status};
use crate::memory;
use crate::store::{self, Loaded, Place};
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

const HELP: &str = "Jig - file-based issue tracker and project memory\n\
\n\
Usage: jig [--json] COMMAND [ARGS]\n\
\n\
Commands:\n\
  init [--no-hooks]                    create .jig/, AGENTS.md, and project hooks\n\
  start [--json|--hook-json]            workflow summary\n\
  add TITLE [--approved] [-p|--priority N] [--body TEXT|-] [--parent ID] [--dep ID ...]\n\
  ready [--json]                        unblocked open issues\n\
  show ID [--json]                      one full issue\n\
  list [--status S] [--sort oldest|newest] [--json]\n\
                                        issues (S: proposed|open|claimed|review|closed|all)\n\
  tree ID [--json]                      issue with descendants\n\
  approve ID                            move a proposed issue to open\n\
  revise ID --reason TEXT               return an open issue to proposed\n\
  claim ID [--by NAME] [--force]        move an open issue to claimed\n\
  unclaim ID                            return a claimed issue to open\n\
  update ID [--title T] [-p|--priority N] [--body TEXT|-]\n\
  review ID [--summary TEXT]            submit a claimed issue for review\n\
  rework ID --reason TEXT               return a review issue to claimed\n\
  close ID --reason TEXT [--summary T]  archive an active issue\n\
  reopen ID --reason TEXT               reactivate a closed issue\n\
  dep add|rm ID BLOCKER                 manage blockers\n\
  note ID TEXT [--by NAME]              append a one-line note\n\
  defer ID --until YYYY-MM-DD | --clear\n\
  remember TOPIC TEXT                   record a project fact\n\
  recall [TOPIC] [--json]               read project facts\n\
  check [--json]                         report registry problems without repairing\n\
\n\
Options:\n\
  --json          machine output for read commands (before or after the command)\n\
  --help          this text\n\
  --version       version\n\
  --              stop option parsing";

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
                        && args[i] != "--"
                        && !(args[i].len() > 1 && args[i].starts_with('-'))
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
    let jig = store::discover(cwd).map_err(fail)?;
    store::lock(&jig).map_err(fail)
}

fn with_registry<T>(cwd: &Path, mut read: impl FnMut(&Path) -> R<T>) -> R<T> {
    let jig = store::discover(cwd).map_err(fail)?;
    store::with_read(&jig, store::Recovery::Allow, || read(&jig)).map_err(fail)?
}

fn resolve_id(raw: &str) -> R<String> {
    issue::id_alias(raw).ok_or_else(|| usage(format!("invalid issue id {raw:?}")))
}

fn load(jig: &Path, raw: &str) -> R<Loaded> {
    let id = resolve_id(raw)?;
    store::find(jig, &id)
        .map_err(fail)?
        .ok_or_else(|| fail(format!("issue {id} not found")))
}

fn read_stdin() -> R<String> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| fail(e.to_string()))?;
    Ok(s)
}

fn body_arg(p: &Parsed) -> R<Option<String>> {
    match opt(p, "body") {
        Some("-") => Ok(Some(read_stdin()?)),
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

fn validated_history_text<'a>(text: &'a str, option: &str) -> R<&'a str> {
    if text.chars().any(|c| c != '\n' && c.is_control()) {
        return Err(usage(format!(
            "{option}: control characters other than newline are not allowed"
        )));
    }
    issue::structural_check(text, option).map_err(usage)?;
    Ok(text)
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

fn row_json(i: &Issue) -> String {
    format!(
        "{{\"id\":{},\"title\":{},\"status\":{},\"priority\":{},\"assignee\":{}}}",
        issue::esc(&i.id),
        issue::esc(&i.title),
        issue::esc(i.status.as_str()),
        i.priority
            .map(|p| p.to_string())
            .unwrap_or_else(|| "null".into()),
        jopt(&i.assignee)
    )
}

fn sort_rows(rows: &mut [Issue]) {
    rows.sort_by(|a, b| {
        (
            a.priority.unwrap_or(u8::MAX),
            a.created_at.is_none(),
            a.created_at.clone().unwrap_or_default(),
            &a.id,
        )
            .cmp(&(
                b.priority.unwrap_or(u8::MAX),
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

const JSON_CMDS: [&str; 7] = ["start", "ready", "show", "list", "tree", "recall", "check"];

pub fn run(args: &[String], cwd: &Path) -> Result<CommandOutput, Fail> {
    if args.is_empty() {
        return Ok(CommandOutput::plain(format!("{HELP}\n")));
    }
    let mut json = false;
    let mut i = 0usize;
    let cmd = loop {
        let Some(a) = args.get(i) else {
            return Err(usage("missing command; try `jig --help`"));
        };
        match a.as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--" => {
                i += 1;
                let Some(a) = args.get(i) else {
                    return Err(usage("missing command; try `jig --help`"));
                };
                break a.to_string();
            }
            "--help" | "-h" | "help" => return Ok(CommandOutput::plain(format!("{HELP}\n"))),
            "--version" => {
                return Ok(CommandOutput::plain(format!(
                    "jig {}\n",
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
    match cmd.as_str() {
        "ready" => cmd_ready(&tail, cwd, json),
        "check" => cmd_check(&tail, cwd, json),
        "init" => cmd_init(&tail, cwd).map(CommandOutput::plain),
        "start" => cmd_start(&tail, cwd, json).map(CommandOutput::plain),
        "add" => cmd_add(&tail, cwd).map(CommandOutput::plain),
        "show" => cmd_show(&tail, cwd, json).map(CommandOutput::plain),
        "list" => cmd_list(&tail, cwd, json),
        "tree" => cmd_tree(&tail, cwd, json),
        "approve" => cmd_approve(&tail, cwd).map(CommandOutput::plain),
        "revise" => cmd_revise(&tail, cwd).map(CommandOutput::plain),
        "claim" => cmd_claim(&tail, cwd).map(CommandOutput::plain),
        "unclaim" => cmd_unclaim(&tail, cwd).map(CommandOutput::plain),
        "update" => cmd_update(&tail, cwd).map(CommandOutput::plain),
        "review" => cmd_review(&tail, cwd).map(CommandOutput::plain),
        "rework" => cmd_rework(&tail, cwd).map(CommandOutput::plain),
        "close" => cmd_close(&tail, cwd).map(CommandOutput::plain),
        "reopen" => cmd_reopen(&tail, cwd).map(CommandOutput::plain),
        "dep" => cmd_dep(&tail, cwd).map(CommandOutput::plain),
        "note" => cmd_note(&tail, cwd).map(CommandOutput::plain),
        "defer" => cmd_defer(&tail, cwd).map(CommandOutput::plain),
        "remember" => cmd_remember(&tail, cwd).map(CommandOutput::plain),
        "recall" => cmd_recall(&tail, cwd, json).map(CommandOutput::plain),
        _ => Err(usage(format!("unknown command {cmd}; try `jig --help`"))),
    }
}

fn cmd_init(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--no-hooks", Arity::Switch, "no-hooks")], &[])?;
    pos_n(&p, 0, "no positional arguments")?;
    let created = store::init(cwd).map_err(fail)?;
    let _lock = registry(cwd)?;
    let notice = integration::install(cwd, !flag(&p, "no-hooks")).map_err(fail)?;
    let mut out = if created {
        "initialized .jig/\n".to_string()
    } else {
        ".jig/ already initialized\n".to_string()
    };
    if let Some(notice) = notice {
        out.push_str(&notice);
        out.push('\n');
    }
    Ok(out)
}

fn cmd_start(args: &[String], cwd: &Path, json: bool) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("--json", Arity::Switch, "json"),
            ("--hook-json", Arity::Switch, "hook-json"),
        ],
        &[],
    )?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    if flag(&p, "hook-json") {
        if json {
            return Err(usage("--json and --hook-json are mutually exclusive"));
        }
        store::discover(cwd).map_err(fail)?;
        return Ok(format!(
            "{{\"hookSpecificOutput\":{{\"hookEventName\":\"SessionStart\",\"additionalContext\":{}}}}}\n",
            issue::esc(instructions::WORKFLOW.trim_end_matches('\n'))
        ));
    }
    with_registry(cwd, |_jig| {
        let workflow = instructions::WORKFLOW.trim_end_matches('\n');
        if json {
            Ok(format!("{{\"workflow\":{}}}\n", issue::esc(workflow)))
        } else {
            Ok(format!("{workflow}\n"))
        }
    })
}

fn cmd_add(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("-p", Arity::One, "priority"),
            ("--priority", Arity::One, "priority"),
            ("--body", Arity::One, "body"),
            ("--parent", Arity::One, "parent"),
            ("--dep", Arity::Many, "dep"),
            ("--approved", Arity::Switch, "approved"),
        ],
        &["dep"],
    )?;
    pos_n(&p, 1, "add TITLE")?;
    let title = &p.pos[0];
    if !issue::valid_label(title) {
        return Err(usage("title must be a non-empty single line"));
    }
    let priority = match opt(&p, "priority") {
        Some(v) => Some(parse_priority(v)?),
        None => Some(2),
    };
    let body = body_arg(&p)?.unwrap_or_default();
    let lock = registry(cwd)?;
    let jig = lock.jig.clone();
    let parent = match opt(&p, "parent") {
        Some(raw) => Some(load(&jig, raw)?.issue.id),
        None => None,
    };
    let mut depends_on = Vec::new();
    for raw in p.opts.get("dep").into_iter().flatten() {
        let id = load(&jig, raw)?.issue.id;
        if depends_on.contains(&id) {
            return Err(usage(format!("duplicate dependency {id}")));
        }
        depends_on.push(id);
    }
    let id = match &parent {
        Some(p) => next_child(&jig, p)?,
        None => {
            let salt = ident::install_salt(&state_dir()?).map_err(fail)?;
            ident::new_issue_id(&salt, |id| store::exists(&jig, id)).map_err(fail)?
        }
    };
    let now = clock::now_iso();
    let new = Issue {
        id: id.clone(),
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
        claim_id: None,
        body,
        history: String::new(),
        notes: String::new(),
        closing_summary: None,
    };
    store::write_new(&jig, &new).map_err(fail)?;
    Ok(format!("{id}\n"))
}

fn next_child(jig: &Path, parent: &str) -> R<String> {
    let prefix = format!("{parent}.");
    let mut max: u64 = 0;
    for place in [Place::Open, Place::Closed] {
        let dir = jig.join(place.dir());
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
            "skipping {}: {}; run `jig check`",
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
                "{prefix}{}: {problem}; run `jig check`",
                rec.path.display()
            ));
        }
    }
    warnings
}

fn cmd_ready(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_tail(args, &[("--json", Arity::Switch, "json")], &[])?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |jig| {
        let snap = check::scan(jig);
        let today = clock::today();
        let mut warnings = file_warnings(&snap, |p| p.starts_with(".jig/open"), true);
        let mut rows = Vec::new();
        for i in snap.issues() {
            if !domain::field_problems(i).is_empty() {
                continue;
            }
            match domain::eligibility(i, &today, false, |id| snap.resolve(id)) {
                Ok(_) => rows.push(i.clone()),
                Err(domain::Ineligible::Reference(reference)) => warnings.push(format!(
                    "skipping {}: unresolved {reference}; run `jig check`",
                    i.id
                )),
                Err(_) => continue,
            }
        }
        sort_rows(&mut rows);
        let stdout = if json {
            let items: Vec<String> = rows.iter().map(row_json).collect();
            format!("[{}]\n", items.join(","))
        } else {
            let mut out = String::new();
            for i in &rows {
                let pri = i
                    .priority
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into());
                out.push_str(&format!("{} {pri} {}\n", i.id, i.title));
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
    let p = parse_tail(args, &[("--json", Arity::Switch, "json")], &[])?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    let jig = store::discover_raw(cwd).map_err(fail)?;
    let problems = store::with_read(&jig, store::Recovery::ReportOnly, || check::collect(&jig))
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

fn cmd_show(args: &[String], cwd: &Path, json: bool) -> R<String> {
    let p = parse_tail(args, &[("--json", Arity::Switch, "json")], &[])?;
    pos_n(&p, 1, "show ID")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |jig| {
        let l = load(jig, &p.pos[0])?;
        check::scan(jig).resolve(&l.issue.id).map_err(fail)?;
        let i = &l.issue;
        if json {
            let deps = format!(
                "[{}]",
                i.depends_on
                    .iter()
                    .map(|d| issue::esc(d))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            Ok(format!(
                "{{\"id\":{},\"title\":{},\"status\":{},\"priority\":{},\"parent\":{},\
                 \"depends_on\":{},\"assignee\":{},\"defer_until\":{},\"created_at\":{},\
                 \"updated_at\":{},\"closed_at\":{},\"body\":{},\"history\":{},\"notes\":{},\
                 \"closing_summary\":{}}}\n",
                issue::esc(&i.id),
                issue::esc(&i.title),
                issue::esc(i.status.as_str()),
                i.priority
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "null".into()),
                jopt(&i.parent),
                deps,
                jopt(&i.assignee),
                jopt(&i.defer_until),
                jopt(&i.created_at),
                jopt(&i.updated_at),
                jopt(&i.closed_at),
                issue::esc(&i.body),
                issue::esc(&i.history),
                issue::esc(&i.notes),
                jopt(&i.closing_summary),
            ))
        } else {
            Ok(l.text.clone())
        }
    })
}

fn cmd_list(args: &[String], cwd: &Path, json: bool) -> R<CommandOutput> {
    let p = parse_tail(
        args,
        &[
            ("--json", Arity::Switch, "json"),
            ("--status", Arity::One, "status"),
            ("--sort", Arity::One, "sort"),
        ],
        &[],
    )?;
    pos_n(&p, 0, "no arguments")?;
    let json = json || flag(&p, "json");
    let filter = opt(&p, "status").unwrap_or("active").to_string();
    if ![
        "active", "proposed", "open", "claimed", "review", "closed", "all",
    ]
    .contains(&filter.as_str())
    {
        return Err(usage(format!("invalid status filter {filter:?}")));
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
    with_registry(cwd, |jig| {
        let snap = check::scan(jig);
        let want = |s: Status| {
            filter == "all"
                || (filter == "active" && s.active())
                || Status::parse(&filter) == Some(s)
        };
        let mut rows: Vec<Issue> = snap.issues().filter(|i| want(i.status)).cloned().collect();
        match order {
            Some(order) => sort_chronologically(&mut rows, order),
            None => sort_rows(&mut rows),
        }
        let stdout = if json {
            let items: Vec<String> = rows.iter().map(row_json).collect();
            format!("[{}]\n", items.join(","))
        } else {
            let mut out = String::new();
            for i in &rows {
                let pri = i
                    .priority
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into());
                let who = i.assignee.clone().unwrap_or_else(|| "-".into());
                let created = order
                    .map(|_| format!(" {}", i.created_at.as_deref().unwrap_or("-")))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{} {} {pri} {who}{created} {}\n",
                    i.id,
                    i.status.as_str(),
                    i.title
                ));
            }
            out
        };
        Ok(CommandOutput {
            stdout,
            warnings: file_warnings(&snap, |_| true, false),
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
    let p = parse_tail(args, &[("--json", Arity::Switch, "json")], &[])?;
    pos_n(&p, 1, "tree ID")?;
    let json = json || flag(&p, "json");
    with_registry(cwd, |jig| {
        let snap = check::scan(jig);
        let id = resolve_id(&p.pos[0])?;
        let root = snap.resolve(&id).map_err(fail)?;
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
            format!("{}\n", tree_json(&root, &kids, &mut HashSet::new()))
        } else {
            let mut out = String::new();
            tree_text(&mut out, &root, &kids, 0, &mut HashSet::new());
            out
        };
        Ok(CommandOutput {
            stdout,
            warnings: file_warnings(&snap, |_| true, false),
            code: 0,
        })
    })
}

fn tree_json(i: &Issue, kids: &HashMap<String, Vec<Issue>>, seen: &mut HashSet<String>) -> String {
    let mut out = format!(
        "{{\"id\":{},\"title\":{},\"status\":{},\"children\":[",
        issue::esc(&i.id),
        issue::esc(&i.title),
        issue::esc(i.status.as_str())
    );
    if seen.insert(i.id.clone())
        && let Some(list) = kids.get(&i.id)
    {
        let parts: Vec<String> = list.iter().map(|c| tree_json(c, kids, seen)).collect();
        out.push_str(&parts.join(","));
    }
    out.push_str("]}");
    out
}

fn tree_text(
    out: &mut String,
    i: &Issue,
    kids: &HashMap<String, Vec<Issue>>,
    depth: usize,
    seen: &mut HashSet<String>,
) {
    out.push_str(&"  ".repeat(depth));
    out.push_str(&format!("{} {} {}\n", i.id, i.status.as_str(), i.title));
    if !seen.insert(i.id.clone()) {
        return;
    }
    if let Some(list) = kids.get(&i.id) {
        for c in list {
            tree_text(out, c, kids, depth + 1, seen);
        }
    }
}

fn cmd_claim(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("--by", Arity::One, "by"),
            ("--force", Arity::Switch, "force"),
        ],
        &[],
    )?;
    pos_n(&p, 1, "claim ID")?;
    if let Some(b) = opt(&p, "by")
        && !issue::valid_label(b)
    {
        return Err(usage("--by must be a non-empty single line"));
    }
    let force = flag(&p, "force");
    let lock = registry(cwd)?;
    let jig = &lock.jig;
    let l = load(jig, &p.pos[0])?;
    let snap = check::scan(jig);
    if let Err(e) = snap.resolve(&l.issue.id) {
        return Err(fail(e));
    }
    if let Some((problem, _)) = domain::field_problems(&l.issue).first() {
        return Err(fail(format!("issue {}: {problem}", l.issue.id)));
    }
    let today = clock::today();
    let bypassed = domain::eligibility(&l.issue, &today, force, |id| snap.resolve(id))
        .map_err(|e| fail(e.message(&l.issue.id)))?;
    let by = opt(&p, "by").map(|s| s.to_string());
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Claim).map_err(fail)?;
    i.assignee = by.clone();
    i.claim_id = Some(ident::new_claim_id().map_err(fail)?);
    i.updated_at = Some(clock::now_iso());
    if force && !bypassed.is_empty() {
        let who = by
            .as_deref()
            .map(|b| format!(" by {b}"))
            .unwrap_or_default();
        hist_push(
            &mut i,
            hist_entry(&format!(
                "claimed --force{who} bypassing {}",
                bypassed.join(", ")
            )),
        );
    }
    store::write_issue(&l, &i).map_err(fail)?;
    let who = by.map(|b| format!(" as {b}")).unwrap_or_default();
    Ok(format!("claimed {}{who}\n", i.id))
}

fn cmd_approve(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[], &[])?;
    pos_n(&p, 1, "approve ID")?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Approve).map_err(fail)?;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, hist_entry("approved"));
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("approved {}\n", i.id))
}

fn cmd_revise(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--reason", Arity::One, "reason")], &[])?;
    pos_n(&p, 1, "revise ID")?;
    let reason = opt(&p, "reason")
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| usage("--reason is required"))?;
    let reason = validated_history_text(reason, "--reason")?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Revise).map_err(fail)?;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, hist_entry(&history_text("revised:", reason)));
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("returned {} to proposed\n", i.id))
}

fn cmd_unclaim(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[], &[])?;
    pos_n(&p, 1, "unclaim ID")?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Unclaim).map_err(fail)?;
    i.assignee = None;
    i.claim_id = None;
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("unclaimed {}\n", i.id))
}

fn cmd_update(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("-p", Arity::One, "priority"),
            ("--priority", Arity::One, "priority"),
            ("--title", Arity::One, "title"),
            ("--body", Arity::One, "body"),
        ],
        &[],
    )?;
    pos_n(&p, 1, "update ID")?;
    if opt(&p, "title").is_none() && opt(&p, "priority").is_none() && opt(&p, "body").is_none() {
        return Err(usage("update requires at least one change"));
    }
    let body = body_arg(&p)?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    if let Some(t) = opt(&p, "title") {
        if t.is_empty() {
            return Err(usage("empty title"));
        }
        i.title = t.to_string();
    }
    if let Some(v) = opt(&p, "priority") {
        i.priority = Some(parse_priority(v)?);
    }
    if let Some(b) = body {
        i.body = b;
    }
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("updated {}\n", i.id))
}

fn cmd_review(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--summary", Arity::One, "summary")], &[])?;
    pos_n(&p, 1, "review ID")?;
    let summary = opt(&p, "summary")
        .map(|s| validated_history_text(s, "--summary"))
        .transpose()?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Review).map_err(fail)?;
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
    Ok(format!("submitted {} for review\n", i.id))
}

fn cmd_rework(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--reason", Arity::One, "reason")], &[])?;
    pos_n(&p, 1, "rework ID")?;
    let reason = opt(&p, "reason")
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| usage("--reason is required"))?;
    let reason = validated_history_text(reason, "--reason")?;
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Rework).map_err(fail)?;
    if i.claim_id.is_none() {
        i.claim_id = Some(ident::new_claim_id().map_err(fail)?);
    }
    i.updated_at = Some(clock::now_iso());
    hist_push(
        &mut i,
        hist_entry(&history_text("rework requested:", reason)),
    );
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("returned {} for rework\n", i.id))
}

fn cmd_close(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("--reason", Arity::One, "reason"),
            ("--summary", Arity::One, "summary"),
        ],
        &[],
    )?;
    pos_n(&p, 1, "close ID")?;
    let reason = opt(&p, "reason")
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| usage("--reason is required"))?;
    let reason = validated_history_text(reason, "--reason")?;
    let summary = opt(&p, "summary")
        .map(|s| validated_history_text(s, "--summary"))
        .transpose()?;
    let lock = registry(cwd)?;
    let jig = &lock.jig;
    let l = load(jig, &p.pos[0])?;
    let now = clock::now_iso();
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Close).map_err(fail)?;
    i.closed_at = Some(now.clone());
    i.updated_at = Some(now);
    i.claim_id = None;
    if let Some(s) = summary {
        i.closing_summary = Some(s.to_string());
    }
    hist_push(&mut i, hist_entry(&history_text("closed:", reason)));
    store::move_issue(jig, &l, &i).map_err(fail)?;
    Ok(format!("closed {}\n", i.id))
}

fn cmd_reopen(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--reason", Arity::One, "reason")], &[])?;
    pos_n(&p, 1, "reopen ID")?;
    let reason = opt(&p, "reason")
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| usage("--reason is required"))?;
    let reason = validated_history_text(reason, "--reason")?;
    let lock = registry(cwd)?;
    let jig = &lock.jig;
    let l = load(jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    i.status = domain::transition(&i, Action::Reopen).map_err(fail)?;
    let mut entry = hist_entry(&history_text("reopened:", reason));
    if let Some(summary) = i.closing_summary.take() {
        entry.push_str("\n  ");
        entry.push_str(&history_text("closing summary:", &summary));
    }
    i.assignee = None;
    i.claim_id = None;
    i.closed_at = None;
    i.updated_at = Some(clock::now_iso());
    hist_push(&mut i, entry);
    store::move_issue(jig, &l, &i).map_err(fail)?;
    Ok(format!("reopened {}\n", i.id))
}

fn dep_graph(jig: &Path) -> R<HashMap<String, Vec<String>>> {
    let mut g = HashMap::new();
    for l in store::load_all(jig).map_err(fail)? {
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
    let p = parse_tail(args, &[], &[])?;
    pos_n(&p, 3, "dep add|rm ID BLOCKER")?;
    let lock = registry(cwd)?;
    let jig = &lock.jig;
    let id = resolve_id(&p.pos[1])?;
    let blocker = resolve_id(&p.pos[2])?;
    if id == blocker {
        return Err(fail("issue cannot depend on itself"));
    }
    let l = store::find(jig, &id)
        .map_err(fail)?
        .ok_or_else(|| fail(format!("issue {id} not found")))?;
    let mut i = l.issue.clone();
    match p.pos[0].as_str() {
        "add" => {
            store::find(jig, &blocker)
                .map_err(fail)?
                .ok_or_else(|| fail(format!("issue {blocker} not found")))?;
            if i.depends_on.contains(&blocker) {
                return Err(fail(format!("{id} already depends on {blocker}")));
            }
            let g = dep_graph(jig)?;
            if reaches(&g, &blocker, &id) {
                return Err(fail(format!("dependency cycle via {blocker}")));
            }
            i.depends_on.push(blocker.clone());
            i.updated_at = Some(clock::now_iso());
            store::write_issue(&l, &i).map_err(fail)?;
            Ok(format!("{id} now blocked by {blocker}\n"))
        }
        "rm" => {
            if !i.depends_on.contains(&blocker) {
                return Err(fail(format!("{id} does not depend on {blocker}")));
            }
            i.depends_on.retain(|d| d != &blocker);
            i.updated_at = Some(clock::now_iso());
            store::write_issue(&l, &i).map_err(fail)?;
            Ok(format!("{id} no longer blocked by {blocker}\n"))
        }
        other => Err(usage(format!("unknown dep action {other}; use add or rm"))),
    }
}

fn cmd_note(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[("--by", Arity::One, "by")], &[])?;
    pos_n(&p, 2, "note ID TEXT")?;
    if let Some(b) = opt(&p, "by")
        && !issue::valid_label(b)
    {
        return Err(usage("--by must be a non-empty single line"));
    }
    let text = &p.pos[1];
    if text.is_empty() {
        return Err(usage("note text is empty"));
    }
    if text
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(usage(
            "note text must be a single line without control characters or Unicode line separators",
        ));
    }
    let count = text.chars().count();
    if count > 500 {
        return Err(usage(format!(
            "note has {count} characters; maximum is 500"
        )));
    }
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    let mut i = l.issue.clone();
    let who = opt(&p, "by").map(|b| format!("{b}: ")).unwrap_or_default();
    if !i.notes.is_empty() {
        i.notes.push('\n');
    }
    i.notes
        .push_str(&format!("- {} {who}{text}", clock::now_stamp()));
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    Ok(format!("noted {}\n", i.id))
}

fn cmd_defer(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(
        args,
        &[
            ("--until", Arity::One, "until"),
            ("--clear", Arity::Switch, "clear"),
        ],
        &[],
    )?;
    pos_n(&p, 1, "defer ID")?;
    let until = opt(&p, "until");
    let clear = flag(&p, "clear");
    if until.is_some() == clear {
        return Err(usage("defer requires exactly one of --until or --clear"));
    }
    if let Some(d) = until
        && !clock::valid_date(d)
    {
        return Err(usage(format!("invalid defer date {d:?}")));
    }
    let lock = registry(cwd)?;
    let l = load(&lock.jig, &p.pos[0])?;
    if l.place == Place::Closed && until.is_some() {
        return Err(fail(format!("issue {} is closed", l.issue.id)));
    }
    let mut i = l.issue.clone();
    i.defer_until = until.map(|d| d.to_string());
    i.updated_at = Some(clock::now_iso());
    store::write_issue(&l, &i).map_err(fail)?;
    match until {
        Some(d) => Ok(format!("deferred {} until {d}\n", i.id)),
        None => Ok(format!("cleared deferral on {}\n", i.id)),
    }
}

fn load_memory(jig: &Path) -> R<memory::Memory> {
    let text = std::fs::read_to_string(jig.join("memory.md"))
        .map_err(|e| fail(format!("read memory.md: {e}")))?;
    memory::parse(&text).map_err(|e| fail(format!("memory.md: {e}")))
}

fn cmd_remember(args: &[String], cwd: &Path) -> R<String> {
    let p = parse_tail(args, &[], &[])?;
    pos_n(&p, 2, "remember TOPIC TEXT")?;
    let topic = &p.pos[0];
    if !memory::valid_topic(topic) {
        return Err(usage(format!("invalid topic {topic:?}")));
    }
    let lock = registry(cwd)?;
    let jig = &lock.jig;
    let mut mem = load_memory(jig)?;
    let value = p.pos[1].clone();
    match mem.entries.iter_mut().find(|(t, _)| t == topic) {
        Some((_, v)) => *v = value,
        None => mem.entries.push((topic.clone(), value)),
    }
    let text = memory::render(&mem);
    store::atomic_write(&jig.join("memory.md"), text.as_bytes()).map_err(fail)?;
    Ok(format!("remembered {topic}\n"))
}

fn cmd_recall(args: &[String], cwd: &Path, json: bool) -> R<String> {
    let p = parse_tail(args, &[("--json", Arity::Switch, "json")], &[])?;
    if p.pos.len() > 1 {
        return Err(usage("expected at most one topic"));
    }
    let json = json || flag(&p, "json");
    with_registry(cwd, |jig| {
        let mem = load_memory(jig)?;
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
