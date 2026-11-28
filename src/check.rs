use crate::clock;
use crate::config;
use crate::domain;
use crate::instructions;
use crate::issue::{Issue, Status};
use crate::memory;
use crate::names::Names;
use crate::store::{self, Place};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    IssueDirectory,
    IssueDirectoryUnlistable,
    IssueEntryUnlistable,
    IssueNameNotUtf8,
    TemporaryFile,
    UnexpectedFile,
    IssueUnreadable,
    IssuePriority,
    DuplicateId,
    FileNameMismatch,
    WrongDirectory,
    DependencyCycle,
    DocsUnlistable,
    DocsDirectory,
    DocsEntryUnlistable,
    DocsNameNotUtf8,
    DocsEntryUninspectable,
    DocsSymlink,
    DocsNotDirectory,
    DocsOrphan,
    MemoryFile,
    MemoryMissing,
    MemoryUnreadable,
    MemoryMalformed,
    GitignoreFile,
    GitignoreMissing,
    GitignoreUnreadable,
    GitignoreLegacyRule,
    AgentInstructions,
    PendingMove,
    MoveJournal,
    FieldConflict,
    MissingParent,
    MissingBlocker,
    UnresolvedReference,
    ConfigFile,
    ConfigMissing,
    ConfigInvalid,
    DuplicateNumber,
}

impl ProblemKind {
    pub const ALL: [ProblemKind; 39] = [
        ProblemKind::IssueDirectory,
        ProblemKind::IssueDirectoryUnlistable,
        ProblemKind::IssueEntryUnlistable,
        ProblemKind::IssueNameNotUtf8,
        ProblemKind::TemporaryFile,
        ProblemKind::UnexpectedFile,
        ProblemKind::IssueUnreadable,
        ProblemKind::IssuePriority,
        ProblemKind::DuplicateId,
        ProblemKind::FileNameMismatch,
        ProblemKind::WrongDirectory,
        ProblemKind::DependencyCycle,
        ProblemKind::DocsUnlistable,
        ProblemKind::DocsDirectory,
        ProblemKind::DocsEntryUnlistable,
        ProblemKind::DocsNameNotUtf8,
        ProblemKind::DocsEntryUninspectable,
        ProblemKind::DocsSymlink,
        ProblemKind::DocsNotDirectory,
        ProblemKind::DocsOrphan,
        ProblemKind::MemoryFile,
        ProblemKind::MemoryMissing,
        ProblemKind::MemoryUnreadable,
        ProblemKind::MemoryMalformed,
        ProblemKind::GitignoreFile,
        ProblemKind::GitignoreMissing,
        ProblemKind::GitignoreUnreadable,
        ProblemKind::GitignoreLegacyRule,
        ProblemKind::AgentInstructions,
        ProblemKind::PendingMove,
        ProblemKind::MoveJournal,
        ProblemKind::FieldConflict,
        ProblemKind::MissingParent,
        ProblemKind::MissingBlocker,
        ProblemKind::UnresolvedReference,
        ProblemKind::ConfigFile,
        ProblemKind::ConfigMissing,
        ProblemKind::ConfigInvalid,
        ProblemKind::DuplicateNumber,
    ];
}

#[derive(Clone)]
pub(crate) struct Problem {
    pub kind: ProblemKind,
    pub path: PathBuf,
    pub problem: String,
    pub hint: String,
}

impl Problem {
    fn at(
        kind: ProblemKind,
        path: PathBuf,
        problem: impl Into<String>,
        hint: impl Into<String>,
    ) -> Problem {
        Problem {
            kind,
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
    reserved: Vec<u64>,
    usable: BTreeMap<String, Issue>,
    unavailable: BTreeMap<String, Unavailable>,
    children: BTreeMap<String, Vec<ChildRef>>,
    pub records: Vec<Record>,
    pub problems: Vec<Problem>,
}

struct Unavailable {
    reason: String,
    path: PathBuf,
}

#[derive(Clone)]
struct ChildRef {
    id: String,
    status: Option<Status>,
    path: PathBuf,
    usable: bool,
}

impl Snapshot {
    pub(crate) fn resolve(&self, id: &str) -> Result<Issue, domain::ResolveError> {
        if let Some(issue) = self.usable.get(id) {
            return Ok(issue.clone());
        }
        if let Some(entry) = self.unavailable.get(id) {
            return Err(domain::ResolveError::Unusable {
                id: id.to_string(),
                path: entry.path.clone(),
                reason: entry.reason.clone(),
            });
        }
        Err(domain::ResolveError::Missing { id: id.to_string() })
    }

    pub(crate) fn names(&self, prefix: &str) -> Names {
        let mut names = Names::new(prefix);
        for record in &self.records {
            names.add(&record.issue, record.path.clone());
        }
        for number in &self.reserved {
            names.reserve(*number);
        }
        names
    }

    pub(crate) fn issues(&self) -> impl Iterator<Item = &Issue> {
        self.usable.values()
    }

    pub(crate) fn descendants(&self, id: &str) -> Vec<(String, Status)> {
        let mut result = Vec::new();
        let mut stack = self
            .children
            .get(id)
            .into_iter()
            .flat_map(|issues| issues.iter())
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        while let Some(child) = stack.pop() {
            if !seen.insert(child.id.as_str()) {
                continue;
            }
            if child.usable
                && child.status.is_some()
                && let Some(issue) = self.usable.get(&child.id)
            {
                result.push((issue.id.clone(), issue.status));
            }
            if let Some(nested) = self.children.get(child.id.as_str()) {
                stack.extend(nested.iter());
            }
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    pub(crate) fn unresolved_descendants(&self, id: &str) -> Vec<PathBuf> {
        let mut result = Vec::new();
        let mut stack = self
            .children
            .get(id)
            .into_iter()
            .flat_map(|children| children.iter())
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        while let Some(child) = stack.pop() {
            if !seen.insert(child.id.clone()) {
                continue;
            }
            if !child.usable {
                result.push(child.path.clone());
            }
            if let Some(nested) = self.children.get(&child.id) {
                stack.extend(nested.iter());
            }
        }
        result.sort();
        result
    }
}

fn rel(place: Place, name: &std::ffi::OsStr) -> PathBuf {
    PathBuf::from(".billet").join(place.dir()).join(name)
}

fn stray_number(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("number: "))
        .and_then(|value| value.trim().parse().ok())
}

pub(crate) fn scan(billet: &Path) -> Snapshot {
    let mut reserved = Vec::new();
    let mut records = Vec::new();
    let mut problems = Vec::new();
    let mut claimants: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut parse_failures: Vec<(String, PathBuf)> = Vec::new();
    for place in [Place::Open, Place::Closed] {
        let dir = billet.join(place.dir());
        let rel_dir = PathBuf::from(".billet").join(place.dir());
        match store::metadata_cause(&dir, true) {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(e) => {
                problems.push(Problem::at(
                    ProblemKind::IssueDirectory,
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
                    ProblemKind::IssueDirectoryUnlistable,
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
                        ProblemKind::IssueEntryUnlistable,
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
                    ProblemKind::IssueNameNotUtf8,
                    path,
                    "unexpected file with a non-UTF-8 name",
                    "remove or rename the file",
                ));
                continue;
            };
            if name.starts_with('.') && name.contains(".tmp-") {
                problems.push(Problem::at(
                    ProblemKind::TemporaryFile,
                    path,
                    "abandoned temporary file",
                    "remove it after checking no blt process is running",
                ));
                continue;
            }
            if name.starts_with('.') || !name.ends_with(".md") {
                problems.push(Problem::at(
                    ProblemKind::UnexpectedFile,
                    path,
                    "unexpected file in an issue directory",
                    "remove or rename the file",
                ));
                continue;
            }
            let stem = name[..name.len() - 3].to_string();
            claimants
                .entry(stem.clone())
                .or_default()
                .push(path.clone());
            match store::read_issue_file(&ent.path()) {
                Err(e) => {
                    let (kind, hint) = if e.starts_with(crate::issue::BAD_PRIORITY)
                        || e == "missing frontmatter key \"priority\""
                    {
                        (
                            ProblemKind::IssuePriority,
                            "set the `priority` field to an integer from 0 to 4",
                        )
                    } else {
                        (
                            ProblemKind::IssueUnreadable,
                            "fix the file or restore it from version control",
                        )
                    };
                    problems.push(Problem::at(kind, path, e, hint));
                    if !stem.contains('.')
                        && let Some(number) = fs::read_to_string(ent.path())
                            .ok()
                            .as_deref()
                            .and_then(stray_number)
                    {
                        reserved.push(number);
                    }
                    parse_failures.push((stem, rel(place, &ent.file_name())));
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
    let mut unavailable: BTreeMap<String, Unavailable> = BTreeMap::new();
    for (id, paths) in &claimants {
        if paths.len() < 2 {
            continue;
        }
        for path in paths {
            problems.push(Problem::at(
                ProblemKind::DuplicateId,
                path.clone(),
                format!("duplicate issue key {id}"),
                "keep one record for this issue and remove the others",
            ));
        }
        unavailable.insert(
            id.clone(),
            Unavailable {
                reason: "duplicate issue key".to_string(),
                path: paths[0].clone(),
            },
        );
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
                ProblemKind::FileNameMismatch,
                rec.path.clone(),
                format!("file name does not match issue key {}", rec.issue.id),
                format!("rename the file to {}.md or fix the id field", rec.issue.id),
            ));
            rec.usable = false;
            unavailable
                .entry(rec.issue.id.clone())
                .or_insert_with(|| Unavailable {
                    reason: "file name does not match issue key".to_string(),
                    path: rec.path.clone(),
                });
            unavailable.entry(stem).or_insert_with(|| Unavailable {
                reason: format!("file decodes as issue key {}", rec.issue.id),
                path: rec.path.clone(),
            });
            continue;
        }
        if Place::of(rec.issue.status) != rec.place {
            problems.push(Problem::at(
                ProblemKind::WrongDirectory,
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
                .or_insert_with(|| Unavailable {
                    reason: "issue file is in the wrong directory".to_string(),
                    path: rec.path.clone(),
                });
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
    for (stem, path) in parse_failures {
        unavailable.entry(stem).or_insert_with(|| Unavailable {
            reason: "file cannot be parsed".to_string(),
            path,
        });
    }
    let mut children: BTreeMap<String, Vec<ChildRef>> = BTreeMap::new();
    for rec in &records {
        if let Some(parent) = rec.issue.parent.as_deref() {
            let child_usable = rec.usable && crate::domain::field_problems(&rec.issue).is_empty();
            children
                .entry(parent.to_string())
                .or_default()
                .push(ChildRef {
                    id: rec.issue.id.clone(),
                    status: Some(rec.issue.status),
                    path: rec.path.clone(),
                    usable: child_usable,
                });
        }
    }
    for (stem, Unavailable { path, .. }) in unavailable.iter() {
        let mut child = stem.as_str();
        while let Some((parent, _)) = child.rsplit_once('.') {
            if !children
                .entry(parent.to_string())
                .or_default()
                .iter()
                .any(|existing| existing.id == *stem && existing.path == *path)
            {
                children
                    .entry(parent.to_string())
                    .or_default()
                    .push(ChildRef {
                        id: stem.clone(),
                        status: None,
                        path: path.clone(),
                        usable: false,
                    });
            }
            child = parent;
        }
    }
    problems.sort_by(|a, b| a.path.cmp(&b.path).then(a.problem.cmp(&b.problem)));
    Snapshot {
        reserved,
        usable,
        unavailable,
        children,
        records,
        problems,
    }
}

fn dependency_cycles(snap: &Snapshot, names: &Names) -> Vec<Problem> {
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
                    let mut display = cycle
                        .iter()
                        .map(|key| names.name(key))
                        .collect::<Vec<_>>()
                        .join(" -> ");
                    display.push_str(" -> ");
                    display.push_str(&names.name(&first));
                    problems.push(Problem::at(
                        ProblemKind::DependencyCycle,
                        PathBuf::from(".billet")
                            .join(Place::of(graph[first.as_str()].status).dir())
                            .join(format!("{first}.md")),
                        format!("dependency cycle {display}"),
                        format!(
                            "remove one edge, e.g. `blt dep rm {} {}`",
                            names.name(&last),
                            names.name(&first)
                        ),
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

fn issue_docs_problems(billet: &Path, snap: &Snapshot) -> Vec<Problem> {
    let docs = billet.join("docs");
    let mut problems = Vec::new();
    let metadata = match store::metadata_cause(&docs, true) {
        Ok(Some(_)) => match fs::read_dir(&docs) {
            Ok(entries) => entries,
            Err(error) => {
                problems.push(Problem::at(
                    ProblemKind::DocsUnlistable,
                    PathBuf::from(".billet/docs"),
                    format!("cannot list directory: {error}"),
                    "fix directory permissions",
                ));
                return problems;
            }
        },
        Ok(None) => return problems,
        Err(error) => {
            problems.push(Problem::at(
                ProblemKind::DocsDirectory,
                PathBuf::from(".billet/docs"),
                error,
                "restore a real directory at this path or remove it",
            ));
            return problems;
        }
    };
    let existing: BTreeSet<&str> = snap
        .records
        .iter()
        .map(|rec| rec.issue.id.as_str())
        .collect();
    for entry in metadata {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                problems.push(Problem::at(
                    ProblemKind::DocsEntryUnlistable,
                    PathBuf::from(".billet/docs"),
                    format!("cannot list entry: {error}"),
                    "fix directory permissions",
                ));
                continue;
            }
        };
        let path = PathBuf::from(".billet/docs").join(entry.file_name());
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(name) => name,
            None => {
                problems.push(Problem::at(
                    ProblemKind::DocsNameNotUtf8,
                    path,
                    "name is not valid UTF-8",
                    "rename it to the issue ID or remove it",
                ));
                continue;
            }
        };
        let entry_metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) => {
                problems.push(Problem::at(
                    ProblemKind::DocsEntryUninspectable,
                    path,
                    format!("cannot inspect entry: {error}"),
                    "rename it to the issue ID or remove it",
                ));
                continue;
            }
        };
        if entry_metadata.file_type().is_symlink() {
            problems.push(Problem::at(
                ProblemKind::DocsSymlink,
                path,
                "must be a real directory (not a symbolic link)",
                "rename it to the issue ID or remove it",
            ));
        } else if !entry_metadata.is_dir() {
            problems.push(Problem::at(
                ProblemKind::DocsNotDirectory,
                path,
                "entry is not a directory",
                "rename it to the issue ID or remove it",
            ));
        } else if !existing.contains(name) {
            problems.push(Problem::at(
                ProblemKind::DocsOrphan,
                path,
                format!("no issue with id {name} exists"),
                "rename it to the issue ID or remove it",
            ));
        }
    }
    problems
}

pub fn problem_kinds(root: &Path) -> Vec<(ProblemKind, PathBuf)> {
    collect(&root.join(".billet"))
        .into_iter()
        .map(|problem| (problem.kind, problem.path))
        .collect()
}

pub(crate) fn collect(billet: &Path) -> Vec<Problem> {
    let snap = scan(billet);
    let mut problems = snap.problems.clone();
    problems.extend(issue_docs_problems(billet, &snap));

    let rel_config = PathBuf::from(".billet/config.json");
    let mut prefix = None;
    match store::metadata_cause(&billet.join(config::FILE), false) {
        Err(e) => problems.push(Problem::at(
            ProblemKind::ConfigFile,
            rel_config.clone(),
            e,
            "restore .billet/config.json as a regular file",
        )),
        Ok(None) => problems.push(Problem::at(
            ProblemKind::ConfigMissing,
            rel_config.clone(),
            "config.json is missing",
            "run `blt init` to write the default configuration",
        )),
        Ok(Some(_)) => match config::read(billet) {
            Ok(loaded) => prefix = Some(loaded.prefix),
            Err(e) => problems.push(Problem::at(
                ProblemKind::ConfigInvalid,
                rel_config,
                e,
                format!(
                    "write {{\"prefix\": \"{}\"}} or another valid prefix ({})",
                    config::DEFAULT_PREFIX,
                    config::PREFIX_RULE
                ),
            )),
        },
    }
    let names = match &prefix {
        Some(prefix) => snap.names(prefix),
        None => Names::new(""),
    };
    if prefix.is_some() {
        for (number, roots) in names.duplicates() {
            for root in roots {
                let others = roots
                    .iter()
                    .filter(|other| other.key != root.key)
                    .map(|other| other.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                problems.push(Problem::at(
                    ProblemKind::DuplicateNumber,
                    root.path.clone(),
                    format!("number {}{number} is also used by {others}", names.prefix),
                    format!(
                        "a person gives one of them an unused number: `blt update {} --number N`",
                        root.key
                    ),
                ));
            }
        }
    }

    let rel_mem = PathBuf::from(".billet/memory.md");
    match store::metadata_cause(&billet.join("memory.md"), false) {
        Err(e) => problems.push(Problem::at(
            ProblemKind::MemoryFile,
            rel_mem,
            e,
            "restore memory.md as a regular file or run `blt init`",
        )),
        Ok(None) => problems.push(Problem::at(
            ProblemKind::MemoryMissing,
            rel_mem,
            "memory.md is missing",
            "run `blt init` to create it",
        )),
        Ok(Some(_)) => match fs::read_to_string(billet.join("memory.md")) {
            Err(e) => problems.push(Problem::at(
                ProblemKind::MemoryUnreadable,
                rel_mem,
                format!("cannot read: {e}"),
                "fix file permissions or restore memory.md",
            )),
            Ok(text) => {
                if let Err(e) = memory::parse(&text) {
                    problems.push(Problem::at(
                        ProblemKind::MemoryMalformed,
                        rel_mem,
                        e,
                        "fix memory.md manually",
                    ));
                }
            }
        },
    }

    let rel_gi = PathBuf::from(".billet/.gitignore");
    const LEGACY: &str = "/.*.tmp-*";
    const RULE: &str = ".*.tmp-*";
    match store::metadata_cause(&billet.join(".gitignore"), false) {
        Err(e) => problems.push(Problem::at(
            ProblemKind::GitignoreFile,
            rel_gi,
            e,
            "restore .billet/.gitignore as a regular file",
        )),
        Ok(None) => problems.push(Problem::at(
            ProblemKind::GitignoreMissing,
            rel_gi,
            ".gitignore is missing",
            format!("add the line `{RULE}` to .billet/.gitignore"),
        )),
        Ok(Some(_)) => match fs::read_to_string(billet.join(".gitignore")) {
            Err(e) => problems.push(Problem::at(
                ProblemKind::GitignoreUnreadable,
                rel_gi,
                format!("cannot read: {e}"),
                "fix file permissions or restore .billet/.gitignore",
            )),
            Ok(text) => {
                if text.lines().any(|l| l == LEGACY) {
                    problems.push(Problem::at(
                        ProblemKind::GitignoreLegacyRule,
                        rel_gi,
                        format!("legacy `{LEGACY}` rule does not ignore nested temporary files"),
                        format!("replace `{LEGACY}` with `{RULE}` in .billet/.gitignore"),
                    ));
                }
            }
        },
    }

    if let Some((problem, hint)) = instructions::problem(billet.parent().unwrap()) {
        problems.push(Problem::at(
            ProblemKind::AgentInstructions,
            PathBuf::from("AGENTS.md"),
            problem,
            hint,
        ));
    }

    match store::inspect_move(billet) {
        Ok(None) => {}
        Ok(Some(id)) => problems.push(Problem::at(
            ProblemKind::PendingMove,
            PathBuf::from(".billet/.move"),
            format!("pending move journal for {id}"),
            "finish it with any write command in a writable checkout, or resolve .billet/.move manually",
        )),
        Err(e) => problems.push(Problem::at(
            ProblemKind::MoveJournal,
            PathBuf::from(".billet/.move"),
            e,
            "resolve .billet/.move manually",
        )),
    }

    for rec in &snap.records {
        for (problem, hint) in domain::field_problems(&rec.issue) {
            problems.push(Problem::at(
                ProblemKind::FieldConflict,
                rec.path.clone(),
                problem,
                format!("{hint} in {}", rec.path.display()),
            ));
        }
        if let Some(parent) = &rec.issue.parent
            && let Err(e) = snap.resolve(parent)
        {
            problems.push(Problem::at(
                ProblemKind::MissingParent,
                rec.path.clone(),
                format!("parent {}: {}", names.name(parent), e.text(&names)),
                format!(
                    "restore the issue file for {} or edit parent in {}",
                    names.name(parent),
                    rec.path.display()
                ),
            ));
        }
        for dep in &rec.issue.depends_on {
            if let Err(e) = snap.resolve(dep) {
                problems.push(Problem::at(
                    ProblemKind::MissingBlocker,
                    rec.path.clone(),
                    format!("blocker {}: {}", names.name(dep), e.text(&names)),
                    format!(
                        "restore the issue file for {} or run `blt dep rm {} {}`",
                        names.name(dep),
                        names.name(&rec.issue.id),
                        names.name(dep)
                    ),
                ));
            }
        }
    }

    let today = clock::today();
    let field_problem_paths: BTreeSet<String> = snap
        .records
        .iter()
        .filter(|child| !crate::domain::field_problems(&child.issue).is_empty())
        .map(|child| child.path.display().to_string())
        .collect();
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
        let eligibility = domain::eligibility(
            &rec.issue,
            &today,
            |id| snap.resolve(id),
            |id| snap.descendants(id),
            |id| snap.unresolved_descendants(id),
        );
        if !eligibility.unresolved.is_empty() {
            for reference in eligibility.unresolved {
                if let crate::domain::Unresolved::Subtask { path } = &reference
                    && field_problem_paths.contains(&path.display().to_string())
                {
                    continue;
                }
                problems.push(Problem::at(
                    ProblemKind::UnresolvedReference,
                    rec.path.clone(),
                    format!("unresolved {}", reference.text(&names)),
                    "fix or restore the referenced issue; see the other reported problems",
                ));
            }
        }
    }

    problems.extend(dependency_cycles(&snap, &names));
    problems.sort_by(|a, b| a.path.cmp(&b.path).then(a.problem.cmp(&b.problem)));
    problems
}
