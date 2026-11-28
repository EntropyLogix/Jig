use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

fn urandom(buf: &mut [u8]) -> Result<(), String> {
    let mut f = fs::File::open("/dev/urandom").map_err(|e| e.to_string())?;
    f.read_exact(buf).map_err(|e| e.to_string())
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for &c in b {
        s.push_str(&format!("{c:02x}"));
    }
    s
}

fn salt_path(state: &Path) -> PathBuf {
    state.join("jig").join("install-id")
}

fn parse_salt(text: &str) -> Result<[u8; 16], String> {
    let t = text.trim();
    if t.len() != 32 || !t.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid install-id contents".to_string());
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn install_salt(state: &Path) -> Result<[u8; 16], String> {
    let path = salt_path(state);
    if let Ok(text) = fs::read_to_string(&path) {
        return parse_salt(&text);
    }
    let dir = path.parent().unwrap();
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut fresh = [0u8; 16];
    urandom(&mut fresh)?;
    let tmp = dir.join(format!(".install-id.tmp-{}", std::process::id()));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(hex(&fresh).as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| e.to_string())?;
    }
    match fs::hard_link(&tmp, &path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e.to_string());
        }
    }
    let _ = fs::remove_file(&tmp);
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    match fs::read_to_string(&path) {
        Ok(text) => parse_salt(&text),
        Err(e) => Err(e.to_string()),
    }
}

pub fn new_claim_id() -> Result<String, String> {
    let mut b = [0u8; 16];
    urandom(&mut b)?;
    Ok(hex(&b))
}

pub fn new_issue_id(salt: &[u8; 16], taken: impl Fn(&str) -> bool) -> Result<String, String> {
    for _ in 0..256 {
        let mut rand = [0u8; 16];
        urandom(&mut rand)?;
        let mut h = DefaultHasher::new();
        h.write(salt);
        h.write(&rand);
        let key = format!("{:06x}", h.finish() & 0x00ff_ffff);
        if crate::issue::valid_root_key(&key) && !taken(&key) {
            return Ok(key);
        }
    }
    Err("could not allocate a fresh issue key".to_string())
}
