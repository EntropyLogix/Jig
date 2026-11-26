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

const MOVE_MAGIC: &str = "JIG-MOVE-1";
const GITIGNORE: &str = "/.lock\n/.move\n.*.tmp-*\n";
const MEMORY_INIT: &str = "# Project Memory\n";

pub(crate) fn diagnostic_path(path: &Path) -> &Path {
    path.ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == ".jig"))
        .and_then(|jig| jig.parent())
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

fn validate_registry(jig: &Path) -> Result<(), String> {
    if metadata_of(&jig.join("memory.md"), false)?.is_none() {
        return Err(format!(
            "partial .jig registry at {}; run `jig init` to repair it",
            diagnostic_path(jig).display()
        ));
    }
    for place in [Place::Open, Place::Closed] {
        metadata_of(&jig.join(place.dir()), true)?;
    }
    Ok(())
}

fn registry_at(dir: &Path) -> Result<Option<PathBuf>, String> {
    let jig = dir.join(".jig");
    if metadata_of(&jig, true)?.is_none() {
        return Ok(None);
    }
    validate_registry(&jig)?;
    Ok(Some(jig))
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
        let jig = dir.join(".jig");
        if metadata_of(&jig, true)?.is_some() {
            return Ok(jig);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    Err("no .jig registry found; run `jig init` in the project root".to_string())
}

pub fn discover(cwd: &Path) -> Result<PathBuf, String> {
    let jig = discover_raw(cwd)?;
    validate_registry(&jig)?;
    Ok(jig)
}

fn validate_children(jig: &Path) -> Result<(), String> {
    for place in [Place::Open, Place::Closed] {
        metadata_of(&jig.join(place.dir()), true)?;
    }
    metadata_of(&jig.join(".gitignore"), false)?;
    metadata_of(&jig.join("memory.md"), false)?;
    Ok(())
}

pub fn init(cwd: &Path) -> Result<bool, String> {
    let jig = cwd.join(".jig");
    if metadata_of(&jig, true)?.is_some() {
        validate_children(&jig)?;
    } else {
        if !cwd.join(".git").exists() {
            for dir in ancestors(cwd).into_iter().skip(1) {
                if registry_at(&dir)?.is_some() {
                    return Err(format!(
                        "refusing to nest .jig under existing registry at {}",
                        diagnostic_path(&dir.join(".jig")).display()
                    ));
                }
                if dir.join(".git").exists() {
                    break;
                }
            }
        }
        match fs::create_dir(&jig) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                validate_children(&jig)?;
            }
            Err(e) => {
                return Err(format!("create {}: {e}", diagnostic_path(&jig).display()));
            }
        }
    }
    let _file = exclusive_lock(&jig)?;
    validate_children(&jig)?;
    let mut created = false;
    for place in [Place::Open, Place::Closed] {
        let dir = jig.join(place.dir());
        match fs::create_dir(&dir) {
            Ok(()) => created = true,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(format!("create {}: {e}", diagnostic_path(&dir).display()));
            }
        }
    }
    if metadata_of(&jig.join(".gitignore"), false)?.is_none() {
        atomic_write(&jig.join(".gitignore"), GITIGNORE.as_bytes())?;
        created = true;
    }
    if metadata_of(&jig.join("memory.md"), false)?.is_none() {
        atomic_write(&jig.join("memory.md"), MEMORY_INIT.as_bytes())?;
        created = true;
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
    pub jig: PathBuf,
    _file: fs::File,
}

fn exclusive_lock(jig: &Path) -> Result<fs::File, String> {
    let path = jig.join(".lock");
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

pub fn lock(jig: &Path) -> Result<Lock, String> {
    let file = exclusive_lock(jig)?;
    recover(jig)?;
    Ok(Lock {
        jig: jig.to_path_buf(),
        _file: file,
    })
}

fn optional_read_lock(jig: &Path) -> Result<Option<fs::File>, String> {
    let path = jig.join(".lock");
    if metadata_of(&path, false)?.is_none() {
        return Ok(None);
    }
    match fs::File::open(&path) {
        Ok(f) => Ok(Some(f)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("open lock: {e}")),
    }
}

fn move_present(jig: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(jig.join(".move")) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!(
            "inspect {}: {e}",
            diagnostic_path(&jig.join(".move")).display()
        )),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Recovery {
    Allow,
    ReportOnly,
}

pub(crate) fn with_read<T>(
    jig: &Path,
    recovery: Recovery,
    mut read: impl FnMut() -> T,
) -> Result<T, String> {
    loop {
        match optional_read_lock(jig)? {
            Some(file) => {
                file.lock_shared()
                    .map_err(|e| format!("acquire shared lock: {e}"))?;
                if matches!(recovery, Recovery::Allow) && move_present(jig)? {
                    file.unlock()
                        .map_err(|e| format!("release shared lock: {e}"))?;
                    file.lock()
                        .map_err(|e| format!("acquire lock for pending move: {e}"))?;
                    recover(jig).map_err(|e| {
                        format!(
                            "pending .jig/.move recovery failed: {e}; finishing the journal needs a writable registry — inspect it with `jig check`"
                        )
                    })?;
                }
                return Ok(read());
            }
            None => {
                let pending = move_present(jig);
                let result = match pending {
                    Err(e) => Err(e),
                    Ok(true) if matches!(recovery, Recovery::Allow) => Err(
                        ".jig/.move is pending but no lock exists; the registry needs a writable \
                         checkout to finish it — inspect with `jig check` and run a write command \
                         where .jig is writable"
                            .to_string(),
                    ),
                    _ => Ok(read()),
                };
                if metadata_of(&jig.join(".lock"), false)?.is_some() {
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

fn pending_move(jig: &Path) -> Result<Option<MovePlan>, String> {
    let mp = jig.join(".move");
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
        metadata_of(&jig.join(place.dir()), true)?;
    }
    let id = old_issue.id;
    let src = jig
        .join(Place::of(old_issue.status).dir())
        .join(format!("{id}.md"));
    let dst = jig
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

fn recover(jig: &Path) -> Result<(), String> {
    let Some(plan) = pending_move(jig)? else {
        return Ok(());
    };
    let mp = jig.join(".move");
    let src_dir = plan.src.parent().unwrap().to_path_buf();
    let dst_dir = plan.dst.parent().unwrap().to_path_buf();
    loop {
        match move_step(&plan).map_err(|e| format!("{e}; inspect with `jig check`"))? {
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
                sync_dir(jig)?;
                return Ok(());
            }
        }
    }
}

pub(crate) fn inspect_move(jig: &Path) -> Result<Option<String>, String> {
    match pending_move(jig)? {
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
            "{rel}: file name does not match issue id {}",
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

pub fn load_place(jig: &Path, place: Place) -> Result<Vec<Loaded>, String> {
    let dir = jig.join(place.dir());
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

pub fn load_all(jig: &Path) -> Result<Vec<Loaded>, String> {
    let mut all = load_place(jig, Place::Open)?;
    all.extend(load_place(jig, Place::Closed)?);
    let mut seen = std::collections::HashSet::new();
    for l in &all {
        if !seen.insert(l.issue.id.as_str()) {
            return Err(format!("duplicate issue id {}", l.issue.id));
        }
    }
    Ok(all)
}

pub fn find(jig: &Path, id: &str) -> Result<Option<Loaded>, String> {
    let mut found: Option<Loaded> = None;
    for place in [Place::Open, Place::Closed] {
        let path = jig.join(place.dir()).join(format!("{id}.md"));
        if metadata_of(&path, false)?.is_none() {
            continue;
        }
        let l = load_file(place, path)?;
        if l.issue.id != id {
            continue;
        }
        if found.is_some() {
            return Err(format!("duplicate issue id {id}"));
        }
        found = Some(l);
    }
    Ok(found)
}

pub fn exists(jig: &Path, id: &str) -> bool {
    [Place::Open, Place::Closed]
        .iter()
        .any(|p| fs::symlink_metadata(jig.join(p.dir()).join(format!("{id}.md"))).is_ok())
}

fn ensure_consistent(loaded: &Loaded) -> Result<(), String> {
    if let Some((problem, _)) = domain::field_problems(&loaded.issue).first() {
        return Err(format!(
            "{}: {problem}; run `jig check`",
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

pub fn write_new(jig: &Path, issue: &Issue) -> Result<PathBuf, String> {
    let place = Place::of(issue.status);
    let dir = jig.join(place.dir());
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

pub fn move_issue(jig: &Path, loaded: &Loaded, new_issue: &Issue) -> Result<(), String> {
    ensure_consistent(loaded)?;
    let dst_dir = jig.join(loaded.place.other().dir());
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
    atomic_write(&jig.join(".move"), journal.as_bytes())?;
    recover(jig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp/tests");
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
        let jig = root.join(".jig");
        fs::create_dir(&jig).unwrap();
        fs::create_dir(jig.join("open")).unwrap();
        let calls = AtomicUsize::new(0);
        let observed = with_read(&jig, Recovery::Allow, || {
            let n = fs::read_dir(&jig).unwrap().count();
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                drop(exclusive_lock(&jig).unwrap());
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
        fs::remove_file(jig.join(".lock")).unwrap();
        let observed: Result<usize, String> = with_read(&jig, Recovery::Allow, || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                drop(exclusive_lock(&jig).unwrap());
                return Err("first attempt failed".to_string());
            }
            Ok(fs::read_dir(&jig).unwrap().count())
        })
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(observed.unwrap(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_retries_and_recovers_move_staged_mid_read() {
        let root = scratch("readmove");
        let jig = root.join(".jig");
        fs::create_dir(&jig).unwrap();
        fs::create_dir(jig.join("open")).unwrap();
        fs::create_dir(jig.join("closed")).unwrap();
        let old = "---\nid: \"B-000001\"\ntitle: \"T\"\nstatus: open\npriority: 2\nparent: null\n\
            depends_on: []\nassignee: null\ndefer_until: null\ncreated_at: null\n\
            updated_at: null\nclosed_at: null\nrelease: null\nclaim_id: null\n---\n\n## History\n\n## Notes\n\n";
        let new = old.replace("status: open", "status: closed");
        fs::write(jig.join("open/B-000001.md"), old).unwrap();

        let calls = AtomicUsize::new(0);
        let observed: Result<crate::issue::Status, String> =
            with_read(&jig, Recovery::Allow, || {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    let guard = exclusive_lock(&jig).unwrap();
                    let mut journal = String::from(MOVE_MAGIC);
                    journal.push('\n');
                    journal.push_str(&old.len().to_string());
                    journal.push('\n');
                    journal.push_str(&new.len().to_string());
                    journal.push('\n');
                    journal.push_str(old);
                    journal.push_str(&new);
                    fs::write(jig.join(".move"), journal).unwrap();
                    fs::rename(jig.join("open/B-000001.md"), jig.join("closed/B-000001.md"))
                        .unwrap();
                    drop(guard);
                }
                let loaded =
                    find(&jig, "B-000001")?.ok_or_else(|| "issue vanished mid-move".to_string())?;
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
        assert!(!jig.join(".move").exists(), "recovery removed the journal");
        assert!(jig.join("closed/B-000001.md").is_file());
        assert!(!jig.join("open/B-000001.md").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
