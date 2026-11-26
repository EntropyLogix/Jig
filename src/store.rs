use crate::issue::{self, Issue, Status};
use std::fs;
use std::io::{self, Read, Write};
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

pub(crate) fn metadata_of(path: &Path, directory: bool) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if (if directory { m.is_dir() } else { m.is_file() }) => Ok(Some(m)),
        Ok(_) => Err(format!(
            "{} must be a real {} (not a symbolic link)",
            path.display(),
            if directory {
                "directory"
            } else {
                "regular file"
            }
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}

fn registry_at(dir: &Path) -> Result<Option<PathBuf>, String> {
    let jig = dir.join(".jig");
    if metadata_of(&jig, true)?.is_none() {
        return Ok(None);
    }
    if metadata_of(&jig.join("memory.md"), false)?.is_none() {
        return Err(format!("partial .jig registry at {}", jig.display()));
    }
    for place in [Place::Open, Place::Closed] {
        metadata_of(&jig.join(place.dir()), true)?;
    }
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

pub fn discover(cwd: &Path) -> Result<PathBuf, String> {
    for dir in ancestors(cwd) {
        if let Some(jig) = registry_at(&dir)? {
            return Ok(jig);
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    Err("no .jig registry found; run `jig init` in the project root".to_string())
}

pub fn init(cwd: &Path) -> Result<bool, String> {
    if let Some(_jig) = registry_at(cwd)? {
        return Ok(false);
    }
    if !cwd.join(".git").exists() {
        for dir in ancestors(cwd).into_iter().skip(1) {
            if registry_at(&dir)?.is_some() {
                return Err(format!(
                    "refusing to nest .jig under existing registry at {}",
                    dir.join(".jig").display()
                ));
            }
            if dir.join(".git").exists() {
                break;
            }
        }
    }
    let jig = cwd.join(".jig");
    match fs::create_dir(&jig) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return registry_at(cwd).and_then(|r| {
                r.map(|_| false)
                    .ok_or_else(|| "registry changed during initialization".to_string())
            });
        }
        Err(e) => return Err(format!("create {}: {e}", jig.display())),
    }
    fs::create_dir(jig.join(Place::Open.dir())).map_err(|e| e.to_string())?;
    fs::create_dir(jig.join(Place::Closed.dir())).map_err(|e| e.to_string())?;
    atomic_write(&jig.join(".gitignore"), GITIGNORE.as_bytes())?;
    atomic_write(&jig.join("memory.md"), MEMORY_INIT.as_bytes())?;
    Ok(true)
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("sync {}: {e}", dir.display()))
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
            Err(e) => return Err(format!("create temp for {}: {e}", path.display())),
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
            return Err(format!("write {}: {e}", path.display()));
        }
        if let Err(e) = fs::rename(&tmp, path) {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(format!("rename {}: {e}", path.display()));
        }
        return sync_dir(dir);
    }
    Err(format!("could not create temp file for {}", path.display()))
}

pub struct Lock {
    pub jig: PathBuf,
    _file: fs::File,
}

pub fn lock(jig: &Path) -> Result<Lock, String> {
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
    recover(jig)?;
    Ok(Lock {
        jig: jig.to_path_buf(),
        _file: file,
    })
}

fn read_opt(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if metadata_of(path, false)?.is_none() {
        return Ok(None);
    }
    fs::read(path)
        .map(Some)
        .map_err(|e| format!("read {}: {e}", path.display()))
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

fn recover(jig: &Path) -> Result<(), String> {
    let mp = jig.join(".move");
    let Some(raw) = read_opt(&mp)? else {
        return Ok(());
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
    let id = &old_issue.id;
    let src_dir = jig.join(Place::of(old_issue.status).dir());
    let dst_dir = jig.join(Place::of(new_issue.status).dir());
    let src = src_dir.join(format!("{id}.md"));
    let dst = dst_dir.join(format!("{id}.md"));
    loop {
        let src_b = read_opt(&src)?;
        let dst_b = read_opt(&dst)?;
        match (src_b.as_deref(), dst_b.as_deref()) {
            (Some(s), None) if s == old.as_slice() => {
                fs::rename(&src, &dst).map_err(|e| format!("recover move {id}: {e}"))?;
                sync_dir(&src_dir)?;
                sync_dir(&dst_dir)?;
            }
            (None, Some(d)) if d == old.as_slice() => {
                atomic_write(&dst, &new)?;
            }
            (None, Some(d)) if d == new.as_slice() => {
                fs::remove_file(&mp).map_err(|e| format!("remove move journal: {e}"))?;
                sync_dir(jig)?;
                return Ok(());
            }
            _ => {
                return Err(format!(
                    "move journal for {id} does not match files; resolve .jig/.move manually"
                ));
            }
        }
    }
}

fn load_file(place: Place, path: PathBuf) -> Result<Loaded, String> {
    if metadata_of(&path, false)?.is_none() {
        return Err(format!("{} not found", path.display()));
    }
    let mut text = String::new();
    fs::File::open(&path)
        .and_then(|mut f| f.read_to_string(&mut text))
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let issue = issue::decode(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let want_name = format!("{}.md", issue.id);
    if path.file_name().and_then(|n| n.to_str()) != Some(want_name.as_str()) {
        return Err(format!(
            "{}: file name does not match issue id {}",
            path.display(),
            issue.id
        ));
    }
    if Place::of(issue.status) != place {
        return Err(format!(
            "{}: status {} does not belong in {}/",
            path.display(),
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
        Err(e) => return Err(format!("read {}: {e}", dir.display())),
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

pub fn write_issue(loaded: &Loaded, issue: &Issue) -> Result<(), String> {
    let text = issue::encode(issue)?;
    atomic_write(&loaded.path, text.as_bytes())
}

pub fn write_new(jig: &Path, issue: &Issue) -> Result<PathBuf, String> {
    let place = Place::of(issue.status);
    let dir = jig.join(place.dir());
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.md", issue.id));
    if metadata_of(&path, false)?.is_some() {
        return Err(format!("issue file {} already exists", path.display()));
    }
    let text = issue::encode(issue)?;
    atomic_write(&path, text.as_bytes())?;
    Ok(path)
}

pub fn move_issue(jig: &Path, loaded: &Loaded, new_issue: &Issue) -> Result<(), String> {
    let dst_dir = jig.join(loaded.place.other().dir());
    fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
    let dst = dst_dir.join(format!("{}.md", loaded.issue.id));
    if metadata_of(&dst, false)?.is_some() {
        return Err(format!("destination {} already exists", dst.display()));
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
