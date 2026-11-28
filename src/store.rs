use crate::domain;
use crate::issue::{self, Issue, Status};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
    Open,
    Closed,
}

impl Place {
    pub fn dir(self) -> &'static str {
        match self {
            Place::Open => "open",
            Place::Closed => "closed",
        }
    }

    pub fn of(status: Status) -> Place {
        if status.active() {
            Place::Open
        } else {
            Place::Closed
        }
    }

    pub fn other(self) -> Place {
        match self {
            Place::Open => Place::Closed,
            Place::Closed => Place::Open,
        }
    }
}

pub struct Loaded {
    pub issue: Issue,
    pub place: Place,
    pub path: PathBuf,
    pub text: String,
}

const MOVE_MAGIC: &str = "BILLET-MOVE-1";
const GITIGNORE: &str = "/.lock\n/.move\n.*.tmp-*\n";
const MEMORY_INIT: &str = "# Project Memory\n";

pub(crate) fn diagnostic_path(path: &Path) -> &Path {
    path.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".billet"))
        .and_then(|billet| billet.parent())
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
}

pub(crate) fn metadata_cause(path: &Path, directory: bool) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if (if directory { m.is_dir() } else { m.is_file() }) => Ok(Some(m)),
        Ok(_) => Err(format!(
            "must be a real {} (not a symbolic link)",
            if directory {
                "directory"
            } else {
                "regular file"
            }
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot inspect: {e}")),
    }
}

pub(crate) fn metadata_of(path: &Path, directory: bool) -> Result<Option<fs::Metadata>, String> {
    metadata_cause(path, directory).map_err(|e| format!("{}: {e}", diagnostic_path(path).display()))
}

fn validate_registry(billet: &Path) -> Result<(), String> {
    if metadata_of(&billet.join("memory.md"), false)?.is_none() {
        return Err(format!(
            "partial .billet registry at {}; run `blt init` to repair it",
            diagnostic_path(billet).display()
        ));
    }
    for place in [Place::Open, Place::Closed] {
        metadata_of(&billet.join(place.dir()), true)?;
    }
    Ok(())
}

fn registry_at(dir: &Path) -> Result<Option<PathBuf>, String> {
    let billet = dir.join(".billet");
    if metadata_of(&billet, true)?.is_none() {
        return Ok(None);
    }
    validate_registry(&billet)?;
    Ok(Some(billet))
}

fn ancestors(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(dir.to_path_buf());
    while let Some(d) = cur {
        out.push(d.clone());
        cur = d.parent().map(|p| p.to_path_buf());
    }
    out
}

pub(crate) fn discover_raw(cwd: &Path) -> Result<PathBuf, String> {
    for dir in ancestors(cwd) {
        let billet = dir.join(".billet");
        if metadata_of(&billet, true)?.is_some() {
            return Ok(billet);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    Err("no .billet registry found; run `blt init` in the project root".to_string())
}

pub fn discover(cwd: &Path) -> Result<PathBuf, String> {
    let billet = discover_raw(cwd)?;
    validate_registry(&billet)?;
    Ok(billet)
}

fn validate_children(billet: &Path) -> Result<(), String> {
    for place in [Place::Open, Place::Closed] {
        metadata_of(&billet.join(place.dir()), true)?;
    }
    metadata_of(&billet.join(".gitignore"), false)?;
    metadata_of(&billet.join("memory.md"), false)?;
    metadata_of(&billet.join(crate::config::FILE), false)?;
    Ok(())
}

pub fn init(cwd: &Path, prefix: Option<&str>) -> Result<bool, String> {
    if let Some(prefix) = prefix
        && !crate::issue::valid_prefix(prefix)
    {
        return Err(format!(
            "invalid prefix {prefix:?}; {}",
            crate::config::PREFIX_RULE
        ));
    }
    let billet = cwd.join(".billet");
    if metadata_of(&billet, true)?.is_some() {
        validate_children(&billet)?;
    } else {
        if !cwd.join(".git").exists() {
            for dir in ancestors(cwd).into_iter().skip(1) {
                if registry_at(&dir)?.is_some() {
                    return Err(format!(
                        "refusing to nest .billet under existing registry at {}",
                        diagnostic_path(&dir.join(".billet")).display()
                    ));
                }
                if dir.join(".git").exists() {
                    break;
                }
            }
        }
        match fs::create_dir(&billet) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                validate_children(&billet)?;
            }
            Err(e) => {
                return Err(format!(
                    "create {}: {e}",
                    diagnostic_path(&billet).display()
                ));
            }
        }
    }
    let _file = exclusive_lock(&billet)?;
    validate_children(&billet)?;
    let mut created = false;
    for place in [Place::Open, Place::Closed] {
        let dir = billet.join(place.dir());
        match fs::create_dir(&dir) {
            Ok(()) => created = true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(format!("create {}: {e}", diagnostic_path(&dir).display()));
            }
        }
    }
    if metadata_of(&billet.join(".gitignore"), false)?.is_none() {
        atomic_write(&billet.join(".gitignore"), GITIGNORE.as_bytes())?;
        created = true;
    }
    if metadata_of(&billet.join("memory.md"), false)?.is_none() {
        atomic_write(&billet.join("memory.md"), MEMORY_INIT.as_bytes())?;
        created = true;
    }
    let config_path = billet.join(crate::config::FILE);
    if metadata_of(&config_path, false)?.is_none() {
        let prefix = prefix.unwrap_or(crate::config::DEFAULT_PREFIX);
        atomic_write(&config_path, crate::config::render(prefix).as_bytes())?;
        created = true;
    } else if let Some(prefix) = prefix {
        let current = crate::config::load(&billet)?;
        if current.prefix != prefix {
            return Err(format!(
                "this registry uses prefix {:?}; edit .billet/config.json to change it",
                current.prefix
            ));
        }
    }
    Ok(created)
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("sync {}: {e}", diagnostic_path(dir).display()))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("no parent directory")?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("bad file name")?;
    let permissions = metadata_of(path, false)?.map(|m| m.permissions().mode() & 0o777);
    let private = name == ".move";
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    if permissions.is_some() || private {
        options.mode(0o600);
    }
    for attempt in 0..16u32 {
        let tmp = dir.join(format!(".{name}.tmp-{}-{attempt}", std::process::id()));
        let mut f = match options.open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "create temp for {}: {e}",
                    diagnostic_path(path).display()
                ));
            }
        };
        if let Err(e) = f
            .write_all(bytes)
            .and_then(|()| match permissions {
                Some(mode) if !private => f.set_permissions(fs::Permissions::from_mode(mode)),
                _ => Ok(()),
            })
            .and_then(|()| f.sync_all())
        {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(format!("write {}: {e}", diagnostic_path(path).display()));
        }
        if let Err(e) = fs::rename(&tmp, path) {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(format!("rename {}: {e}", diagnostic_path(path).display()));
        }
        return sync_dir(dir);
    }
    Err(format!(
        "could not create temp file for {}",
        diagnostic_path(path).display()
    ))
}

pub struct Lock {
    pub billet: PathBuf,
    _file: fs::File,
}

fn exclusive_lock(billet: &Path) -> Result<fs::File, String> {
    let path = billet.join(".lock");
    metadata_of(&path, false)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("open lock: {e}"))?;
    file.lock().map_err(|e| format!("acquire lock: {e}"))?;
    Ok(file)
}

pub fn lock(billet: &Path) -> Result<Lock, String> {
    let file = exclusive_lock(billet)?;
    recover(billet)?;
    Ok(Lock {
        billet: billet.to_path_buf(),
        _file: file,
    })
}

fn optional_read_lock(billet: &Path) -> Result<Option<fs::File>, String> {
    let path = billet.join(".lock");
    if metadata_of(&path, false)?.is_none() {
        return Ok(None);
    }
    match fs::File::open(&path) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("open lock: {e}")),
    }
}

fn move_present(billet: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(billet.join(".move")) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "inspect {}: {e}",
            diagnostic_path(&billet.join(".move")).display()
        )),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Recovery {
    Allow,
    ReportOnly,
}

pub(crate) fn with_read<T>(
    billet: &Path,
    recovery: Recovery,
    mut read: impl FnMut() -> T,
) -> Result<T, String> {
    loop {
        match optional_read_lock(billet)? {
            Some(file) => {
                file.lock_shared()
                    .map_err(|e| format!("acquire shared lock: {e}"))?;
                if matches!(recovery, Recovery::Allow) && move_present(billet)? {
                    file.unlock()
                        .map_err(|e| format!("release shared lock: {e}"))?;
                    file.lock()
                        .map_err(|e| format!("acquire lock for pending move: {e}"))?;
                    recover(billet).map_err(|e| {
                        format!(
                            "pending .billet/.move recovery failed: {e}; finishing the journal needs a writable registry — inspect it with `blt check`"
                        )
                    })?;
                }
                return Ok(read());
            }
            None => {
                let pending = move_present(billet);
                let result = match pending {
                    Err(e) => Err(e),
                    Ok(true) if matches!(recovery, Recovery::Allow) => Err(
                        ".billet/.move is pending but no lock exists; the registry needs a writable \
                         checkout to finish it — inspect with `blt check` and run a write command \
                         where .billet is writable"
                            .to_string(),
                    ),
                    _ => Ok(read()),
                };
                if metadata_of(&billet.join(".lock"), false)?.is_some() {
                    continue;
                }
                return result;
            }
        }
    }
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if metadata_cause(path, false)?.is_none() {
        return Ok(None);
    }
    fs::read(path)
        .map(Some)
        .map_err(|e| format!("cannot read: {e}"))
}

fn parse_journal(raw: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let text = std::str::from_utf8(raw).map_err(|_| "corrupt move journal".to_string())?;
    let rest = text
        .strip_prefix(MOVE_MAGIC)
        .and_then(|r| r.strip_prefix('\n'))
        .ok_or("corrupt move journal magic")?;
    let (old_len, rest) = rest.split_once('\n').ok_or("corrupt move journal")?;
    let (new_len, rest) = rest.split_once('\n').ok_or("corrupt move journal")?;
    let old_len: usize = old_len.parse().map_err(|_| "corrupt move journal")?;
    let new_len: usize = new_len.parse().map_err(|_| "corrupt move journal")?;
    let bytes = rest.as_bytes();
    if old_len.checked_add(new_len) != Some(bytes.len()) {
        return Err("corrupt move journal lengths".to_string());
    }
    Ok((bytes[..old_len].to_vec(), bytes[old_len..].to_vec()))
}

struct MovePlan {
    old: Vec<u8>,
    new: Vec<u8>,
    id: String,
    src: PathBuf,
    dst: PathBuf,
}

fn pending_move(billet: &Path) -> Result<Option<MovePlan>, String> {
    let mp = billet.join(".move");
    let Some(raw) = read_opt(&mp)? else {
        return Ok(None);
    };
    let (old, new) = parse_journal(&raw)?;
    let old_text = String::from_utf8(old.clone()).map_err(|_| "corrupt move journal")?;
    let new_text = String::from_utf8(new.clone()).map_err(|_| "corrupt move journal")?;
    let old_issue = issue::decode(&old_text).map_err(|e| format!("move journal: {e}"))?;
    let new_issue = issue::decode(&new_text).map_err(|e| format!("move journal: {e}"))?;
    if old_issue.id != new_issue.id {
        return Err(format!(
            "move journal id mismatch {} vs {}",
            old_issue.id, new_issue.id
        ));
    }
    if old_issue.status.active() == new_issue.status.active() {
        return Err("move journal without status crossing".to_string());
    }
    for place in [Place::Open, Place::Closed] {
        metadata_of(&billet.join(place.dir()), true)?;
    }
    let id = old_issue.id;
    let src = billet
        .join(Place::of(old_issue.status).dir())
        .join(format!("{id}.md"));
    let dst = billet
        .join(Place::of(new_issue.status).dir())
        .join(format!("{id}.md"));
    Ok(Some(MovePlan {
        old,
        new,
        id,
        src,
        dst,
    }))
}

enum MoveStep {
    Rename,
    Rewrite,
    Remove,
}

fn move_step(plan: &MovePlan) -> Result<MoveStep, String> {
    let src_b = read_opt(&plan.src)
        .map_err(|e| format!("{}: {e}", diagnostic_path(&plan.src).display()))?;
    let dst_b = read_opt(&plan.dst)
        .map_err(|e| format!("{}: {e}", diagnostic_path(&plan.dst).display()))?;
    match (src_b.as_deref(), dst_b.as_deref()) {
        (Some(s), None) if s == plan.old.as_slice() => Ok(MoveStep::Rename),
        (None, Some(d)) if d == plan.old.as_slice() => Ok(MoveStep::Rewrite),
        (None, Some(d)) if d == plan.new.as_slice() => Ok(MoveStep::Remove),
        _ => Err(format!("move journal for {} does not match files", plan.id)),
    }
}

fn recover(billet: &Path) -> Result<(), String> {
    let Some(plan) = pending_move(billet)? else {
        return Ok(());
    };
    let mp = billet.join(".move");
    let src_dir = plan.src.parent().unwrap().to_path_buf();
    let dst_dir = plan.dst.parent().unwrap().to_path_buf();
    loop {
        match move_step(&plan).map_err(|e| format!("{e}; inspect with `blt check`"))? {
            MoveStep::Rename => {
                fs::rename(&plan.src, &plan.dst)
                    .map_err(|e| format!("recover move {}: {e}", plan.id))?;
                sync_dir(&src_dir)?;
                sync_dir(&dst_dir)?;
            }
            MoveStep::Rewrite => {
                atomic_write(&plan.dst, &plan.new)?;
            }
            MoveStep::Remove => {
                fs::remove_file(&mp).map_err(|e| format!("remove move journal: {e}"))?;
                sync_dir(billet)?;
                return Ok(());
            }
        }
    }
}

pub(crate) fn inspect_move(billet: &Path) -> Result<Option<String>, String> {
    match pending_move(billet)? {
        None => Ok(None),
        Some(plan) => {
            move_step(&plan)?;
            Ok(Some(plan.id))
        }
    }
}

pub(crate) fn read_issue_file(path: &Path) -> Result<(Issue, String), String> {
    if metadata_cause(path, false)?.is_none() {
        return Err("file not found".to_string());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read issue: {e}"))?;
    let issue = issue::decode(&text)?;
    Ok((issue, text))
}

fn load_file(place: Place, path: PathBuf) -> Result<Loaded, String> {
    let rel = diagnostic_path(&path).display().to_string();
    let (issue, text) = read_issue_file(&path).map_err(|e| format!("{rel}: {e}"))?;
    let want_name = format!("{}.md", issue.id);
    if path.file_name().and_then(|n| n.to_str()) != Some(want_name.as_str()) {
        return Err(format!(
            "{rel}: file name does not match issue key {}",
            issue.id
        ));
    }
    if Place::of(issue.status) != place {
        return Err(format!(
            "{rel}: status {} does not belong in {}/",
            issue.status.as_str(),
            place.dir()
        ));
    }
    Ok(Loaded {
        issue,
        place,
        path,
        text,
    })
}

pub fn load_place(billet: &Path, place: Place) -> Result<Vec<Loaded>, String> {
    let dir = billet.join(place.dir());
    let mut out = Vec::new();
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(format!("read {}: {e}", diagnostic_path(&dir).display()));
        }
    };
    for ent in rd {
        let ent = ent.map_err(|e| e.to_string())?;
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || !name.ends_with(".md") {
            continue;
        }
        out.push(load_file(place, ent.path())?);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn load_all(billet: &Path) -> Result<Vec<Loaded>, String> {
    let mut all = load_place(billet, Place::Open)?;
    all.extend(load_place(billet, Place::Closed)?);
    let mut seen = std::collections::HashSet::new();
    for l in &all {
        if !seen.insert(l.issue.id.as_str()) {
            return Err(format!("duplicate issue key {}", l.issue.id));
        }
    }
    Ok(all)
}

pub fn find(billet: &Path, id: &str) -> Result<Option<Loaded>, String> {
    let mut found: Option<Loaded> = None;
    for place in [Place::Open, Place::Closed] {
        let path = billet.join(place.dir()).join(format!("{id}.md"));
        if metadata_of(&path, false)?.is_none() {
            continue;
        }
        let l = load_file(place, path)?;
        if l.issue.id != id {
            continue;
        }
        if found.is_some() {
            return Err(format!("duplicate issue key {id}"));
        }
        found = Some(l);
    }
    Ok(found)
}

pub fn exists(billet: &Path, id: &str) -> bool {
    [Place::Open, Place::Closed]
        .iter()
        .any(|p| fs::symlink_metadata(billet.join(p.dir()).join(format!("{id}.md"))).is_ok())
}

fn ensure_consistent(loaded: &Loaded) -> Result<(), String> {
    if let Some((problem, _)) = domain::field_problems(&loaded.issue).first() {
        return Err(format!(
            "{}: {problem}; run `blt check`",
            diagnostic_path(&loaded.path).display()
        ));
    }
    Ok(())
}

pub fn write_issue(loaded: &Loaded, issue: &Issue) -> Result<(), String> {
    ensure_consistent(loaded)?;
    let text = issue::encode(issue)?;
    atomic_write(&loaded.path, text.as_bytes())
}

pub fn write_new(billet: &Path, issue: &Issue) -> Result<PathBuf, String> {
    let place = Place::of(issue.status);
    let dir = billet.join(place.dir());
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.md", issue.id));
    if metadata_of(&path, false)?.is_some() {
        return Err(format!(
            "issue file {} already exists",
            diagnostic_path(&path).display()
        ));
    }
    let text = issue::encode(issue)?;
    atomic_write(&path, text.as_bytes())?;
    Ok(path)
}

pub fn move_issue(billet: &Path, loaded: &Loaded, new_issue: &Issue) -> Result<(), String> {
    ensure_consistent(loaded)?;
    let dst_dir = billet.join(loaded.place.other().dir());
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let dst = dst_dir.join(format!("{}.md", loaded.issue.id));
    if metadata_of(&dst, false)?.is_some() {
        return Err(format!(
            "destination {} already exists",
            diagnostic_path(&dst).display()
        ));
    }
    let new_text = issue::encode(new_issue)?;
    let mut journal = String::from(MOVE_MAGIC);
    journal.push('\n');
    journal.push_str(&loaded.text.len().to_string());
    journal.push('\n');
    journal.push_str(&new_text.len().to_string());
    journal.push('\n');
    journal.push_str(&loaded.text);
    journal.push_str(&new_text);
    atomic_write(&billet.join(".move"), journal.as_bytes())?;
    recover(billet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join("billet-tests");
        fs::create_dir_all(&base).unwrap();
        let p = base.join(format!(
            "{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&p).unwrap();
        p
    }

    #[test]
    fn read_rechecks_lock_created_mid_read() {
        let root = scratch("readrecheck");
        let billet = root.join(".billet");
        fs::create_dir(&billet).unwrap();
        fs::create_dir(billet.join("open")).unwrap();
        let calls = AtomicUsize::new(0);
        let observed = with_read(&billet, Recovery::Allow, || {
            let n = fs::read_dir(&billet).unwrap().count();
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                drop(exclusive_lock(&billet).unwrap());
            }
            n
        })
        .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "read must retry under the new lock"
        );
        assert_eq!(observed, 2, "second attempt sees open/ plus .lock");

        let calls = AtomicUsize::new(0);
        fs::remove_file(billet.join(".lock")).unwrap();
        let observed: Result<usize, String> = with_read(&billet, Recovery::Allow, || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                drop(exclusive_lock(&billet).unwrap());
                return Err("first attempt failed".to_string());
            }
            Ok(fs::read_dir(&billet).unwrap().count())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(observed.unwrap(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_retries_and_recovers_move_staged_mid_read() {
        let root = scratch("readmove");
        let billet = root.join(".billet");
        fs::create_dir(&billet).unwrap();
        fs::create_dir(billet.join("open")).unwrap();
        fs::create_dir(billet.join("closed")).unwrap();
        let old = "---\nid: \"1aaaaa\"\nnumber: 1\ntitle: \"T\"\nstatus: open\npriority: 2\nparent: null\n\
            depends_on: []\nassignee: null\ndefer_until: null\ncreated_at: null\n\
            updated_at: null\nclosed_at: null\nrelease: null\nclaim_id: null\n---\n\n## History\n\n## Notes\n\n";
        let new = old.replace("status: open", "status: closed");
        fs::write(billet.join("open/1aaaaa.md"), old).unwrap();

        let calls = AtomicUsize::new(0);
        let observed: Result<crate::issue::Status, String> =
            with_read(&billet, Recovery::Allow, || {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    let guard = exclusive_lock(&billet).unwrap();
                    let mut journal = String::from(MOVE_MAGIC);
                    journal.push('\n');
                    journal.push_str(&old.len().to_string());
                    journal.push('\n');
                    journal.push_str(&new.len().to_string());
                    journal.push('\n');
                    journal.push_str(old);
                    journal.push_str(&new);
                    fs::write(billet.join(".move"), journal).unwrap();
                    fs::rename(
                        billet.join("open/1aaaaa.md"),
                        billet.join("closed/1aaaaa.md"),
                    )
                    .unwrap();
                    drop(guard);
                }
                let loaded = find(&billet, "1aaaaa")?
                    .ok_or_else(|| "issue vanished mid-move".to_string())?;
                if Place::of(loaded.issue.status) != loaded.place {
                    return Err(format!(
                        "mismatched in-between state: status {} in {}/",
                        loaded.issue.status.as_str(),
                        loaded.place.dir()
                    ));
                }
                Ok(loaded.issue.status)
            })
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(observed.unwrap(), crate::issue::Status::Closed);
        assert!(
            !billet.join(".move").exists(),
            "recovery removed the journal"
        );
        assert!(billet.join("closed/1aaaaa.md").is_file());
        assert!(!billet.join("open/1aaaaa.md").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
