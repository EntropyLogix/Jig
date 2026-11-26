use crate::clock;
use crate::domain;
use crate::instructions;
use crate::issue::Issue;
use crate::memory;
use crate::store::{self, Place};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Problem {
    pub path: PathBuf,
    pub problem: String,
    pub hint: String,
}

impl Problem {
    fn at(path: PathBuf, problem: impl Into<String>, hint: impl Into<String>) -> Problem {
        Problem {
            path,
            problem: problem.into(),
            hint: hint.into(),
        }
    }
}

pub(crate) struct Record {
    pub path: PathBuf,
    pub place: Place,
    pub issue: Issue,
    pub usable: bool,
}

pub(crate) struct Snapshot {
    usable: BTreeMap<String, Issue>,
    unavailable: BTreeMap<String, String>,
    pub records: Vec<Record>,
    pub problems: Vec<Problem>,
}

impl Snapshot {
    pub(crate) fn resolve(&self, id: &str) -> Result<Issue, String> {
        if let Some(issue) = self.usable.get(id) {
            return Ok(issue.clone());
        }
        if let Some(reason) = self.unavailable.get(id) {
            return Err(format!("issue {id} is unusable: {reason}"));
        }
        Err(format!("missing issue {id}"))
    }

    pub(crate) fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.usable.values()
    }
}

fn rel(place: Place, name: &std::ffi::OsStr) -> PathBuf {
    PathBuf::from(".jig").join(place.dir()).join(name)
}

pub(crate) fn scan(jig: &Path) -> Snapshot {
    let mut records = Vec::new();
    let mut problems = Vec::new();
    let mut claimants: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for place in [Place::Open, Place::Closed] {
        let dir = jig.join(place.dir());
        let rel_dir = PathBuf::from(".jig").join(place.dir());
        match store::metadata_cause(&dir, true) {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(e) => {
                problems.push(Problem::at(
                    rel_dir,
                    e,
                    "restore a real directory at this path",
                ));
                continue;
            }
        }
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                problems.push(Problem::at(
                    rel_dir,
                    format!("cannot list directory: {e}"),
                    "fix directory permissions",
                ));
                continue;
            }
        };
        for ent in rd {
            let ent = match ent {
                Ok(ent) => ent,
                Err(e) => {
                    problems.push(Problem::at(
                        rel_dir.clone(),
                        format!("cannot list entry: {e}"),
                        "fix directory permissions",
                    ));
                    continue;
                }
            };
            let name = ent.file_name();
            let path = rel(place, &name);
            let Some(name) = name.to_str() else {
                problems.push(Problem::at(
                    path,
                    "unexpected file with a non-UTF-8 name",
                    "remove or rename the file",
                ));
                continue;
            };
            if name.starts_with('.') && name.contains(".tmp-") {
                problems.push(Problem::at(
                    path,
                    "abandoned temporary file",
                    "remove it after checking no jig process is running",
                ));
                continue;
            }
            if name.starts_with('.') || !name.ends_with(".md") {
                problems.push(Problem::at(
                    path,
                    "unexpected file in an issue directory",
                    "remove or rename the file",
                ));
                continue;
            }
            let stem = name[..name.len() - 3].to_string();
            claimants.entry(stem).or_default().push(path.clone());
            match store::read_issue_file(&ent.path()) {
                Err(e) => {
                    problems.push(Problem::at(
                        path,
                        e,
                        "fix the file or restore it from version control",
                    ));
                }
                Ok((issue, _)) => {
                    if issue.id != name[..name.len() - 3] {
                        claimants
                            .entry(issue.id.clone())
                            .or_default()
                            .push(path.clone());
                    }
                    records.push(Record {
                        path,
                        place,
                        issue,
                        usable: true,
                    });
                }
            }
        }
    }
    let mut unavailable: BTreeMap<String, String> = BTreeMap::new();
    for (id, paths) in &claimants {
        if paths.len() < 2 {
            continue;
        }
        for path in paths {
            problems.push(Problem::at(
                path.clone(),
                format!("duplicate issue id {id}"),
                "keep one record for this issue and remove the others",
            ));
        }
        unavailable.insert(id.clone(), "duplicate issue id".to_string());
    }
    for rec in &mut records {
        let stem = rec
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if rec.issue.id != stem {
            problems.push(Problem::at(
                rec.path.clone(),
                format!("file name does not match issue id {}", rec.issue.id),
                format!("rename the file to {}.md or fix the id field", rec.issue.id),
            ));
            rec.usable = false;
            unavailable
                .entry(rec.issue.id.clone())
                .or_insert_with(|| "file name does not match issue id".to_string());
            unavailable
                .entry(stem)
                .or_insert_with(|| format!("file decodes as issue {}", rec.issue.id));
            continue;
        }
        if Place::of(rec.issue.status) != rec.place {
            problems.push(Problem::at(
                rec.path.clone(),
                format!(
                    "status {} does not belong in {}/",
                    rec.issue.status.as_str(),
                    rec.place.dir()
                ),
                "move the file to the other directory or fix the status field",
            ));
            rec.usable = false;
            unavailable
                .entry(rec.issue.id.clone())
                .or_insert_with(|| "issue file is in the wrong directory".to_string());
            continue;
        }
        if unavailable.contains_key(&rec.issue.id) {
            rec.usable = false;
        }
    }
    let usable = records
        .iter()
        .filter(|r| r.usable)
        .map(|r| (r.issue.id.clone(), r.issue.clone()))
        .collect();
    problems.sort_by(|a, b| a.path.cmp(&b.path).then(a.problem.cmp(&b.problem)));
    Snapshot {
        usable,
        unavailable,
        records,
        problems,
    }
}

fn dependency_cycles(snap: &Snapshot) -> Vec<Problem> {
    let mut graph: BTreeMap<&str, &Issue> = BTreeMap::new();
    let mut ambiguous: BTreeSet<String> = BTreeSet::new();
    for rec in &snap.records {
        let stem = rec
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if rec.issue.id != stem {
            continue;
        }
        if graph.insert(rec.issue.id.as_str(), &rec.issue).is_some() {
            ambiguous.insert(rec.issue.id.clone());
        }
    }
    for id in &ambiguous {
        graph.remove(id.as_str());
    }

    let mut problems = Vec::new();
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    let mut reported: BTreeSet<Vec<String>> = BTreeSet::new();
    for start in graph.keys() {
        if state.get(start) == Some(&2) {
            continue;
        }
        let mut path: Vec<&str> = vec![start];
        let mut iters: Vec<usize> = vec![0];
        state.insert(*start, 1);
        while let Some(next) = iters.last_mut() {
            let node = *path.last().unwrap();
            let deps = &graph[node].depends_on;
            if *next >= deps.len() {
                state.insert(node, 2);
                path.pop();
                iters.pop();
                continue;
            }
            let dep = deps[*next].as_str();
            *next += 1;
            match state.get(dep).copied().unwrap_or(0) {
                2 => continue,
                1 => {
                    let at = path.iter().position(|n| *n == dep).unwrap();
                    let mut cycle: Vec<String> = path[at..].iter().map(|s| s.to_string()).collect();
                    let min_at = cycle
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| a.cmp(b))
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    cycle.rotate_left(min_at);
                    let first = cycle[0].clone();
                    let last = cycle.last().unwrap().clone();
                    if !reported.insert(cycle.clone()) {
                        continue;
                    }
                    let mut display = cycle.join(" -> ");
                    display.push_str(" -> ");
                    display.push_str(&first);
                    problems.push(Problem::at(
                        PathBuf::from(".jig")
                            .join(Place::of(graph[first.as_str()].status).dir())
                            .join(format!("{first}.md")),
                        format!("dependency cycle {display}"),
                        format!("remove one edge, e.g. `jig dep rm {last} {first}`"),
                    ));
                }
                _ => {
                    if graph.contains_key(dep) {
                        state.insert(dep, 1);
                        path.push(dep);
                        iters.push(0);
                    }
                }
            }
        }
    }
    problems
}

pub(crate) fn collect(jig: &Path) -> Vec<Problem> {
    let snap = scan(jig);
    let mut problems = snap.problems.clone();

    let rel_mem = PathBuf::from(".jig/memory.md");
    match store::metadata_cause(&jig.join("memory.md"), false) {
        Err(e) => problems.push(Problem::at(
            rel_mem,
            e,
            "restore memory.md as a regular file or run `jig init`",
        )),
        Ok(None) => problems.push(Problem::at(
            rel_mem,
            "memory.md is missing",
            "run `jig init` to create it",
        )),
        Ok(Some(_)) => match fs::read_to_string(jig.join("memory.md")) {
            Err(e) => problems.push(Problem::at(
                rel_mem,
                format!("cannot read: {e}"),
                "fix file permissions or restore memory.md",
            )),
            Ok(text) => {
                if let Err(e) = memory::parse(&text) {
                    problems.push(Problem::at(rel_mem, e, "fix memory.md manually"));
                }
            }
        },
    }

    let rel_gi = PathBuf::from(".jig/.gitignore");
    const LEGACY: &str = "/.*.tmp-*";
    const RULE: &str = ".*.tmp-*";
    match store::metadata_cause(&jig.join(".gitignore"), false) {
        Err(e) => problems.push(Problem::at(
            rel_gi,
            e,
            "restore .jig/.gitignore as a regular file",
        )),
        Ok(None) => problems.push(Problem::at(
            rel_gi,
            ".gitignore is missing",
            format!("add the line `{RULE}` to .jig/.gitignore"),
        )),
        Ok(Some(_)) => match fs::read_to_string(jig.join(".gitignore")) {
            Err(e) => problems.push(Problem::at(
                rel_gi,
                format!("cannot read: {e}"),
                "fix file permissions or restore .jig/.gitignore",
            )),
            Ok(text) => {
                if text.lines().any(|l| l == LEGACY) {
                    problems.push(Problem::at(
                        rel_gi,
                        format!("legacy `{LEGACY}` rule does not ignore nested temporary files"),
                        format!("replace `{LEGACY}` with `{RULE}` in .jig/.gitignore"),
                    ));
                }
            }
        },
    }

    if let Some((problem, hint)) = instructions::problem(jig.parent().unwrap()) {
        problems.push(Problem::at(PathBuf::from("AGENTS.md"), problem, hint));
    }

    match store::inspect_move(jig) {
        Ok(None) => {}
        Ok(Some(id)) => problems.push(Problem::at(
            PathBuf::from(".jig/.move"),
            format!("pending move journal for {id}"),
            "finish it with any write command in a writable checkout, or resolve .jig/.move manually",
        )),
        Err(e) => problems.push(Problem::at(
            PathBuf::from(".jig/.move"),
            e,
            "resolve .jig/.move manually",
        )),
    }

    for rec in &snap.records {
        for (problem, hint) in domain::field_problems(&rec.issue) {
            problems.push(Problem::at(
                rec.path.clone(),
                problem,
                format!("{hint} in {}", rec.path.display()),
            ));
        }
        if let Some(parent) = &rec.issue.parent
            && let Err(e) = snap.resolve(parent)
        {
            problems.push(Problem::at(
                rec.path.clone(),
                format!("parent {parent}: {e}"),
                format!(
                    "restore the issue file for {parent} or edit parent in {}",
                    rec.path.display()
                ),
            ));
        }
        for dep in &rec.issue.depends_on {
            if let Err(e) = snap.resolve(dep) {
                problems.push(Problem::at(
                    rec.path.clone(),
                    format!("blocker {dep}: {e}"),
                    format!(
                        "restore the issue file for {dep} or run `jig dep rm {} {dep}`",
                        rec.issue.id
                    ),
                ));
            }
        }
    }

    let today = clock::today();
    for rec in &snap.records {
        if !rec.usable {
            continue;
        }
        let direct_ok = rec
            .issue
            .parent
            .as_deref()
            .is_none_or(|p| snap.resolve(p).is_ok())
            && rec.issue.depends_on.iter().all(|d| snap.resolve(d).is_ok());
        if !direct_ok {
            continue;
        }
        if let Err(domain::Ineligible::Reference(reference)) =
            domain::eligibility(&rec.issue, &today, false, |id| snap.resolve(id))
            && !reference.starts_with("ancestor cycle")
        {
            problems.push(Problem::at(
                rec.path.clone(),
                format!("unresolved {reference}"),
                "fix or restore the referenced issue; see the other reported problems",
            ));
        }
    }

    problems.extend(dependency_cycles(&snap));
    problems.sort_by(|a, b| a.path.cmp(&b.path).then(a.problem.cmp(&b.problem)));
    problems
}
