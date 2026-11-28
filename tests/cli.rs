use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

static N: AtomicUsize = AtomicUsize::new(0);

struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn dir(name: &str) -> TestDir {
    let base = std::env::temp_dir().join("billet-tests");
    fs::create_dir_all(&base).unwrap();
    loop {
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p = base.join(format!("{name}-{}-{n}", std::process::id()));
        match fs::create_dir(&p) {
            Ok(()) => return TestDir(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("create {}: {e}", p.display()),
        }
    }
}

fn spawn(dir: &Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_blt"))
        .args(args)
        .current_dir(dir)
        .env("XDG_STATE_HOME", dir.join("xdg"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn run(dir: &Path, args: &[&str]) -> Output {
    spawn(dir, args).wait_with_output().unwrap()
}

fn run_stdin(dir: &Path, args: &[&str], input: &str) -> Output {
    let mut ch = Command::new(env!("CARGO_BIN_EXE_blt"))
        .args(args)
        .current_dir(dir)
        .env("XDG_STATE_HOME", dir.join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ch.stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    ch.wait_with_output().unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert!(
        o.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn bad(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert!(!o.status.success(), "{args:?} unexpectedly succeeded");
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn add(dir: &Path, title: &str) -> String {
    ok(dir, &["add", title, "--approved"]).trim().to_string()
}

fn number_of(key: &str) -> u64 {
    u64::from_str_radix(key.split('.').next().unwrap(), 16).unwrap()
}

fn h(key: &str) -> String {
    let root = key.split('.').next().unwrap();
    format!("B{}{}", number_of(root), &key[root.len()..])
}

fn fm(id: &str, status: &str, extra: &str) -> String {
    let number = if id.contains('.') {
        "null".to_string()
    } else {
        number_of(id).to_string()
    };
    format!(
        "---\nid: \"{id}\"\nnumber: {number}\ntitle: \"T {id}\"\nstatus: {status}\npriority: 2\nparent: null\n\
         depends_on: []\nassignee: null\ndefer_until: null\ncreated_at: null\nupdated_at: null\n\
         closed_at: null\nrelease: null\nclaim_id: null\n{extra}---\n\n## History\n\n## Notes\n\n"
    )
}

fn render_key(d: &Path, token: &str) -> String {
    if !billet::issue::valid_key(token) {
        return token.to_string();
    }
    let root = token.split('.').next().unwrap();
    for place in ["open", "closed"] {
        if let Ok(text) = fs::read_to_string(d.join(format!(".blt/{place}/{root}.md")))
            && let Ok(issue) = billet::issue::decode(&text)
            && let Some(number) = issue.number
        {
            return format!("B{number}{}", &token[root.len()..]);
        }
    }
    token.to_string()
}

fn render_keys(d: &Path, text: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '/' | '-' | '_') {
            token.push(c);
        } else {
            out.push_str(&render_key(d, &token));
            token.clear();
            out.push(c);
        }
    }
    out.push_str(&render_key(d, &token));
    out
}

fn render_json_keys(d: &Path, value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(render_keys(d, text)),
        Value::Array(items) => Value::Array(items.iter().map(|v| render_json_keys(d, v)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), render_json_keys(d, v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn key_of(d: &Path, handle: &str) -> String {
    show_value(d, handle)["key"].as_str().unwrap().to_string()
}

fn journal(old: &str, new: &str) -> String {
    format!("BLT-MOVE-1\n{}\n{}\n{old}{new}", old.len(), new.len())
}

#[test]
fn init_and_nested_discovery() {
    let d = dir("init");
    let proj = d.join("proj");
    fs::create_dir_all(&proj).unwrap();
    assert_eq!(ok(&proj, &["init"]), "initialized .blt/\n");
    assert!(proj.join(".blt/open").is_dir());
    assert!(proj.join(".blt/closed").is_dir());
    assert!(proj.join(".blt/memory.md").is_file());
    let gi = fs::read_to_string(proj.join(".blt/.gitignore")).unwrap();
    assert!(gi.contains("/.lock") && gi.contains("/.move") && gi.contains(".*.tmp-"));
    assert!(
        gi.lines().any(|s| s == ".*.tmp-*"),
        "tmp rule must be unanchored to cover nested files, got {gi:?}"
    );

    let id = add(&proj, "work");
    let deep = proj.join("a/b/c");
    fs::create_dir_all(&deep).unwrap();
    let out = ok(&deep, &["ready"]);
    assert!(out.contains(&id), "ready from nested dir: {out}");

    let e = bad(&deep, &["init"]);
    assert!(e.contains("nest"), "{e}");
    assert_eq!(ok(&proj, &["init"]), ".blt/ already initialized\n");

    let mem = proj.join(".blt/memory.md");
    let before = fs::read_to_string(&mem).unwrap();
    ok(&proj, &["init"]);
    assert_eq!(fs::read_to_string(&mem).unwrap(), before);
}

#[test]
fn git_boundary_stops_discovery() {
    let d = dir("gitb");
    let outer = d.join("outer");
    fs::create_dir_all(&outer).unwrap();
    ok(&outer, &["init"]);
    let repo = outer.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join(".git"), "gitdir: /elsewhere\n").unwrap();
    let e = bad(&repo, &["ready"]);
    assert!(e.contains("no .blt"), "{e}");
    let deep = repo.join("sub");
    fs::create_dir_all(&deep).unwrap();
    let e = bad(&deep, &["ready"]);
    assert!(e.contains("no .blt"), "{e}");
    assert_eq!(ok(&repo, &["init"]), "initialized .blt/\n");
}

#[test]
fn missing_dirs_are_empty_registry() {
    let d = dir("emptydirs");
    let proj = d.join("proj");
    fs::create_dir_all(proj.join(".blt")).unwrap();
    fs::write(proj.join(".blt/memory.md"), "# Project Memory\n").unwrap();
    fs::write(proj.join(".blt/config.json"), "{\"prefix\": \"B\"}\n").unwrap();
    assert_eq!(ok(&proj, &["ready"]), "");
    let id = add(&proj, "x");
    assert_eq!(id, "B1", "numbering starts at 1");
    let key = key_of(&proj, &id);
    assert!(proj.join(format!(".blt/open/{key}.md")).is_file());
}

#[test]
fn concurrent_claim_single_winner() {
    let d = dir("race");
    ok(&d, &["init"]);
    let id = add(&d, "contested");
    let a = spawn(&d, &["claim", &id, "--by", "one"]);
    let b = spawn(&d, &["claim", &id, "--by", "two"]);
    let oa = a.wait_with_output().unwrap();
    let ob = b.wait_with_output().unwrap();
    assert_eq!(
        oa.status.success() as u8 + ob.status.success() as u8,
        1,
        "stdout {:?} stderr {:?}",
        String::from_utf8_lossy(&oa.stdout),
        String::from_utf8_lossy(&ob.stderr)
    );
    let key = key_of(&d, &id);
    let text = fs::read_to_string(d.join(format!(".blt/open/{key}.md"))).unwrap();
    assert!(text.contains("status: claimed"));
    assert!(text.contains("claim_id: \""));
    let i = billet::issue::decode(&text).unwrap();
    let who = i.assignee.clone().unwrap();
    assert!(who == "one" || who == "two");
}

#[test]
fn same_name_copies_get_distinct_claim_ids() {
    let d1 = dir("claimA");
    let d2 = dir("claimB");
    ok(&d1, &["init"]);
    ok(&d2, &["init"]);
    let shared = fm("1aaaaa", "open", "");
    fs::write(d1.join(".blt/open/1aaaaa.md"), &shared).unwrap();
    fs::write(d2.join(".blt/open/1aaaaa.md"), &shared).unwrap();
    let o1 = ok(&d1, &["claim", "1aaaaa", "--by", "bot"]);
    let o2 = ok(&d2, &["claim", "1aaaaa", "--by", "bot"]);
    let t1 = fs::read_to_string(d1.join(".blt/open/1aaaaa.md")).unwrap();
    let t2 = fs::read_to_string(d2.join(".blt/open/1aaaaa.md")).unwrap();
    let cid = |t: &str| billet::issue::decode(t).unwrap().claim_id.unwrap();
    assert_ne!(cid(&t1), cid(&t2));
    assert!(!o1.contains(&cid(&t1)) && !o2.contains(&cid(&t2)));
}

#[test]
fn child_numbering_and_tree() {
    let d = dir("children");
    ok(&d, &["init"]);
    let p = add(&d, "parent");
    let c1 = ok(&d, &["add", "c1", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    let c2 = ok(&d, &["add", "c2", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    assert_eq!(c1, format!("{p}.1"));
    assert_eq!(c2, format!("{p}.2"));
    ok(&d, &["close", &c1, "--reason", "done"]);
    let c3 = ok(&d, &["add", "c3", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    assert_eq!(c3, format!("{p}.3"), "closed child slot must not be reused");
    let g = ok(&d, &["add", "gc", "--parent", &c2, "--approved"])
        .trim()
        .to_string();
    assert_eq!(g, format!("{c2}.1"));
    let t = ok(&d, &["tree", &p]);
    assert!(t.contains(&format!("{p} open parent")));
    assert!(t.contains(&format!("  {c1} closed c1")));
    assert!(t.contains(&format!("    {g} open gc")));
    let j = ok(&d, &["tree", &p, "--json"]);
    assert!(j.contains("\"children\":[") && j.contains(&format!("\"id\":\"{g}\"")));
    let e = bad(&d, &["add", "orphan", "--parent", "1e0000"]);
    assert!(e.contains("not found"), "{e}");
}

#[test]
fn show_documents_directory_and_files() {
    let d = dir("showdocs");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "documented");
    let key = key_of(&d, &id);
    let docs_dir = format!(".blt/docs/{key}/");

    let text = ok(&d, &["show", &id]);
    assert!(text.ends_with(&format!("Documents: {docs_dir} (none)\n")));
    let json: Value = serde_json::from_str(&ok(&d, &["show", &id, "--json"])).unwrap();
    assert_eq!(json["docs_dir"], docs_dir);
    assert_eq!(json["docs"], json!([]));

    fs::create_dir_all(d.join(format!(".blt/docs/{key}/review"))).unwrap();
    fs::write(d.join(format!(".blt/docs/{key}/z.md")), "secret").unwrap();
    fs::write(d.join(format!(".blt/docs/{key}/review/r1.md")), "report").unwrap();
    let text = ok(&d, &["show", &id]);
    assert!(text.ends_with(&format!(
        "Documents: {docs_dir}\n  .blt/docs/{key}/review/r1.md\n  .blt/docs/{key}/z.md\n"
    )));
    let json: Value = serde_json::from_str(&ok(&d, &["show", &id, "--json"])).unwrap();
    assert_eq!(
        json["docs"],
        json!([
            format!(".blt/docs/{key}/review/r1.md"),
            format!(".blt/docs/{key}/z.md")
        ])
    );
    ok(&d, &["close", &id, "--reason", "done"]);
    assert!(d.join(format!(".blt/docs/{key}/z.md")).is_file());
    ok(&d, &["reopen", &id, "--reason", "continue"]);
    assert!(d.join(format!(".blt/docs/{key}/review/r1.md")).is_file());
}

#[test]
fn memory_roundtrip() {
    let d = dir("memory");
    ok(&d, &["init"]);
    ok(&d, &["remember", "alpha", "first"]);
    ok(&d, &["remember", "beta", "line1\nline2"]);
    ok(&d, &["remember", "alpha", "second"]);
    assert_eq!(ok(&d, &["recall", "alpha"]), "second\n");
    assert_eq!(ok(&d, &["recall", "beta"]), "line1\nline2\n");
    assert_eq!(ok(&d, &["recall"]), "alpha\nbeta\n");
    let j = ok(&d, &["recall", "--json"]);
    assert!(j.contains("\"alpha\"") && j.contains("\"beta\""));
    let j = ok(&d, &["recall", "alpha", "--json"]);
    assert_eq!(j, "{\"topic\":\"alpha\",\"value\":\"second\"}\n");
    bad(&d, &["recall", "nope"]);
    bad(&d, &["remember", "bad topic!", "v"]);
}

#[test]
fn stdin_body_does_not_hold_lock() {
    let d = dir("stdlock");
    ok(&d, &["init"]);
    let id = add(&d, "t");
    let mut writer = Command::new(env!("CARGO_BIN_EXE_blt"))
        .args(["update", &id, "--body", "-"])
        .current_dir(&d)
        .env("XDG_STATE_HOME", d.join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let feed = writer.stdin.take().unwrap();
    let reader = spawn(&d, &["ready"]);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(reader.wait_with_output().unwrap());
    });
    let read = receiver.recv_timeout(Duration::from_secs(10));
    let mut feed = feed;
    feed.write_all(b"fresh body").unwrap();
    drop(feed);
    let read = read.expect("ready hung behind an update that is still reading stdin");
    assert!(read.status.success());
    let out = writer.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(show_value(&d, &id)["body"], "fresh body");
}

#[test]
fn add_dep_many_preserves_order() {
    let d = dir("depmany");
    ok(&d, &["init"]);
    let a = add(&d, "a");
    let b = add(&d, "b");
    let c = add(&d, "c");
    let id = ok(
        &d,
        &["add", "t", "--dep", &a, &b, "--dep", &c, "--approved"],
    )
    .trim()
    .to_string();
    let key = key_of(&d, &id);
    let i =
        billet::issue::decode(&fs::read_to_string(d.join(format!(".blt/open/{key}.md"))).unwrap())
            .unwrap();
    assert_eq!(
        i.depends_on,
        vec![key_of(&d, &a), key_of(&d, &b), key_of(&d, &c)]
    );
    let e = bad(&d, &["add", "x", "--dep", &a, &a]);
    assert!(e.contains("duplicate"), "{e}");
    let e = bad(&d, &["add", "x", "--dep", "not-an-id!!"]);
    assert!(e.contains("invalid issue reference"), "{e}");
    let e = bad(&d, &["add", "x", "--dep"]);
    assert!(e.contains("missing value"), "{e}");
}

#[test]
fn symlink_registry_and_dirs_refuse() {
    let d = dir("links");
    let real = d.join("real");
    fs::create_dir_all(&real).unwrap();
    ok(&real, &["init"]);
    add(&real, "target");
    let linked = d.join("linked");
    fs::create_dir(&linked).unwrap();
    std::os::unix::fs::symlink(real.join(".blt"), linked.join(".blt")).unwrap();
    let e = bad(&linked, &["ready"]);
    assert!(e.contains("symbolic link"), "{e}");
    let e = bad(&linked, &["add", "x"]);
    assert!(e.contains("symbolic link"), "{e}");
    let e = bad(&linked, &["init"]);
    assert!(e.contains("symbolic link"), "{e}");
    assert_eq!(
        fs::read_dir(real.join(".blt/open")).unwrap().count(),
        1,
        "linked registry untouched"
    );

    let d2 = dir("linkopen");
    ok(&d2, &["init"]);
    fs::remove_dir(d2.join(".blt/open")).unwrap();
    let victim = d2.join("victim");
    fs::create_dir(&victim).unwrap();
    std::os::unix::fs::symlink(&victim, d2.join(".blt/open")).unwrap();
    let e = bad(&d2, &["ready"]);
    assert!(e.contains("symbolic link"), "{e}");
    let e = bad(&d2, &["add", "x"]);
    assert!(e.contains("symbolic link"), "{e}");
    assert!(
        fs::read_dir(&victim).unwrap().next().is_none(),
        "nothing written through symlinked open/"
    );
}

#[test]
fn permissions_preserved_on_rewrite() {
    use std::os::unix::fs::PermissionsExt;
    let d = dir("perms");
    ok(&d, &["init"]);
    let id = add(&d, "p");
    let key = key_of(&d, &id);
    let open = d.join(format!(".blt/open/{key}.md"));
    let closed = d.join(format!(".blt/closed/{key}.md"));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(&open, fs::Permissions::from_mode(0o600)).unwrap();
    ok(&d, &["update", &id, "--title", "p2"]);
    assert_eq!(mode(&open), 0o600, "update must preserve mode");
    ok(&d, &["close", &id, "--reason", "done", "--summary", "s"]);
    assert_eq!(mode(&closed), 0o600, "close must preserve mode");
    ok(&d, &["reopen", &id, "--reason", "again"]);
    assert_eq!(mode(&open), 0o600, "reopen must preserve mode");
}

#[cfg(target_os = "linux")]
#[test]
fn stdout_write_failure_exits_one() {
    let d = dir("full");
    ok(&d, &["init"]);
    let id = add(&d, "s");
    let devfull = fs::File::options().write(true).open("/dev/full").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_blt"))
        .args(["show", &id, "--json"])
        .current_dir(&d)
        .env("XDG_STATE_HOME", d.join("xdg"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(devfull))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("write stdout"), "{stderr}");
}

#[test]
fn init_installs_agent_integrations_idempotently() {
    let standard =
        json!({"hooks":[{"type":"command","command":"blt start --hook-json","timeout":5}]});

    let d = dir("integrations");
    let proj = d.join("proj");
    fs::create_dir_all(&proj).unwrap();
    let agents_orig = "# Własne instrukcje\n\nZasady projektu bez końcowego wiersza";
    fs::write(proj.join("AGENTS.md"), agents_orig).unwrap();
    fs::create_dir_all(proj.join(".claude")).unwrap();
    let claude_orig = r#"{
  "permissions": {"allow": ["Bash(cargo test)"]},
  "hooks": {
    "SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}],
    "PostToolUse": [{"matcher": "Edit", "hooks": [{"type": "command", "command": "fmt"}]}]
  }
}"#;
    fs::write(proj.join(".claude/settings.json"), claude_orig).unwrap();
    fs::create_dir_all(proj.join(".codex")).unwrap();
    let codex_orig = r#"{"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "codex-hook"}]}]}}"#;
    fs::write(proj.join(".codex/hooks.json"), codex_orig).unwrap();
    let sentinel = "model = \"keepme\"\n";
    fs::write(proj.join(".codex/config.toml"), sentinel).unwrap();

    assert_eq!(ok(&proj, &["init"]), "initialized .blt/\n");
    let agents = fs::read_to_string(proj.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with(agents_orig), "original bytes preserved");
    assert_eq!(agents, format!("{agents_orig}\n\n{AGENTS_LINE}\n"));

    let claude: Value =
        serde_json::from_str(&fs::read_to_string(proj.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        claude["permissions"],
        json!({"allow": ["Bash(cargo test)"]})
    );
    let session = claude["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(session[0]["hooks"][0]["command"], "echo hi");
    assert_eq!(session.iter().filter(|g| **g == standard).count(), 1);
    assert_eq!(
        claude["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
        "fmt"
    );
    let codex: Value =
        serde_json::from_str(&fs::read_to_string(proj.join(".codex/hooks.json")).unwrap()).unwrap();
    let codex_session = codex["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(codex_session[0]["hooks"][0]["command"], "codex-hook");
    assert_eq!(codex_session.iter().filter(|g| **g == standard).count(), 1);
    assert_eq!(
        fs::read_to_string(proj.join(".codex/config.toml")).unwrap(),
        sentinel,
        "config.toml untouched"
    );

    let proj2 = d.join("proj2");
    fs::create_dir_all(&proj2).unwrap();
    fs::write(proj2.join("AGENTS.md"), "# Team rules\n").unwrap();
    ok(&proj2, &["init"]);
    let agents2 = fs::read_to_string(proj2.join("AGENTS.md")).unwrap();
    assert!(agents2.starts_with("# Team rules\n\n"));

    for p in [&proj, &proj2] {
        let files = ["AGENTS.md", ".claude/settings.json", ".codex/hooks.json"];
        let before: Vec<Vec<u8>> = files.iter().map(|f| fs::read(p.join(f)).unwrap()).collect();
        ok(p, &["init"]);
        let after: Vec<Vec<u8>> = files.iter().map(|f| fs::read(p.join(f)).unwrap()).collect();
        assert_eq!(before, after, "repeat init must not rewrite files");
    }
}

#[test]
fn init_no_hooks_and_integration_preflight() {
    let d = dir("preflight");
    let proj = d.join("proj");
    fs::create_dir_all(proj.join(".codex")).unwrap();
    fs::write(proj.join(".codex/hooks.json"), "{not json").unwrap();
    fs::create_dir_all(proj.join(".claude")).unwrap();
    let claude_orig = "{\"hooks\": {}}";
    fs::write(proj.join(".claude/settings.json"), claude_orig).unwrap();
    let e = bad(&proj, &["init"]);
    assert!(e.contains(".codex/hooks.json"), "{e}");
    assert!(
        !proj.join("AGENTS.md").exists(),
        "AGENTS must not be written on preflight failure"
    );
    assert_eq!(
        fs::read_to_string(proj.join(".claude/settings.json")).unwrap(),
        claude_orig
    );

    ok(&proj, &["init", "--no-hooks"]);
    assert!(proj.join("AGENTS.md").exists());
    assert_eq!(
        fs::read_to_string(proj.join(".codex/hooks.json")).unwrap(),
        "{not json",
        "malformed config left untouched"
    );

    let d2 = dir("nohooksfresh");
    ok(&d2, &["init", "--no-hooks"]);
    assert!(d2.join("AGENTS.md").exists());
    assert!(!d2.join(".claude").exists() && !d2.join(".codex").exists());
}

#[test]
fn init_refuses_linked_agent_configuration() {
    let d = dir("linkcfg");
    let proj = d.join("proj");
    let vault = d.join("vault");
    fs::create_dir_all(&proj).unwrap();
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("settings.json"), "{}").unwrap();
    std::os::unix::fs::symlink(&vault, proj.join(".claude")).unwrap();
    let e = bad(&proj, &["init"]);
    assert!(e.contains("symbolic link"), "{e}");
    assert_eq!(
        fs::read_to_string(vault.join("settings.json")).unwrap(),
        "{}",
        "external config bytes unchanged"
    );

    let proj2 = d.join("proj2");
    fs::create_dir_all(&proj2).unwrap();
    let outside = d.join("outside.md");
    fs::write(&outside, "external\n").unwrap();
    std::os::unix::fs::symlink(&outside, proj2.join("AGENTS.md")).unwrap();
    let e = bad(&proj2, &["init"]);
    assert!(e.contains("symbolic link"), "{e}");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "external\n");
}

#[test]
fn session_start_hook_payload_is_static_and_read_only() {
    let d = dir("hookjson");
    ok(&d, &["init"]);
    ok(&d, &["remember", "secret.topic", "classified payload"]);
    let id = add(&d, "issue secret body");
    let nested = d.join("a/b");
    fs::create_dir_all(&nested).unwrap();
    let out = ok(&nested, &["start", "--hook-json"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    let hso = &v["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "SessionStart");
    let workflow: Value = serde_json::from_str(&ok(&d, &["start", "--json"])).unwrap();
    assert_eq!(hso["additionalContext"], workflow["workflow"]);
    assert!(
        !out.contains("classified")
            && !out.contains("secret body")
            && !out.contains(&key_of(&d, &id)),
        "hook payload must carry only the static workflow"
    );

    let journal = "BLT-MOVE-1\n18446744073709551615\n2\nxy";
    fs::write(d.join(".blt/.move"), journal).unwrap();
    let out2 = ok(&nested, &["start", "--hook-json"]);
    assert!(out2.contains("SessionStart"));
    assert_eq!(
        fs::read_to_string(d.join(".blt/.move")).unwrap(),
        journal,
        "hook must not recover or mutate the journal"
    );
    fs::remove_file(d.join(".blt/.move")).unwrap();

    let e = bad(&d, &["start", "--json", "--hook-json"]);
    assert!(e.contains("mutually exclusive"), "{e}");
    let e = bad(&d, &["--json", "start", "--hook-json"]);
    assert!(e.contains("mutually exclusive"), "{e}");
}

#[test]
fn list_sort_by_creation_date() {
    let d = dir("listsort");
    ok(&d, &["init", "--no-hooks"]);
    let seed = |id: &str, place: &str, pri: &str, created: &str| {
        let status = if place == "closed" { "closed" } else { "open" };
        let number: u64 = id[1..].parse().unwrap();
        let key = format!("{number:05}a");
        let text = fm(&key, status, "")
            .replace(
                &format!("number: {}", number_of(&key)),
                &format!("number: {number}"),
            )
            .replace(&format!("T {key}"), &format!("T {id}"))
            .replace("priority: 2", &format!("priority: {pri}"))
            .replace("created_at: null", &format!("created_at: {created}"));
        fs::write(d.join(format!(".blt/{place}/{key}.md")), text).unwrap();
    };
    seed("B100", "open", "3", "\"2026-04-03\"");
    seed("B101", "open", "1", "\"2026-04-12T06:30:00Z\"");
    seed("B102", "open", "4", "\"2026-04-12T06:30:00Z\"");
    seed("B103", "open", "0", "null");
    seed("B104", "closed", "2", "\"2026-03-01\"");

    let ids = |out: &str| -> Vec<String> {
        out.lines()
            .filter_map(|l| {
                let first = l.split_whitespace().next()?;
                (!matches!(first, "REVIEW" | "PROPOSED" | "CLAIMED" | "OPEN" | "CLOSED"))
                    .then(|| first.to_string())
            })
            .collect()
    };
    let def = ok(&d, &["list"]);
    assert_eq!(ids(&def), ["B103", "B101", "B100", "B102"]);
    assert_eq!(def.lines().next().unwrap(), "OPEN (4)");
    assert_eq!(ids(&ok(&d, &["ready"])), ["B103", "B101", "B100", "B102"]);
    assert_eq!(
        ids(&ok(&d, &["list", "--sort", "oldest"])),
        ["B100", "B101", "B102", "B103"]
    );
    assert_eq!(
        ids(&ok(&d, &["list", "--sort", "newest"])),
        ["B101", "B102", "B100", "B103"]
    );

    let oldest = ok(&d, &["list", "--sort", "oldest"]);
    let lines: Vec<&str> = oldest.lines().collect();
    assert_eq!(lines[0], "B100 open 3 - 2026-04-03 T B100");
    assert_eq!(lines[1], "B101 open 1 - 2026-04-12T06:30:00Z T B101");
    assert_eq!(lines[3], "B103 open 0 - - T B103");

    let j: Value = serde_json::from_str(&ok(
        &d,
        &["list", "--status", "all", "--sort", "oldest", "--json"],
    ))
    .unwrap();
    let arr = j.as_array().unwrap();
    let jids: Vec<&str> = arr.iter().map(|v| v["id"].as_str().unwrap()).collect();
    assert_eq!(jids, ["B104", "B100", "B101", "B102", "B103"]);
    assert_eq!(
        arr[0],
        json!({"id":"B104","key":"00104a","title":"T B104","status":"closed","priority":2,"assignee":null})
    );

    let e = bad(&d, &["list", "--sort", "closing"]);
    assert!(e.contains("oldest or newest"), "{e}");
}

#[cfg(unix)]
#[test]
fn broken_pipe_stdout_exits_zero() {
    use std::os::unix::io::OwnedFd;
    use std::os::unix::net::UnixStream;
    let d = dir("bpipe");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "s");
    let (a, b) = UnixStream::pair().unwrap();
    drop(b);
    let fd: OwnedFd = a.into();
    let o = Command::new(env!("CARGO_BIN_EXE_blt"))
        .args(["show", &id, "--json"])
        .current_dir(&d)
        .env("XDG_STATE_HOME", d.join("xdg"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(o.stderr.is_empty(), "stderr must be silent on EPIPE");
}

fn tree_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, rel: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut ents: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let p = rel.join(e.file_name());
            let ft = e.file_type().unwrap();
            if ft.is_dir() {
                walk(&e.path(), &p, out);
            } else if ft.is_file() {
                out.push((p.to_string_lossy().to_string(), fs::read(e.path()).unwrap()));
            } else {
                out.push((p.to_string_lossy().to_string(), b"LINK".to_vec()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, Path::new(""), &mut out);
    out
}

fn seed_bare_registry(d: &Path) -> PathBuf {
    let billet = d.join(".blt");
    fs::create_dir_all(billet.join("open")).unwrap();
    fs::create_dir_all(billet.join("closed")).unwrap();
    fs::write(billet.join("memory.md"), "# Project Memory\n").unwrap();
    fs::write(billet.join(".gitignore"), "/.lock\n/.move\n.*.tmp-*\n").unwrap();
    billet
}

#[test]
fn tmp_ignore_rule_covers_nested_files() {
    let d = dir("tmpignore");
    let billet = seed_bare_registry(&d);
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&d)
        .output()
        .unwrap();
    assert!(init.status.success(), "git init must succeed for this test");
    fs::write(billet.join("open/.10000a.md.tmp-42-1"), "x").unwrap();
    let tmp = ".blt/open/.10000a.md.tmp-42-1";
    let ignored = || {
        Command::new("git")
            .args(["check-ignore", "--no-index", tmp])
            .current_dir(&d)
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(
        ignored(),
        "unanchored `.*.tmp-*` must ignore nested tmp files"
    );
    fs::write(billet.join(".gitignore"), "/.lock\n/.move\n/.*.tmp-*\n").unwrap();
    assert!(
        !ignored(),
        "legacy anchored `/.*.tmp-*` must not ignore nested tmp files"
    );
}

#[cfg(unix)]
#[test]
fn read_only_registry_reads_without_lock() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    let d = dir("readonly");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "ro work");
    ok(&d, &["remember", "topic", "value"]);
    fs::remove_file(d.join(".blt/.lock")).unwrap();
    let xdg = d.join("xdg");
    fs::create_dir_all(&xdg).unwrap();
    fs::set_permissions(&xdg, fs::Permissions::from_mode(0o777)).unwrap();

    let locked: Vec<PathBuf> =
        [d.join(".blt"), d.join(".blt/open"), d.join(".blt/closed")].to_vec();
    struct Restore(Vec<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
            }
        }
    }
    for p in &locked {
        fs::set_permissions(p, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let restore = Restore(locked.clone());

    for args in [
        vec!["start"],
        vec!["ready"],
        vec!["show", &id],
        vec!["list"],
        vec!["tree", &id],
        vec!["recall"],
        vec!["check"],
    ] {
        let o = run(&d, &args);
        assert!(
            o.status.success(),
            "{args:?} failed on read-only registry: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    assert!(
        !d.join(".blt/.lock").exists(),
        "read commands must not create .lock"
    );

    let probe = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(d.join(".blt/.probe"));
    let as_root = probe.is_ok();
    if as_root {
        fs::remove_file(d.join(".blt/.probe")).unwrap();
    }
    let blt_cmd = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_blt"));
        cmd.args(args)
            .current_dir(&d)
            .env("XDG_STATE_HOME", &xdg)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if as_root {
            cmd.uid(65534).gid(65534);
        }
        cmd.output().unwrap()
    };
    let o = blt_cmd(&["add", "must fail"]);
    assert!(
        !o.status.success(),
        "write succeeded in a read-only registry"
    );
    assert!(!o.stderr.is_empty());
    assert_eq!(fs::read_dir(d.join(".blt/open")).unwrap().count(), 1);
    drop(restore);

    let old = fm("10ff03", "open", "");
    let new = fm("10ff03", "closed", "");
    fs::write(d.join(".blt/open/10ff03.md"), &old).unwrap();
    fs::write(d.join(".blt/.move"), journal(&old, &new)).unwrap();
    fs::File::create(d.join(".blt/.lock")).unwrap();
    let _relock = Restore(locked.clone());
    for p in &locked {
        fs::set_permissions(p, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let o = blt_cmd(&["show", "10ff03"]);
    assert!(
        !o.status.success(),
        "read must not silently recover in a read-only registry"
    );
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("blt check") || e.contains(".move"), "{e}");
    assert!(d.join(".blt/.move").exists(), "journal must remain");
    assert!(d.join(".blt/open/10ff03.md").exists());
}

#[test]
fn init_repairs_missing_components() {
    let d = dir("initfix");
    ok(&d, &["init", "--no-hooks"]);
    fs::remove_dir_all(d.join(".blt/closed")).unwrap();
    fs::remove_file(d.join(".blt/memory.md")).unwrap();
    fs::write(d.join(".blt/.gitignore"), "custom-only\n").unwrap();
    assert_eq!(ok(&d, &["init", "--no-hooks"]), "initialized .blt/\n");
    assert!(d.join(".blt/closed").is_dir());
    assert_eq!(
        fs::read_to_string(d.join(".blt/memory.md")).unwrap(),
        "# Project Memory\n",
        "repair writes fresh memory.md"
    );
    assert_eq!(
        fs::read_to_string(d.join(".blt/.gitignore")).unwrap(),
        "custom-only\n",
        "existing .gitignore bytes preserved"
    );
    fs::write(
        d.join(".blt/memory.md"),
        "# Project Memory\n\n## kept\n> v\n",
    )
    .unwrap();
    assert_eq!(
        ok(&d, &["init", "--no-hooks"]),
        ".blt/ already initialized\n"
    );
    assert_eq!(
        fs::read_to_string(d.join(".blt/memory.md")).unwrap(),
        "# Project Memory\n\n## kept\n> v\n"
    );
    let _id = add(&d, "post repair");

    #[cfg(unix)]
    {
        fs::remove_dir_all(d.join(".blt/open")).unwrap();
        std::os::unix::fs::symlink("closed", d.join(".blt/open")).unwrap();
        let e = bad(&d, &["init", "--no-hooks"]);
        assert!(e.contains("open"), "{e}");
        fs::remove_file(d.join(".blt/open")).unwrap();
        fs::create_dir(d.join(".blt/open")).unwrap();
    }

    let outer = dir("initouter");
    ok(&outer, &["init", "--no-hooks"]);
    let inner = outer.join("inner");
    fs::create_dir_all(inner.join(".blt/open")).unwrap();
    let e = bad(&inner, &["list"]);
    assert!(
        e.contains("blt init"),
        "partial registry must point at init, not the ancestor: {e}"
    );
    assert_eq!(ok(&inner, &["init", "--no-hooks"]), "initialized .blt/\n");
    assert!(inner.join(".blt/closed").is_dir());
    assert!(inner.join(".blt/memory.md").is_file());
    assert_eq!(ok(&inner, &["list", "--json"]).trim(), "[]");
}

fn show_value(d: &Path, id: &str) -> Value {
    serde_json::from_str(&ok(d, &["show", id, "--json"])).unwrap()
}

fn hist_last_value(history: &str, label: &str) -> Option<String> {
    for part in history.split("\n- ").collect::<Vec<_>>().into_iter().rev() {
        let entry = part.strip_prefix("- ").unwrap_or(part);
        let lines: Vec<&str> = entry.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let content = if i == 0 {
                line.splitn(4, ' ').nth(3).unwrap_or("")
            } else {
                line.strip_prefix("  ").unwrap_or(line)
            };
            if let Some(tail) = content.strip_prefix(&format!("{label} ")) {
                return Some(tail.to_string());
            }
            if content == label {
                let mut v = Vec::new();
                for l in &lines[i + 1..] {
                    match l.strip_prefix("  > ") {
                        Some(x) => v.push(x),
                        None => break,
                    }
                }
                return Some(v.join("\n"));
            }
        }
    }
    None
}

const AGENTS_LINE: &str = "This project tracks work with `blt` (https://github.com/EntropyLogix/Billet): run `blt start` before you begin and follow its rules; if `blt` is missing, ask a person to install it.";

#[test]
fn agent_instruction_line_states_preserve_outside_bytes() {
    let fresh = dir("agfresh");
    ok(&fresh, &["init", "--no-hooks"]);
    assert_eq!(
        fs::read_to_string(fresh.join("AGENTS.md")).unwrap(),
        format!("{AGENTS_LINE}\n")
    );

    for nl in ["\n", "\r\n"] {
        let pre = format!("pre text{nl}{nl}");
        let post = format!("{nl}post text{nl}");
        let line = AGENTS_LINE;
        let near = [
            format!("  {line}"),
            line.trim_end_matches('.').to_string(),
            format!("- {line}"),
        ];

        let fixture = |name: &str, agents: &str| -> TestDir {
            let d = dir(name);
            ok(&d, &["init", "--no-hooks"]);
            fs::write(d.join("AGENTS.md"), agents).unwrap();
            d
        };
        let check_unchanged = |d: &Path, code: i32| -> String {
            let agents = fs::read(d.join("AGENTS.md")).unwrap();
            let tree = tree_bytes(&d.join(".blt"));
            let o = run(d, &["check"]);
            assert_eq!(o.status.code(), Some(code));
            assert_eq!(fs::read(d.join("AGENTS.md")).unwrap(), agents);
            assert_eq!(tree_bytes(&d.join(".blt")), tree);
            String::from_utf8(o.stdout).unwrap()
        };
        let init_writes = |d: &Path, expected: &str| {
            assert_eq!(
                ok(d, &["init", "--no-hooks"]),
                ".blt/ already initialized\n"
            );
            assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), expected);
        };

        let missing = [
            (format!("{pre}custom rules{nl}"), format!("{nl}{line}{nl}")),
            (format!("{pre}custom rules"), format!("{nl}{nl}{line}{nl}")),
            (format!("{pre}custom rules{nl}{nl}"), format!("{line}{nl}")),
            (format!("{pre}{}{post}", near[0]), format!("{nl}{line}{nl}")),
            (format!("{pre}{}{post}", near[1]), format!("{nl}{line}{nl}")),
            (format!("{}{nl}", near[2]), format!("{nl}{line}{nl}")),
        ];
        for (i, (file, appended)) in missing.iter().enumerate() {
            let d = fixture(&format!("ag-miss{i}"), file);
            let out = check_unchanged(&d, 1);
            assert!(
                out.contains("AGENTS.md: Billet instructions are missing"),
                "{out}"
            );
            assert!(out.contains("run `blt init`"), "{out}");
            init_writes(&d, &format!("{file}{appended}"));
            assert!(check_unchanged(&d, 0).contains("clean"));
            init_writes(&d, &format!("{file}{appended}"));
        }

        for (i, file) in [
            format!("{line}{nl}"),
            format!("{pre}{line}{post}"),
            format!("{pre}{line}"),
            format!("{pre}{line}{nl}{line}{nl}"),
        ]
        .iter()
        .enumerate()
        {
            let d = fixture(&format!("ag-cur{i}"), file);
            assert!(check_unchanged(&d, 0).contains("clean"));
            init_writes(&d, file);
        }
    }
}

#[test]
fn start_covers_workflow_rules_in_order() {
    let d = dir("agentrules");
    ok(&d, &["init", "--no-hooks"]);
    let text = ok(&d, &["start"]);
    let json: Value = serde_json::from_str(&ok(&d, &["start", "--json"])).unwrap();
    let hook: Value = serde_json::from_str(&ok(&d, &["start", "--hook-json"])).unwrap();
    let workflow = json["workflow"].as_str().unwrap();
    assert_eq!(format!("{workflow}\n"), text);
    assert_eq!(
        hook["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap(),
        workflow
    );
    let tokens = [
        "Billet workflow",
        "tracks work with `blt`",
        "Do not keep task lists",
        "built-in todo or plan tool",
        "anything that must outlive this session in `blt`",
        concat!(
            "cargo install --locked --git https://github.com/EntropyLogix/Billet --tag v",
            env!("CARGO_PKG_VERSION")
        ),
        "https://github.com/EntropyLogix/Billet",
        "blt start` for the workflow",
        "blt ready` lists approved, unblocked work",
        "blt show ID` reads one issue",
        "authorizes only its described scope",
        "blt claim ID --by NAME",
        "your agent name",
        "blt review ID --summary TEXT",
        "returning for rework",
        "for people unless a person explicitly tells you otherwise",
        "blt add TITLE --body TEXT",
        "blt note ID TEXT",
        "blt unclaim ID",
        "blt recall [TOPIC]",
        "blt remember TOPIC TEXT",
        "your own memory",
        "blt check` and report to a person",
    ];
    let agents = fs::read_to_string(d.join("AGENTS.md")).unwrap();
    let mut at = 0;
    for token in tokens {
        let i = workflow[at..]
            .find(token)
            .unwrap_or_else(|| panic!("{token:?} missing at {at} in {workflow:?}"));
        at += i + token.len();
    }
    for hay in [workflow, agents.as_str()] {
        for gone in [
            ["Entro", "pyJig"].concat(),
            ["ENTROPY", "JIG"].concat(),
            ["j", "ig"].concat(),
            ["J", "ig"].concat(),
            "not approval to implement a contract".to_string(),
            "Readiness is not approval".to_string(),
        ] {
            assert!(!hay.contains(&gone), "{gone} leaked into {hay}");
        }
    }
}

fn file_key(d: &Path, id: &str) -> String {
    if billet::issue::valid_key(id) {
        id.to_string()
    } else {
        key_of(d, id)
    }
}

fn set_priority_null(d: &Path, id: &str) {
    let path = d.join(format!(".blt/open/{}.md", file_key(d, id)));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("priority: 2", "priority: null")).unwrap();
}

fn mcp_run_bytes(dir: &Path, input: &[u8]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_blt"))
        .arg("mcp")
        .current_dir(dir)
        .env("XDG_STATE_HOME", dir.join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).unwrap();
            let _ = sender.send(bytes);
        });
        receiver
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    child.stdin.take().unwrap().write_all(input).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut collect = |pipe: &mpsc::Receiver<Vec<u8>>| {
        let left = deadline.saturating_duration_since(Instant::now());
        pipe.recv_timeout(left).unwrap_or_else(|_| {
            let _ = child.kill();
            panic!("blt mcp hung: no end of output within ten seconds");
        })
    };
    let stdout = collect(&stdout);
    let stderr = String::from_utf8(collect(&stderr)).unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "blt mcp exited {status}: {stderr}");
    String::from_utf8(stdout).unwrap()
}

fn mcp_run(dir: &Path, messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();
    for message in messages {
        input.extend(serde_json::to_vec(message).unwrap());
        input.push(b'\n');
    }
    mcp_run_bytes(dir, &input)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn mcp_call(id: u64, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "method":"tools/call",
        "params":{"name":name,"arguments":arguments}
    })
}

#[test]
fn mcp_initialize_protocol() {
    let d = dir("mcpinit");
    ok(&d, &["init", "--no-hooks"]);
    let subdir = d.join("sub");
    fs::create_dir(&subdir).unwrap();
    let response = mcp_run(
        &subdir,
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
        ],
    );
    let result = &response[0]["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "blt");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["capabilities"], json!({"tools":{}}));
    let start = ok(&d, &["start"]);
    let expected = format!(
        "{}\n\nUse these MCP tools instead of running `blt` in the shell. Each tool mirrors the command of the same name. A person's refusal of a tool call is final and is not retried through the shell.",
        start.trim_end_matches('\n')
    );
    assert_eq!(result["instructions"], expected);
    assert!(!start.contains("MCP"));
    assert!(
        !fs::read_to_string(d.join("AGENTS.md"))
            .unwrap()
            .contains("MCP")
    );
}

#[test]
fn mcp_tools_list_dispatcher_consistency() {
    let d = dir("mcplist");
    let response = mcp_run(&d, &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})]);
    let names: Vec<String> = response[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    let expected: Vec<String> = billet::commands::command_names()
        .filter(|name| *name != "mcp")
        .flat_map(|name| {
            if name == "dep" {
                vec!["dep_add".to_string(), "dep_rm".to_string()]
            } else {
                vec![name.to_string()]
            }
        })
        .collect();
    assert_eq!(names, expected);
    let read = ["ready", "show", "list", "tree", "check", "recall", "start"];
    let decisions = [
        "approve", "revise", "rework", "close", "reopen", "release", "init",
    ];
    for tool in response[0]["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(
            tool["annotations"]["readOnlyHint"]
                .as_bool()
                .unwrap_or(false),
            read.contains(&name)
        );
        assert_eq!(
            tool["description"]
                .as_str()
                .unwrap()
                .starts_with("A person's decision:"),
            decisions.contains(&name)
        );
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
}

struct McpSession {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: mpsc::Receiver<String>,
}

impl McpSession {
    fn start(dir: &Path) -> McpSession {
        let mut child = Command::new(env!("CARGO_BIN_EXE_blt"))
            .arg("mcp")
            .current_dir(dir)
            .env("XDG_STATE_HOME", dir.join("xdg"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        McpSession {
            stdin: child.stdin.take(),
            child,
            lines,
        }
    }

    fn request(&mut self, message: Value) -> Value {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
        let line = self
            .lines
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| {
                let _ = self.child.kill();
                panic!("blt mcp hung: no answer to {message}");
            });
        serde_json::from_str(&line).unwrap()
    }

    fn finish(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        let (sender, receiver) = mpsc::channel();
        let mut child = self.child;
        std::thread::spawn(move || {
            let _ = sender.send(child.wait().unwrap());
        });
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("blt mcp did not exit after EOF")
    }
}

#[test]
fn mcp_protocol_transport() {
    let d = dir("mcpprotocol");
    let mut input = b"not json\n\xff\n".to_vec();
    input.extend(serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"ping"})).unwrap());
    input.push(b'\n');
    input.extend(
        serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"notifications/cancelled"})).unwrap(),
    );
    input.push(b'\n');
    input.extend(serde_json::to_vec(&json!({"jsonrpc":"2.0","id":2,"method":"unknown"})).unwrap());
    input.push(b'\n');
    input.extend(serde_json::to_vec(&json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unknown","arguments":{}}})).unwrap());
    input.push(b'\n');
    let lines: Vec<Value> = mcp_run_bytes(&d, &input)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2]["result"], json!({}));
    assert_eq!(lines[3]["error"]["code"], -32601);
    assert_eq!(lines[4]["error"]["code"], -32602);
    let ids = [
        json!({"x":1}),
        json!([1]),
        json!(true),
        Value::Null,
        json!("s"),
        json!(7),
    ];
    let pings: Vec<Value> = ids
        .iter()
        .map(|id| json!({"jsonrpc":"2.0","id":id,"method":"ping"}))
        .collect();
    let answers = mcp_run(&d, &pings);
    for (id, answer) in ids.iter().zip(&answers) {
        if id.is_string() || id.is_number() {
            assert_eq!(
                answer,
                &json!({"jsonrpc":"2.0","id":id,"result":{}}),
                "id {id}"
            );
        } else {
            assert_eq!(
                answer,
                &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}),
                "id {id} is neither a string nor a number"
            );
        }
    }
    let invalid = mcp_run(
        &d,
        &[
            mcp_call(4, "add", json!({"extra":true})),
            mcp_call(5, "add", json!({"title":"x","priority":"1"})),
            mcp_call(6, "add", json!({})),
        ],
    );
    assert!(
        invalid[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("extra")
    );
    assert!(
        invalid[1]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("priority")
    );
    assert!(
        invalid[2]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("title")
    );
    let state = d.join("state");
    fs::create_dir(&state).unwrap();
    ok(&state, &["init", "--no-hooks"]);
    let id = add(&state, "watched");
    let mut session = McpSession::start(&state);
    let show = |session: &mut McpSession, n: u64| -> String {
        let answer = session.request(mcp_call(n, "show", json!({"id": id})));
        answer["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = show(&mut session, 1);
    assert_eq!(first, ok(&state, &["--json", "show", &id]));
    ok(&state, &["defer", &id, "--until", "2999-01-01"]);
    let second = show(&mut session, 2);
    assert_eq!(
        second,
        ok(&state, &["--json", "show", &id]),
        "the second call in the same session sees the change made through the CLI"
    );
    assert_ne!(first, second);
    assert!(session.finish().success(), "blt mcp exits 0 at EOF");
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn normalize(text: &str, ids: &[(String, String)]) -> String {
    let mut text = text.to_string();
    for (from, to) in ids {
        text = text.replace(from.as_str(), to);
    }
    let bytes = text.as_bytes();
    let is_time = |at: usize| {
        let Some(window) = bytes.get(at..at + 19) else {
            return false;
        };
        window.iter().enumerate().all(|(i, byte)| match i {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T' || *byte == b' ',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
    };
    let hex_run = |at: usize| {
        bytes[at..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count()
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let starts_word = at == 0 || !bytes[at - 1].is_ascii_hexdigit();
        if is_time(at) {
            out.push_str("<time>");
            at += 19;
        } else if starts_word && hex_run(at) == 32 {
            out.push_str("<claim>");
            at += 32;
        } else {
            let len = text[at..].chars().next().unwrap().len_utf8();
            out.push_str(&text[at..at + len]);
            at += len;
        }
    }
    out
}

fn issue_ids(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = tree_bytes(&root.join(".blt"))
        .into_iter()
        .filter_map(|(path, _)| {
            let name = Path::new(&path).file_name()?.to_str()?.to_string();
            let id = name.strip_suffix(".md")?;
            id.starts_with("B-").then(|| id.to_string())
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn cli_as_tool_result(output: &Output) -> (bool, String) {
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    if stdout.is_empty() && !output.status.success() {
        let message = stderr
            .strip_prefix("blt: ")
            .and_then(|message| message.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("refusal is not one `blt: ` line: {stderr:?}"));
        return (true, message.to_string());
    }
    assert!(
        stderr
            .lines()
            .all(|line| line.starts_with("blt: warning: ")),
        "stderr of a completed command holds only warnings: {stderr:?}"
    );
    let mut text = stdout;
    if !stderr.is_empty() && !text.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }
    text.push_str(&stderr);
    (false, text)
}

fn mcp_parity_without_registry(d: &Path) {
    let empty_mcp = d.join("empty-mcp");
    let empty_cli = d.join("empty-cli");
    for path in [&empty_mcp, &empty_cli] {
        fs::create_dir(path).unwrap();
    }
    for (tool, argv) in [("ready", vec!["--json", "ready"]), ("init", vec!["init"])] {
        let response = mcp_run(&empty_mcp, &[mcp_call(1, tool, json!({}))]);
        let (is_error, expected) = cli_as_tool_result(&run(&empty_cli, &argv));
        assert_eq!(
            response[0]["result"]["isError"], is_error,
            "{tool} without a registry"
        );
        assert_eq!(
            normalize(
                response[0]["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap(),
                &[]
            ),
            normalize(&expected, &[]),
            "{tool} without a registry"
        );
        let project = |root: &Path| -> Vec<(String, String)> {
            tree_bytes(root)
                .into_iter()
                .map(|(path, bytes)| (path, normalize(&String::from_utf8_lossy(&bytes), &[])))
                .collect()
        };
        assert_eq!(
            project(&empty_mcp),
            project(&empty_cli),
            "{tool} without a registry: files"
        );
    }
    assert!(empty_mcp.join(".blt").is_dir() && empty_mcp.join("AGENTS.md").is_file());
}

#[test]
fn mcp_cli_parity_all_tools() {
    let d = dir("mcpparity");
    let base = d.join("base");
    fs::create_dir(&base).unwrap();
    ok(&base, &["init", "--no-hooks"]);
    let a = add(&base, "a");
    let b = add(&base, "b");
    let c = ok(&base, &["add", "--approved", "--parent", &a, "c"])
        .trim()
        .to_string();
    let dangling = fm("3deadb", "open", "").replace("depends_on: []", "depends_on: [\"2dead1\"]");
    fs::write(base.join(".blt/open/3deadb.md"), dangling).unwrap();
    let via_mcp = d.join("mcp");
    let via_cli = d.join("cli");
    copy_tree(&base, &via_mcp);
    copy_tree(&base, &via_cli);

    let cases: Vec<(&str, Value, Vec<&str>)> = vec![
        ("init", json!({"no_hooks":true}), vec!["init", "--no-hooks"]),
        (
            "init",
            json!({"no_hooks":true,"prefix":"B"}),
            vec!["init", "--no-hooks", "--prefix", "B"],
        ),
        ("start", json!({}), vec!["--json", "start"]),
        ("show", json!({"id":a}), vec!["--json", "show", &a]),
        ("tree", json!({"id":a}), vec!["--json", "tree", &a]),
        (
            "list",
            json!({"status":"all","sort":"newest"}),
            vec!["--json", "list", "--status", "all", "--sort", "newest"],
        ),
        ("ready", json!({}), vec!["--json", "ready"]),
        (
            "add",
            json!({"title":"--force","approved":true,"priority":1,"body":"line1\nline2","parent":a,"dep":[b]}),
            vec![
                "add",
                "--approved",
                "-p",
                "1",
                "--body",
                "line1\nline2",
                "--parent",
                &a,
                "--dep",
                &b,
                "--",
                "--force",
            ],
        ),
        (
            "add",
            json!({"title":"d","dep":[a, "-x"]}),
            vec!["add", "d", "--dep", &a, "--dep", "-x"],
        ),
        (
            "dep_add",
            json!({"id":b,"blocker":a}),
            vec!["dep", "add", &b, &a],
        ),
        (
            "dep_add",
            json!({"id":b,"blocker":"1dead0"}),
            vec!["dep", "add", &b, "1dead0"],
        ),
        ("claim", json!({"id":b}), vec!["claim", &b]),
        (
            "claim",
            json!({"id":b,"by":"x","force":true}),
            vec!["claim", &b, "--by", "x", "--force"],
        ),
        (
            "dep_rm",
            json!({"id":b,"blocker":a}),
            vec!["dep", "rm", &b, &a],
        ),
        (
            "note",
            json!({"id":b,"text":"--n","by":"-me"}),
            vec!["note", "--by", "-me", "--", &b, "--n"],
        ),
        (
            "update",
            json!({"id":b,"title":"-t","priority":0,"body":"--"}),
            vec!["update", &b, "--title", "-t", "-p", "0", "--body", "--"],
        ),
        (
            "update",
            json!({"id":b,"body":"-"}),
            vec!["update", &b, "--body", "-"],
        ),
        (
            "update",
            json!({"id":b,"release":"-r"}),
            vec!["update", &b, "--release", "-r"],
        ),
        (
            "update",
            json!({"id":a,"number":"77"}),
            vec!["update", &a, "--number", "77"],
        ),
        (
            "update",
            json!({"id":"B77","number":"78"}),
            vec!["update", "B77", "--number", "78"],
        ),
        (
            "update",
            json!({"id":"B78","number":a.trim_start_matches('B')}),
            vec!["update", "B78", "--number", a.trim_start_matches('B')],
        ),
        (
            "review",
            json!({"id":b,"summary":"done"}),
            vec!["review", &b, "--summary", "done"],
        ),
        (
            "rework",
            json!({"id":b,"reason":"again"}),
            vec!["rework", &b, "--reason", "again"],
        ),
        ("unclaim", json!({"id":b}), vec!["unclaim", &b]),
        (
            "revise",
            json!({"id":c,"reason":"r"}),
            vec!["revise", &c, "--reason", "r"],
        ),
        ("approve", json!({"id":c}), vec!["approve", &c]),
        (
            "approve",
            json!({"id":"B-nope00"}),
            vec!["approve", "B-nope00"],
        ),
        (
            "defer",
            json!({"id":c,"until":"2999-01-01"}),
            vec!["defer", &c, "--until", "2999-01-01"],
        ),
        (
            "defer",
            json!({"id":c,"clear":true}),
            vec!["defer", &c, "--clear"],
        ),
        (
            "close",
            json!({"id":c,"reason":"ok","summary":"s"}),
            vec!["close", &c, "--reason", "ok", "--summary", "s"],
        ),
        ("release", json!({"name":"1.0"}), vec!["release", "1.0"]),
        (
            "list",
            json!({"status":"all","release":"1.0"}),
            vec!["--json", "list", "--status", "all", "--release", "1.0"],
        ),
        (
            "reopen",
            json!({"id":c,"reason":"back"}),
            vec!["reopen", &c, "--reason", "back"],
        ),
        (
            "remember",
            json!({"topic":"parity","text":"-v"}),
            vec!["remember", "--", "parity", "-v"],
        ),
        (
            "recall",
            json!({"topic":"parity"}),
            vec!["--json", "recall", "parity"],
        ),
        ("recall", json!({}), vec!["--json", "recall"]),
        ("check", json!({}), vec!["--json", "check"]),
    ];

    let listed = mcp_run(&d, &[json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})]);
    let mut uncovered = Vec::new();
    for tool in listed[0]["result"]["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let used: Vec<&Value> = cases
            .iter()
            .filter(|(case, _, _)| *case == name)
            .map(|(_, arguments, _)| arguments)
            .collect();
        if used.is_empty() {
            uncovered.push(format!("{name}: no case"));
        }
        for argument in tool["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
        {
            if !used
                .iter()
                .any(|arguments| arguments.get(argument).is_some())
            {
                uncovered.push(format!("{name}.{argument}: no case uses it"));
            }
        }
    }
    assert!(uncovered.is_empty(), "parity cases miss: {uncovered:#?}");

    let without_registry = std::thread::spawn({
        let d = d.to_path_buf();
        move || mcp_parity_without_registry(&d)
    });
    let mut ids = Vec::new();
    let mut refused = 0;
    let mut warned = 0;
    for (index, (tool, arguments, argv)) in cases.iter().enumerate() {
        let case = format!("case {index}: {tool} {arguments}");
        let before = (issue_ids(&via_mcp), issue_ids(&via_cli));
        let response = mcp_run(
            &via_mcp,
            &[
                mcp_call(1, tool, arguments.clone()),
                json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
            ],
        );
        let result = &response[0]["result"];
        assert!(result.is_object(), "{case}: protocol error {response:?}");
        assert_eq!(
            response[1]["result"],
            json!({}),
            "{case}: the server keeps serving"
        );
        let cli = match argv.windows(2).position(|w| w == ["--body", "-"]) {
            Some(_) => run_stdin(&via_cli, argv, arguments["body"].as_str().unwrap()),
            None => run(&via_cli, argv),
        };
        let created = |root: &Path, before: &[String]| -> Vec<String> {
            issue_ids(root)
                .into_iter()
                .filter(|id| !before.contains(id))
                .collect()
        };
        let new_mcp = created(&via_mcp, &before.0);
        let new_cli = created(&via_cli, &before.1);
        assert_eq!(
            new_mcp.len(),
            new_cli.len(),
            "{case}: created issues differ"
        );
        ids.extend(new_cli.into_iter().zip(new_mcp));

        let (is_error, expected) = cli_as_tool_result(&cli);
        refused += usize::from(is_error);
        warned += usize::from(expected.contains("blt: warning: "));
        assert_eq!(result["isError"], is_error, "{case}: isError");
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            normalize(text, &[]),
            normalize(&expected, &ids),
            "{case}: result text"
        );
        let registry = |root: &Path, ids: &[(String, String)]| -> Vec<(String, String)> {
            tree_bytes(root)
                .into_iter()
                .map(|(path, bytes)| {
                    (
                        normalize(&path, ids),
                        normalize(&String::from_utf8_lossy(&bytes), ids),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_iter()
                .collect()
        };
        assert_eq!(
            registry(&via_mcp, &[]),
            registry(&via_cli, &ids),
            "{case}: registry"
        );
    }
    assert!(
        refused > 0 && warned > 0,
        "refused {refused}, warned {warned}"
    );
    without_registry.join().unwrap();
}

struct TextField {
    command: &'static str,
    name: &'static str,
    field: &'static str,
    kind: billet::text::TextKind,
    required: bool,
}

fn text_fields() -> Vec<TextField> {
    use billet::commands::{ArgType, COMMANDS};
    COMMANDS
        .iter()
        .flat_map(|command| {
            command.args.iter().filter_map(move |arg| match arg.kind {
                ArgType::Text(kind) | ArgType::TextList(kind) => Some(TextField {
                    command: command.name,
                    name: arg.name,
                    field: arg.field(),
                    kind,
                    required: arg.required,
                }),
                _ => None,
            })
        })
        .collect()
}

const HELP: &str = concat!(
    "Billet - file-based issue tracker and project memory\n\n",
    "Usage: blt [--json] COMMAND [ARGS]\n\n",
    "Browse\n",
    "  list [--status S] [--release NAME] [--sort oldest|newest]\n",
    "                                  issues grouped by status (default)\n",
    "  show ID                         one full issue and its documents\n",
    "  tree ID                         issue with its subtasks\n",
    "  ready                           open, unblocked work for agents\n\n",
    "Create and edit\n",
    "  add TITLE [--approved] [-p N] [--body TEXT|-] [--parent ID] [--dep ID...]\n",
    "                                  new issue (proposed unless --approved)\n",
    "  update ID [--title T] [-p N] [--body TEXT|-] [--release NAME] [--number N]\n",
    "                                  change the given fields\n",
    "  note ID TEXT [--by NAME]        one-line note\n",
    "  dep add|rm ID BLOCKER           manage blockers\n",
    "  defer ID --until YYYY-MM-DD | --clear\n",
    "                                  hide an issue until a date\n\n",
    "Work (agents)\n",
    "  claim ID [--by NAME] [--force]  start work on an open issue\n",
    "  unclaim ID                      give a claimed issue back\n",
    "  review ID [--summary TEXT]      submit for review\n\n",
    "Decide (people)\n",
    "  approve ID                      proposed -> open\n",
    "  revise ID --reason TEXT         open -> proposed\n",
    "  rework ID --reason TEXT         review -> claimed\n",
    "  close ID --reason TEXT [--summary TEXT]\n",
    "                                  archive an issue\n",
    "  reopen ID --reason TEXT         reactivate a closed issue\n",
    "  release NAME                    seal closed issues with a release name\n\n",
    "Memory\n",
    "  remember TOPIC TEXT             record a project fact\n",
    "  recall [TOPIC]                  read project facts\n\n",
    "Setup\n",
    "  init [--no-hooks] [--prefix P]  create .blt/, AGENTS.md and project hooks\n",
    "  start                           workflow summary for agents\n",
    "  check                           report registry problems\n",
    "  mcp                             MCP server over stdio (started by the client)\n\n",
    "Options\n",
    "  --json             machine output for read commands\n",
    "  -p, --priority N   0 (highest) to 4, default 2\n",
    "  S                  proposed|open|claimed|review|closed|all\n",
    "  --                 end of options; what follows is text\n",
    "  --help, --version\n",
);

struct Seeded {
    dir: TestDir,
    base: PathBuf,
    open: String,
    other: String,
    proposed: String,
    claimed: String,
    review: String,
    closed: String,
}

fn seeded(name: &str) -> Seeded {
    let dir = dir(name);
    let base = dir.join("base");
    fs::create_dir(&base).unwrap();
    ok(&base, &["init", "--no-hooks"]);
    let open = add(&base, "open");
    let other = add(&base, "other");
    let proposed = ok(&base, &["add", "proposed"]).trim().to_string();
    let claimed = add(&base, "claimed");
    let review = add(&base, "review");
    let closed = add(&base, "closed");
    ok(&base, &["claim", &claimed]);
    ok(&base, &["claim", &review]);
    ok(&base, &["review", &review]);
    ok(&base, &["close", &closed, "--reason", "done"]);
    ok(&base, &["remember", "topic", "value"]);
    Seeded {
        dir,
        base,
        open,
        other,
        proposed,
        claimed,
        review,
        closed,
    }
}

impl Seeded {
    fn fresh(&self, case: usize) -> PathBuf {
        let path = self.dir.join(format!("case{case}"));
        copy_tree(&self.base, &path);
        path
    }
}

fn text_case_argv(s: &Seeded, command: &str, name: &str, value: &str) -> Option<Vec<String>> {
    let argv: Vec<&str> = match (command, name) {
        ("add", "title") => vec!["add", "--", value],
        ("add", "body") => vec!["add", "x", "--body", value],
        ("add", "parent") => vec!["add", "x", "--parent", value],
        ("add", "dep") => vec!["add", "x", "--dep", value],
        ("update", "id") => vec!["update", "-p", "1", "--", value],
        ("update", "title") => vec!["update", &s.open, "--title", value],
        ("update", "body") => vec!["update", &s.open, "--body", value],
        ("update", "release") => vec!["update", &s.closed, "--release", value],
        ("update", "number") => vec!["update", &s.open, "--number", value],
        ("init", "prefix") => vec!["init", "--no-hooks", "--prefix", value],
        ("list", "release") => vec!["list", "--release", value],
        ("show" | "tree", "id") => vec![command, "--", value],
        ("approve", "id") => vec!["approve", "--", value],
        ("unclaim", "id") => vec!["unclaim", "--", value],
        ("claim", "id") => vec!["claim", "--", value],
        ("claim", "by") => vec!["claim", &s.open, "--by", value],
        ("review", "id") => vec!["review", "--", value],
        ("review", "summary") => vec!["review", &s.claimed, "--summary", value],
        ("revise", "id") => vec!["revise", "--reason", "r", "--", value],
        ("revise", "reason") => vec!["revise", &s.open, "--reason", value],
        ("rework", "id") => vec!["rework", "--reason", "r", "--", value],
        ("rework", "reason") => vec!["rework", &s.review, "--reason", value],
        ("close", "id") => vec!["close", "--reason", "r", "--", value],
        ("close", "reason") => vec!["close", &s.open, "--reason", value],
        ("close", "summary") => vec!["close", &s.open, "--reason", "r", "--summary", value],
        ("reopen", "id") => vec!["reopen", "--reason", "r", "--", value],
        ("reopen", "reason") => vec!["reopen", &s.closed, "--reason", value],
        ("release", "name") => vec!["release", "--", value],
        ("dep", "id") => vec!["dep", "add", "--", value, &s.other],
        ("dep", "blocker") => vec!["dep", "add", "--", &s.open, value],
        ("note", "id") => vec!["note", "--", value, "n"],
        ("note", "text") => vec!["note", "--", &s.open, value],
        ("note", "by") => vec!["note", &s.open, "n", "--by", value],
        ("defer", "id") => vec!["defer", "--clear", "--", value],
        ("defer", "until") => vec!["defer", &s.open, "--until", value],
        ("remember", "topic") => vec!["remember", "--", value, "v"],
        ("remember", "text") => vec!["remember", "--", "topic", value],
        ("recall", "topic") => vec!["recall", "--", value],
        _ => return None,
    };
    Some(argv.into_iter().map(str::to_string).collect())
}

fn id_target(s: &Seeded, command: &str, name: &str) -> String {
    match (command, name) {
        ("approve", _) => s.proposed.clone(),
        ("unclaim" | "review", _) => s.claimed.clone(),
        ("rework", _) => s.review.clone(),
        ("reopen", _) => s.closed.clone(),
        ("dep", "blocker") => s.other.clone(),
        _ => s.open.clone(),
    }
}

fn text_representatives(s: &Seeded, field: &TextField) -> Vec<(String, bool)> {
    use billet::text::TextKind;
    let owned = |items: &[(&str, bool)]| -> Vec<(String, bool)> {
        items.iter().map(|(v, a)| (v.to_string(), *a)).collect()
    };
    match field.kind {
        TextKind::Id => {
            let target = id_target(s, field.command, field.name);
            vec![
                (target.clone(), true),
                (target.to_lowercase(), true),
                (target.trim_start_matches('B').to_string(), true),
                ("B-zzzzzz".to_string(), false),
                ("B012".to_string(), false),
                ("--force".to_string(), false),
            ]
        }
        TextKind::Label => owned(&[
            ("zażółć 🎲 --force", true),
            ("", false),
            ("   ", false),
            ("a\nb", false),
            ("tab\tx", false),
            ("sep\u{2028}x", false),
        ]),
        TextKind::Release => owned(&[
            ("1.0-rc", true),
            (" 1.0", false),
            ("", false),
            ("a\nb", false),
        ]),
        TextKind::Topic => owned(&[("a.b_c-1", true), ("-x", false), ("a b", false)]),
        TextKind::Date => owned(&[
            ("2999-01-01", true),
            ("2999-02-30", false),
            ("tomorrow", false),
        ]),
        TextKind::Note => vec![
            ("-n 🎲".to_string(), true),
            ("ą".repeat(500), true),
            (String::new(), false),
            ("a\nb".to_string(), false),
            ("a\u{7f}b".to_string(), false),
            ("x".repeat(501), false),
        ],
        TextKind::Multiline => owned(&[
            ("line1\n--line2", true),
            ("   ", !field.required),
            ("tab\tx", false),
            ("## History", false),
            ("```\nopen", false),
            ("<<<<<<< a\n=======\n>>>>>>> b", false),
        ]),
        TextKind::Body => owned(&[
            ("x\n```\n## Notes\n```\n\u{1b}", true),
            ("", true),
            ("--", true),
            ("## History", false),
            ("```\nopen", false),
        ]),
        TextKind::Free if field.command == "recall" => owned(&[("topic", true)]),
        TextKind::Prefix => owned(&[("B", true), ("ab", false), ("B-1", false), ("", false)]),
        TextKind::Number => owned(&[("999", true), ("0", false), ("07", false), ("x1", false)]),
        TextKind::Free => owned(&[
            ("first\n\n> quoted\r\n## History\u{1b}", true),
            ("", true),
            ("--x", true),
        ]),
    }
}

fn text_read_back(d: &Path, field: &TextField, s: &Seeded, stdout: &str) -> Option<String> {
    let show =
        |id: &str| -> Value { serde_json::from_str(&ok(d, &["--json", "show", id])).unwrap() };
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_string();
    let history = |id: &str, label: &str| hist_last_value(&text(&show(id), "history"), label);
    let last_note = |id: &str| -> String {
        let notes = text(&show(id), "notes");
        let line = notes.lines().last().unwrap_or_default().to_string();
        line.splitn(5, ' ').nth(4).unwrap_or_default().to_string()
    };
    match (field.command, field.name) {
        ("add", "title") => Some(text(&show(stdout.trim()), "title")),
        ("add", "body") => Some(text(&show(stdout.trim()), "body")),
        ("update", "title") => Some(text(&show(&s.open), "title")),
        ("update", "body") => Some(text(&show(&s.open), "body")),
        ("update", "release") => Some(text(&show(&s.closed), "release")),
        ("update", "number") => {
            let handle = stdout.trim().strip_prefix("updated ").unwrap();
            Some(
                text(&show(handle), "id")
                    .trim_start_matches('B')
                    .to_string(),
            )
        }
        ("release", "name") => Some(text(&show(&s.closed), "release")),
        ("claim", "by") => Some(text(&show(&s.open), "assignee")),
        ("defer", "until") => Some(text(&show(&s.open), "defer_until")),
        ("review", "summary") => history(&s.claimed, "summary:"),
        ("revise", "reason") => history(&s.open, "revised:"),
        ("rework", "reason") => history(&s.review, "rework requested:"),
        ("close", "reason") => history(&s.open, "closed:"),
        ("close", "summary") => Some(text(&show(&s.open), "closing_summary")),
        ("reopen", "reason") => history(&s.closed, "reopened:"),
        ("note", "text") => Some(last_note(&s.open)),
        ("note", "by") => last_note(&s.open).strip_suffix(": n").map(str::to_string),
        ("remember", "text") => {
            let v: Value = serde_json::from_str(&ok(d, &["--json", "recall", "topic"])).unwrap();
            let plain = ok(d, &["recall", "topic"]);
            let value = v["value"].as_str().unwrap().to_string();
            assert_eq!(plain, format!("{value}\n"), "plain recall prints the value");
            Some(value)
        }
        _ => None,
    }
}

fn check_text_field(s: &Seeded, index: usize, field: &TextField) -> Option<String> {
    for (case, (value, accepted)) in text_representatives(s, field).into_iter().enumerate() {
        let Some(argv) = text_case_argv(s, field.command, field.name, &value) else {
            return Some(format!("{}.{}", field.command, field.name));
        };
        let d = s.fresh(index * 100 + case);
        let before = tree_bytes(&d.join(".blt"));
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let o = run(&d, &args);
        if field.kind == billet::text::TextKind::Body && value != "-" {
            let piped: Vec<&str> = args
                .iter()
                .map(|arg| if *arg == value.as_str() { "-" } else { *arg })
                .collect();
            let p = s.fresh(index * 100 + case + 50);
            let from_stdin = run_stdin(&p, &piped, &value);
            assert_eq!(
                (from_stdin.status.code(), &from_stdin.stderr),
                (o.status.code(), &o.stderr),
                "{}.{} value {value:?} through stdin",
                field.command,
                field.name
            );
            fs::remove_dir_all(&p).unwrap();
        }
        let stdout = String::from_utf8_lossy(&o.stdout).to_string();
        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
        let label = format!(
            "{}.{} ({:?}) value {value:?}",
            field.command, field.name, field.kind
        );
        if accepted {
            assert!(o.status.success(), "{label} must be accepted: {stderr}");
            if let Some(stored) = text_read_back(&d, field, s, &stdout) {
                assert_eq!(stored, value, "{label} must be stored exactly");
            }
            assert_eq!(
                ok(&d, &["--json", "check"]),
                "[]\n",
                "{label} left a problem"
            );
        } else {
            let message = field
                .kind
                .check(field.field, field.required, &value)
                .expect_err("a refused representative must break its rule");
            assert_eq!(
                o.status.code(),
                Some(2),
                "{label} must be a usage error: {stderr}"
            );
            assert_eq!(stderr, format!("blt: {message}\n"), "{label}");
            assert_eq!(
                tree_bytes(&d.join(".blt")),
                before,
                "{label} refused write changed .blt"
            );
        }
        fs::remove_dir_all(&d).unwrap();
    }
    None
}

#[test]
fn every_text_argument_by_kind() {
    use billet::text::TextKind;
    let contract: &[(&str, &str, TextKind)] = &[
        ("list", "release", TextKind::Release),
        ("show", "id", TextKind::Id),
        ("tree", "id", TextKind::Id),
        ("add", "title", TextKind::Label),
        ("add", "body", TextKind::Body),
        ("add", "parent", TextKind::Id),
        ("add", "dep", TextKind::Id),
        ("update", "id", TextKind::Id),
        ("update", "title", TextKind::Label),
        ("update", "body", TextKind::Body),
        ("update", "release", TextKind::Release),
        ("update", "number", TextKind::Number),
        ("init", "prefix", TextKind::Prefix),
        ("note", "id", TextKind::Id),
        ("note", "text", TextKind::Note),
        ("note", "by", TextKind::Label),
        ("dep", "id", TextKind::Id),
        ("dep", "blocker", TextKind::Id),
        ("defer", "id", TextKind::Id),
        ("defer", "until", TextKind::Date),
        ("claim", "id", TextKind::Id),
        ("claim", "by", TextKind::Label),
        ("unclaim", "id", TextKind::Id),
        ("review", "id", TextKind::Id),
        ("review", "summary", TextKind::Multiline),
        ("approve", "id", TextKind::Id),
        ("revise", "id", TextKind::Id),
        ("revise", "reason", TextKind::Multiline),
        ("rework", "id", TextKind::Id),
        ("rework", "reason", TextKind::Multiline),
        ("close", "id", TextKind::Id),
        ("close", "reason", TextKind::Multiline),
        ("close", "summary", TextKind::Multiline),
        ("reopen", "id", TextKind::Id),
        ("reopen", "reason", TextKind::Multiline),
        ("release", "name", TextKind::Release),
        ("remember", "topic", TextKind::Topic),
        ("remember", "text", TextKind::Free),
        ("recall", "topic", TextKind::Free),
    ];
    let fields: Vec<TextField> = text_fields();
    let declared: Vec<(&str, &str, TextKind)> = fields
        .iter()
        .map(|field| (field.command, field.name, field.kind))
        .collect();
    let mut expected = contract.to_vec();
    let mut actual = declared.clone();
    expected.sort_by_key(|(c, n, _)| (*c, *n));
    actual.sort_by_key(|(c, n, _)| (*c, *n));
    assert_eq!(
        actual, expected,
        "declared text kinds differ from the SPEC contract"
    );

    let fresh = dir("textsalt");
    ok(&fresh, &["init", "--no-hooks"]);
    assert!(
        !fresh.join("xdg").exists(),
        "init must not create the state home"
    );
    bad(&fresh, &["add", "--", "   "]);
    assert!(
        !fresh.join("xdg").exists(),
        "a refused add must not install the salt"
    );
    add(&fresh, "first");
    assert!(
        fresh.join("xdg/blt/install-id").is_file(),
        "the install salt lives under XDG_STATE_HOME/blt"
    );

    let s = seeded("textkinds");
    let missing: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let s = &s;
                scope.spawn(move || check_text_field(s, index, field))
            })
            .collect();
        workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect()
    });
    assert!(
        missing.is_empty(),
        "text fields without a case: {missing:?}"
    );
}

const LIFECYCLE_ID: &str = "1a1000";

fn lifecycle_seed(status: billet::issue::Status) -> billet::issue::Issue {
    use billet::issue::Status;
    let active_claim = matches!(status, Status::Claimed | Status::Review);
    let closed = status == Status::Closed;
    billet::issue::Issue {
        id: LIFECYCLE_ID.into(),
        number: Some(number_of(LIFECYCLE_ID)),
        title: "seeded".into(),
        status,
        priority: 1,
        parent: None,
        depends_on: Vec::new(),
        assignee: (active_claim || closed).then(|| "bot".into()),
        defer_until: None,
        created_at: Some("2026-01-01T00:00:00Z".into()),
        updated_at: Some("2026-01-01T00:00:00Z".into()),
        closed_at: closed.then(|| "2026-01-01T00:00:00Z".into()),
        release: closed.then(|| "1.0".into()),
        claim_id: active_claim.then(|| "c1".into()),
        body: "body\n\n```\n## Notes\n```".into(),
        history: "- 2026-01-01 00:00:00 UTC seeded".into(),
        notes: "- 2026-01-01 00:00:00 UTC n".into(),
        closing_summary: closed.then(|| "sum1\n\nsum2".into()),
    }
}

fn place_of(status: billet::issue::Status) -> &'static str {
    if status == billet::issue::Status::Closed {
        "closed"
    } else {
        "open"
    }
}

struct Transition {
    stdout: &'static str,
    entry: String,
    change: fn(&mut billet::issue::Issue),
}

fn lifecycle_expect(argv: &[&str], from: billet::issue::Status) -> Result<Transition, String> {
    use billet::issue::Status;
    let name = from.as_str();
    let lifecycle = h(LIFECYCLE_ID);
    let refuse = |verb: &str| Err(format!("issue {lifecycle} is {name} and cannot {verb}"));
    let reason = argv
        .iter()
        .position(|a| *a == "--reason")
        .map(|i| argv[i + 1]);
    let quoted = |label: &str, text: &str| -> String {
        if text.contains('\n') {
            let lines: String = text.split('\n').map(|l| format!("\n  > {l}")).collect();
            format!("{label}{lines}")
        } else {
            format!("{label} {text}")
        }
    };
    let ok = |stdout, entry: String, change| {
        Ok(Transition {
            stdout,
            entry,
            change,
        })
    };
    match (argv[0], from) {
        ("approve", Status::Proposed) => ok("approved ID", "approved".into(), |i| {
            i.status = Status::Open
        }),
        ("approve", _) => refuse("approve"),
        ("revise", Status::Open) => ok(
            "returned ID to proposed",
            quoted("revised:", reason.unwrap()),
            |i| i.status = Status::Proposed,
        ),
        ("revise", _) => refuse("revise"),
        ("claim", Status::Open) => ok("claimed ID as bot", String::new(), |i| {
            i.status = Status::Claimed;
            i.assignee = Some("bot".into());
            i.claim_id = Some("new".into());
        }),
        ("claim", Status::Proposed) => Err(format!(
            "issue {lifecycle} awaits approval; is not ready: status proposed"
        )),
        ("claim", _) => Err(format!("issue {lifecycle} is not ready: status {name}")),
        ("unclaim", Status::Claimed) => ok("unclaimed ID", String::new(), |i| {
            i.status = Status::Open;
            i.assignee = None;
            i.claim_id = None;
        }),
        ("unclaim", _) => refuse("unclaim"),
        ("review", Status::Claimed) => {
            let mut entry = "submitted for review by bot".to_string();
            if let Some(i) = argv.iter().position(|a| *a == "--summary") {
                entry.push_str("\n  ");
                entry.push_str(&quoted("summary:", argv[i + 1]));
            }
            ok("submitted ID for review", entry, |i| {
                i.status = Status::Review
            })
        }
        ("review", _) => refuse("review"),
        ("rework", Status::Review) => ok(
            "returned ID for rework",
            quoted("rework requested:", reason.unwrap()),
            |i| i.status = Status::Claimed,
        ),
        ("rework", _) => refuse("rework"),
        ("close", Status::Closed) => Err(format!("issue {lifecycle} is already closed")),
        ("close", _) => {
            let change: fn(&mut billet::issue::Issue) = if argv.contains(&"--summary") {
                |i| {
                    i.status = Status::Closed;
                    i.claim_id = None;
                    i.closing_summary = Some("s1\ns2".into());
                }
            } else {
                |i| {
                    i.status = Status::Closed;
                    i.claim_id = None;
                }
            };
            ok("closed ID", quoted("closed:", reason.unwrap()), change)
        }
        ("reopen", Status::Closed) => ok(
            "reopened ID",
            format!(
                "{}\n  closing summary:\n  > sum1\n  > \n  > sum2\n  release: 1.0",
                quoted("reopened:", reason.unwrap())
            ),
            |i| {
                i.status = Status::Open;
                i.assignee = None;
                i.closing_summary = None;
                i.release = None;
            },
        ),
        ("reopen", _) => Err(format!("issue {lifecycle} is already open")),
        ("update", _) if argv.contains(&"-p") => {
            ok("updated ID", String::new(), |i| i.priority = 0)
        }
        ("update", Status::Closed) if argv.contains(&"1.0") => {
            ok("updated ID", String::new(), |_| {})
        }
        ("update", Status::Closed) => ok("updated ID", "release: 1.0 -> 2.0".into(), |i| {
            i.release = Some("2.0".into())
        }),
        ("update", _) => Err(format!("issue {lifecycle} is not closed")),
        ("defer", Status::Closed) if argv.contains(&"--until") => {
            Err(format!("issue {lifecycle} is closed"))
        }
        ("defer", _) if argv.contains(&"--until") => {
            ok("deferred ID until 2999-01-01", String::new(), |i| {
                i.defer_until = Some("2999-01-01".into())
            })
        }
        ("defer", _) => ok("cleared deferral on ID", String::new(), |i| {
            i.defer_until = None
        }),
        ("note", _) => ok("noted ID", String::new(), |_| {}),
        _ => panic!("no expectation for {argv:?}"),
    }
}

fn without_stamps(text: &str) -> String {
    text.lines()
        .map(|line| match line.strip_prefix("- ") {
            Some(rest) if rest.len() > 24 && rest.as_bytes()[19..24] == *b" UTC " => {
                format!("- <stamp> {}", &rest[24..])
            }
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn every_status() -> [billet::issue::Status; 5] {
    use billet::issue::Status;
    let all = [
        Status::Proposed,
        Status::Open,
        Status::Claimed,
        Status::Review,
        Status::Closed,
    ];
    for status in all {
        match status {
            Status::Proposed | Status::Open | Status::Claimed | Status::Review | Status::Closed => {
            }
        }
    }
    all
}

fn check_lifecycle_action(root: &Path, index: usize, argv: &[&str]) {
    use billet::issue::Status;
    for from in every_status() {
        let d = root.join(format!("a{index}-{}", from.as_str()));
        copy_tree(&root.join("base"), &d);
        let seed = lifecycle_seed(from);
        let path = d.join(format!(".blt/{}/{LIFECYCLE_ID}.md", place_of(from)));
        fs::write(&path, billet::issue::encode(&seed).unwrap()).unwrap();
        let mut args = vec![argv[0], LIFECYCLE_ID];
        args.extend(&argv[1..]);
        let label = format!("{} from {}", argv.join(" "), from.as_str());
        let before = tree_bytes(&d.join(".blt"));
        let o = run(&d, &args);
        let stdout = String::from_utf8_lossy(&o.stdout).replace(&h(LIFECYCLE_ID), "ID");
        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
        match lifecycle_expect(argv, from) {
            Err(message) => {
                assert_eq!(o.status.code(), Some(1), "{label}: {stderr}");
                assert_eq!(stderr, format!("blt: {message}\n"), "{label}");
                assert_eq!(
                    tree_bytes(&d.join(".blt")),
                    before,
                    "{label} refusal changed .blt"
                );
            }
            Ok(t) => {
                assert!(o.status.success(), "{label}: {stderr}");
                assert_eq!(stdout, format!("{}\n", t.stdout), "{label} stdout");
                if t.entry.is_empty()
                    && argv[0] == "update"
                    && from == Status::Closed
                    && argv.contains(&"1.0")
                {
                    assert_eq!(
                        tree_bytes(&d.join(".blt")),
                        before,
                        "{label} same release is a no-op"
                    );
                }
                let mut expected = seed.clone();
                (t.change)(&mut expected);
                if !t.entry.is_empty() {
                    expected
                        .history
                        .push_str(&format!("\n- <stamp> {}", t.entry));
                }
                if argv[0] == "note" {
                    expected.notes.push_str("\n- <stamp> qa: n");
                }
                let after = d.join(format!(
                    ".blt/{}/{LIFECYCLE_ID}.md",
                    place_of(expected.status)
                ));
                if place_of(expected.status) != place_of(from) {
                    assert!(!path.exists(), "{label} left the old file");
                }
                let text = fs::read_to_string(&after).unwrap();
                let mut actual = billet::issue::decode(&text).unwrap();
                assert_eq!(
                    without_stamps(&actual.history),
                    without_stamps(&expected.history),
                    "{label} history"
                );
                assert_eq!(
                    without_stamps(&actual.notes),
                    without_stamps(&expected.notes),
                    "{label} notes"
                );
                assert_eq!(
                    actual.claim_id.is_some(),
                    expected.claim_id.is_some(),
                    "{label} claim_id"
                );
                assert_eq!(
                    actual.closed_at.is_some(),
                    expected.status == Status::Closed,
                    "{label} closed_at"
                );
                if let Some(summary) = &actual.closing_summary {
                    assert!(
                        text.find("## Closing Summary").unwrap() < text.find("body\n").unwrap(),
                        "{label} closing summary precedes the body"
                    );
                    assert_eq!(Some(summary), expected.closing_summary.as_ref(), "{label}");
                }
                for issue in [&mut actual, &mut expected] {
                    issue.history.clear();
                    issue.notes.clear();
                    issue.claim_id = None;
                    issue.closed_at = None;
                    issue.updated_at = None;
                }
                assert_eq!(actual, expected, "{label} fields");
                assert_eq!(
                    ok(&d, &["--json", "check"]),
                    "[]\n",
                    "{label} left a problem"
                );
            }
        }
        fs::remove_dir_all(&d).unwrap();
    }
}

#[test]
fn lifecycle_transition_matrix() {
    let root = dir("lifecycle");
    fs::create_dir(root.join("base")).unwrap();
    ok(&root.join("base"), &["init", "--no-hooks"]);
    let actions: &[&[&str]] = &[
        &["approve"],
        &["revise", "--reason", "r"],
        &["revise", "--reason", "r1\nr2"],
        &["claim", "--by", "bot"],
        &["claim", "--by", "bot", "--force"],
        &["unclaim"],
        &["review"],
        &["review", "--summary", "s1\ns2"],
        &["rework", "--reason", "r"],
        &["rework", "--reason", "r1\n\nr2"],
        &["close", "--reason", "r"],
        &["close", "--reason", "r1\nr2", "--summary", "s1\ns2"],
        &["reopen", "--reason", "r"],
        &["reopen", "--reason", "first line\nsecond line"],
        &["update", "-p", "0"],
        &["update", "--release", "2.0"],
        &["update", "--release", "1.0"],
        &["defer", "--until", "2999-01-01"],
        &["defer", "--clear"],
        &["note", "n", "--by", "qa"],
    ];
    let writes: Vec<&str> = billet::commands::COMMANDS
        .iter()
        .filter(|c| !billet::commands::JSON_CMDS.contains(&c.name) && c.name != "dep")
        .filter(|c| {
            c.args.iter().any(|a| {
                a.positional
                    && matches!(
                        a.kind,
                        billet::commands::ArgType::Text(billet::text::TextKind::Id)
                    )
            })
        })
        .map(|c| c.name)
        .collect();
    let missing: Vec<&&str> = writes
        .iter()
        .filter(|name| !actions.iter().any(|a| a[0] == **name))
        .collect();
    assert!(
        missing.is_empty(),
        "write commands without a lifecycle row: {missing:?}"
    );
    std::thread::scope(|scope| {
        for (index, argv) in actions.iter().enumerate() {
            let root = &root;
            scope.spawn(move || check_lifecycle_action(root, index, argv));
        }
        scope.spawn(|| lifecycle_lists_and_release(&root.join("lists")));
    });
}

fn lifecycle_lists_and_release(d: &Path) {
    fs::create_dir(d).unwrap();
    ok(d, &["init", "--no-hooks"]);
    let proposed = ok(d, &["add", "proposed"]).trim().to_string();
    let file =
        fs::read_to_string(d.join(format!(".blt/open/{}.md", key_of(d, &proposed)))).unwrap();
    assert!(
        file.lines().any(|l| l == "priority: 2"),
        "add without -p stores 2: {file}"
    );
    assert_eq!(show_value(d, &proposed)["priority"], json!(2));
    let open = add(d, "open");
    let claimed = add(d, "claimed");
    ok(d, &["claim", &claimed]);
    let review = add(d, "review");
    ok(d, &["claim", &review]);
    ok(d, &["review", &review]);
    let first = add(d, "first");
    let second = add(d, "second");
    ok(d, &["close", &first, "--reason", "done"]);
    ok(d, &["close", &second, "--reason", "done"]);
    let ids_of = |args: &[&str]| -> Vec<String> {
        let j: Value = serde_json::from_str(&ok(d, args)).unwrap();
        let mut ids: Vec<String> = j
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    let sorted = |mut ids: Vec<&String>| {
        ids.sort();
        ids.into_iter().cloned().collect::<Vec<String>>()
    };
    for (status, ids) in [
        ("proposed", vec![&proposed]),
        ("open", vec![&open]),
        ("claimed", vec![&claimed]),
        ("review", vec![&review]),
        ("closed", vec![&first, &second]),
        (
            "all",
            vec![&proposed, &open, &claimed, &review, &first, &second],
        ),
    ] {
        assert_eq!(
            ids_of(&["--json", "list", "--status", status]),
            sorted(ids),
            "--status {status}"
        );
    }
    assert_eq!(
        ids_of(&["--json", "list"]),
        sorted(vec![&proposed, &open, &claimed, &review]),
        "the default list is every active issue, proposals included"
    );
    let e = run(d, &["list", "--status", "in_progress"]);
    assert_eq!(e.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&e.stderr).contains("invalid status filter"));

    let unreadable = add(d, "unreadable");
    ok(d, &["close", &unreadable, "--reason", "done"]);
    let unreadable = key_of(d, &unreadable);
    let unreadable_path = d.join(format!(".blt/closed/{unreadable}.md"));
    fs::write(&unreadable_path, "not an issue file").unwrap();
    let inconsistent = add(d, "inconsistent");
    ok(d, &["close", &inconsistent, "--reason", "done"]);
    let inconsistent = key_of(d, &inconsistent);
    let inconsistent_path = d.join(format!(".blt/closed/{inconsistent}.md"));
    let text = fs::read_to_string(&inconsistent_path)
        .unwrap()
        .replace("priority: 2", "priority: 9");
    fs::write(&inconsistent_path, text).unwrap();
    let damaged = [
        fs::read(&unreadable_path).unwrap(),
        fs::read(&inconsistent_path).unwrap(),
    ];

    let sealed = run(d, &["release", "1.0"]);
    assert_eq!(sealed.status.code(), Some(0));
    let mut expected = [
        (key_of(d, &first), first.clone()),
        (key_of(d, &second), second.clone()),
    ];
    expected.sort();
    let expected: Vec<String> = expected.into_iter().map(|(_, handle)| handle).collect();
    assert_eq!(
        String::from_utf8_lossy(&sealed.stdout),
        format!("{}\n", expected.join("\n")),
        "sealed issues in file order"
    );
    let warnings = String::from_utf8_lossy(&sealed.stderr);
    for id in [&unreadable, &inconsistent] {
        assert_eq!(
            warnings.matches(&format!(".blt/closed/{id}.md")).count(),
            1,
            "{warnings}"
        );
    }
    assert!(warnings.contains("blt check"), "{warnings}");
    assert_eq!(
        [
            fs::read(&unreadable_path).unwrap(),
            fs::read(&inconsistent_path).unwrap()
        ],
        damaged,
        "release leaves damaged files untouched"
    );
    assert_eq!(ok(d, &["release", "1.0"]), "nothing sealed\n");
    assert_eq!(show_value(d, &first)["release"], "1.0");
    let both = sorted(vec![&first, &second]);
    assert_eq!(ids_of(&["--json", "list", "--release", "1.0"]), both);
    assert_eq!(
        ids_of(&["--json", "list", "--release", "1.0", "--status", "all"]),
        both
    );
    assert_eq!(
        ids_of(&["--json", "list", "--release", "1.0", "--status", "closed"]),
        both
    );
    let e = run(d, &["list", "--release", "1.0", "--status", "open"]);
    assert_eq!(e.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&e.stderr),
        "blt: --release requires --status closed or all\n"
    );
    let third = add(d, "third");
    ok(d, &["close", &third, "--reason", "done"]);
    assert_eq!(ok(d, &["release", "1.1"]), format!("{third}\n"));
    assert_eq!(ids_of(&["--json", "list", "--release", "1.0"]), both);
}

const LATER: &str = "2999-01-01";

struct Row {
    name: &'static str,
    id: String,
    reasons: String,
    readiness: Value,
    forced: Option<String>,
}

fn row(
    name: &'static str,
    id: &str,
    reasons: String,
    readiness: Value,
    forced: Option<String>,
) -> Row {
    Row {
        name,
        id: id.to_string(),
        reasons,
        readiness,
        forced,
    }
}

fn readiness_of(
    deferred: Option<(&str, Option<&str>)>,
    unresolved: &[String],
    blockers: &[&str],
    subtasks: &[&str],
) -> Value {
    json!({
        "deferred_until": deferred.map(|(until, _)| until),
        "deferred_by": deferred.and_then(|(_, by)| by),
        "unresolved": unresolved,
        "blockers": blockers,
        "subtasks": subtasks,
    })
}

fn child(d: &Path, parent: &str, title: &str, approved: bool) -> String {
    let mut args = vec!["add", title, "--parent", parent];
    if approved {
        args.push("--approved");
    }
    ok(d, &args).trim().to_string()
}

fn seed_file(d: &Path, id: &str, status: &str, edit: impl Fn(String) -> String) {
    let mut text = fm(id, status, "");
    if matches!(status, "claimed" | "review") {
        text = text
            .replace("assignee: null", "assignee: \"bot\"")
            .replace("claim_id: null", "claim_id: \"c1\"");
    }
    fs::write(d.join(format!(".blt/open/{id}.md")), edit(text)).unwrap();
}

fn readiness_rows_status(d: &Path) -> Vec<Row> {
    let none = || readiness_of(None, &[], &[], &[]);
    let ready = |name, id: String| row(name, &id, String::new(), none(), Some(String::new()));
    let status = |name, id: String, s: &str| row(name, &id, format!("status {s}"), none(), None);
    let mut rows = Vec::new();

    rows.push(ready("ready", add(d, "ready")));
    let proposed = ok(d, &["add", "proposed"]).trim().to_string();
    rows.push(status("proposed", proposed, "proposed"));
    let claimed = add(d, "claimed");
    ok(d, &["claim", &claimed]);
    rows.push(status("claimed", claimed, "claimed"));
    let review = add(d, "review");
    ok(d, &["claim", &review]);
    ok(d, &["review", &review]);
    rows.push(status("review", review, "review"));
    let closed = add(d, "closed");
    ok(d, &["close", &closed, "--reason", "done"]);
    rows.push(status("closed", closed, "closed"));

    rows
}

fn readiness_rows_deferred(d: &Path) -> Vec<Row> {
    let none = || readiness_of(None, &[], &[], &[]);
    let ready = |name, id: String| row(name, &id, String::new(), none(), Some(String::new()));
    let mut rows = Vec::new();
    let deferred = add(d, "deferred");
    ok(d, &["defer", &deferred, "--until", LATER]);
    rows.push(row(
        "deferred",
        &deferred,
        format!("deferred until {LATER}"),
        readiness_of(Some((LATER, None)), &[], &[], &[]),
        None,
    ));
    let today = add(d, "deferred until today");
    ok(d, &["defer", &today, "--until", &billet::clock::today()]);
    rows.push(ready("deferred until today", today));
    let parent = add(d, "deferred parent");
    let kid = child(d, &parent, "child", true);
    let middle = child(d, &parent, "middle", true);
    let grandkid = child(d, &middle, "grandchild", true);
    ok(d, &["defer", &parent, "--until", LATER]);
    for (name, id) in [
        ("deferred by ancestor", &kid),
        ("deferred by grandparent", &grandkid),
    ] {
        rows.push(row(
            name,
            id,
            format!("deferred until {LATER} by ancestor {parent}"),
            readiness_of(Some((LATER, Some(&parent))), &[], &[], &[]),
            None,
        ));
    }
    let gone = add(d, "closed deferred parent");
    let orphan = child(d, &gone, "child", true);
    ok(d, &["defer", &gone, "--until", LATER]);
    ok(d, &["close", &gone, "--reason", "cancelled"]);
    rows.push(ready("closed deferred ancestor", orphan));

    rows
}

fn readiness_rows_blockers(d: &Path) -> Vec<Row> {
    let none = || readiness_of(None, &[], &[], &[]);
    let ready = |name, id: String| row(name, &id, String::new(), none(), Some(String::new()));
    let mut rows = Vec::new();
    let blocker = add(d, "blocker");
    let blocked = add(d, "blocked");
    ok(d, &["dep", "add", &blocked, &blocker]);
    rows.push(row(
        "blocker",
        &blocked,
        format!("blocked by {blocker}"),
        readiness_of(None, &[], &[&blocker], &[]),
        Some(format!("blockers {blocker}")),
    ));
    let unfinished = ["1f0000", "1f0001", "1f0002"];
    for (id, status) in unfinished.iter().zip(["proposed", "claimed", "review"]) {
        seed_file(d, id, status, |t| t);
    }
    let waits = add(d, "waits for unfinished blockers");
    for id in unfinished {
        ok(d, &["dep", "add", &waits, id]);
    }
    rows.push(row(
        "blockers in every unfinished status",
        &waits,
        format!("blocked by {}", unfinished.join(", ")),
        readiness_of(None, &[], &unfinished, &[]),
        Some(format!("blockers {}", unfinished.join(", "))),
    ));
    let done = add(d, "closed blocker");
    let after_done = add(d, "after closed blocker");
    ok(d, &["dep", "add", &after_done, &done]);
    ok(d, &["close", &done, "--reason", "done"]);
    assert_eq!(
        show_value(d, &after_done)["depends_on"],
        json!([done]),
        "a closed blocker stays listed"
    );
    rows.push(ready("closed blocker", after_done));
    let again = add(d, "reopened blocker");
    let after_again = add(d, "after reopened blocker");
    ok(d, &["dep", "add", &after_again, &again]);
    ok(d, &["close", &again, "--reason", "done"]);
    ok(d, &["reopen", &again, "--reason", "regression"]);
    rows.push(row(
        "reopened blocker",
        &after_again,
        format!("blocked by {again}"),
        readiness_of(None, &[], &[&again], &[]),
        Some(format!("blockers {again}")),
    ));
    seed_file(d, "1bad0f", "open", |t| {
        t.replace("claim_id: null", "claim_id: \"c\"")
    });
    seed_file(d, "1de002", "open", |t| {
        t.replace("depends_on: []", "depends_on: [\"1bad0f\"]")
    });
    rows.push(row(
        "inconsistent blocker is an ordinary blocker",
        "1de002",
        "blocked by 1bad0f".into(),
        readiness_of(None, &[], &["1bad0f"], &[]),
        Some("blockers 1bad0f".into()),
    ));

    rows
}

fn readiness_rows_subtasks(d: &Path) -> Vec<Row> {
    let none = || readiness_of(None, &[], &[], &[]);
    let status = |name, id: String, s: &str| row(name, &id, format!("status {s}"), none(), None);
    let mut rows = Vec::new();
    let proposed_parent = ok(d, &["add", "proposed parent"]).trim().to_string();
    let approved_sub = child(d, &proposed_parent, "approved child", true);
    rows.push(row(
        "open subtask of a proposed parent",
        &approved_sub,
        String::new(),
        none(),
        Some(String::new()),
    ));
    let parent = ok(d, &["add", "parent"]).trim().to_string();
    let sub = child(d, &parent, "child", false);
    ok(d, &["approve", &parent]);
    assert_eq!(
        show_value(d, &sub)["status"],
        "proposed",
        "approve must not cascade"
    );
    rows.push(row(
        "subtask",
        &parent,
        format!("waiting for subtasks {sub}"),
        readiness_of(None, &[], &[], &[&sub]),
        Some(format!("subtasks {sub}")),
    ));
    rows.push(status(
        "proposed subtask of an approved parent",
        sub,
        "proposed",
    ));
    let all = add(d, "parent of every active status");
    let subs: Vec<String> = ["proposed", "open", "claimed", "review", "nested"]
        .iter()
        .map(|s| child(d, &all, s, *s != "proposed"))
        .collect();
    ok(d, &["claim", &subs[2]]);
    ok(d, &["claim", &subs[3]]);
    ok(d, &["review", &subs[3]]);
    let deep = child(d, &subs[4], "grandchild", true);
    let mut waiting: Vec<&str> = subs.iter().map(String::as_str).collect();
    waiting.push(&deep);
    rows.push(row(
        "subtasks in every active status",
        &all,
        format!("waiting for subtasks {}", waiting.join(", ")),
        readiness_of(None, &[], &[], &waiting),
        Some(format!("subtasks {}", waiting.join(", "))),
    ));
    let both = add(d, "deferred, blocked and waiting");
    let both_blocker = add(d, "its blocker");
    let both_child = child(d, &both, "its child", false);
    ok(d, &["dep", "add", &both, &both_blocker]);
    ok(d, &["defer", &both, "--until", LATER]);
    rows.push(row(
        "deferred, blocked and waiting",
        &both,
        format!(
            "deferred until {LATER}; blocked by {both_blocker}; waiting for subtasks {both_child}"
        ),
        readiness_of(Some((LATER, None)), &[], &[&both_blocker], &[&both_child]),
        None,
    ));

    rows
}

fn readiness_rows_unresolved(d: &Path) -> Vec<Row> {
    let unresolved =
        |name, id: &str, reason: String, blockers: &[&str], subtasks: &[&str], extra: &str| {
            let readiness = readiness_of(None, std::slice::from_ref(&reason), blockers, subtasks);
            row(
                name,
                id,
                format!("unresolved {reason}{extra}"),
                readiness,
                None,
            )
        };
    let mut rows = Vec::new();
    for id in ["1a0001", "1a0002"] {
        seed_file(d, id, "open", |t| {
            t.replace("depends_on: []", "depends_on: [\"1dead0\"]")
        });
        rows.push(unresolved(
            "missing blocker shared by two issues",
            id,
            "blocker 1dead0 (missing)".into(),
            &[],
            &[],
            "",
        ));
    }
    seed_file(d, "1ca000.1", "open", |t| {
        t.replace("parent: null", "parent: \"1ca000\"")
    });
    rows.push(unresolved(
        "missing ancestor",
        "1ca000.1",
        "ancestor 1ca000 (missing)".into(),
        &[],
        &[],
        "",
    ));
    let bad_blocker = add(d, "unusable blocker");
    let behind = add(d, "behind unusable blocker");
    ok(d, &["dep", "add", &behind, &bad_blocker]);
    let bad_blocker = key_of(d, &bad_blocker);
    set_priority_null(d, &bad_blocker);
    rows.push(unresolved(
        "unusable blocker",
        &behind,
        format!("blocker {bad_blocker} (unusable .blt/open/{bad_blocker}.md)"),
        &[],
        &[],
        "",
    ));
    let host = add(d, "parent of invalid subtask");
    let invalid = key_of(d, &child(d, &host, "child", true));
    set_priority_null(d, &invalid);
    rows.push(unresolved(
        "subtask with an invalid field",
        &host,
        format!("subtask .blt/open/{invalid}.md (unusable)"),
        &[],
        &[],
        "",
    ));
    let host = add(d, "parent of unparseable subtask");
    let broken = key_of(d, &child(d, &host, "child", true));
    fs::write(d.join(format!(".blt/open/{broken}.md")), "not parseable").unwrap();
    rows.push(unresolved(
        "unparseable subtask",
        &host,
        format!("subtask .blt/open/{broken}.md (unusable)"),
        &[],
        &[],
        "",
    ));
    let host = add(d, "grandparent of unparseable");
    let middle = child(d, &host, "child", true);
    let leaf = key_of(d, &child(d, &middle, "grandchild", true));
    fs::write(d.join(format!(".blt/open/{leaf}.md")), "not parseable").unwrap();
    rows.push(unresolved(
        "unparseable grandchild",
        &host,
        format!("subtask .blt/open/{leaf}.md (unusable)"),
        &[],
        &[&middle],
        &format!("; waiting for subtasks {middle}"),
    ));
    rows
}

fn check_readiness_row(base: &Path, scratch: &Path, row: &Row, listed: &[Value], warnings: &str) {
    let name = row.name;
    let shown_id = render_keys(base, &row.id);
    let reasons = render_keys(base, &row.reasons);
    let readiness = render_json_keys(base, &row.readiness);
    let unresolved = !readiness["unresolved"].as_array().unwrap().is_empty();
    let line = match (reasons.is_empty(), unresolved) {
        (true, _) => "Ready: yes".to_string(),
        (false, true) => format!("Ready: no — {reasons} (run `blt check`)"),
        (false, false) => format!("Ready: no — {reasons}"),
    };
    let shown = ok(base, &["show", &row.id]);
    let lines: Vec<&str> = shown.lines().filter(|l| l.starts_with("Ready:")).collect();
    assert_eq!(lines, [line.as_str()], "{name}");
    let json: Value = serde_json::from_str(&ok(base, &["--json", "show", &row.id])).unwrap();
    let mut expected = readiness.clone();
    expected["ready"] = json!(reasons.is_empty());
    assert_eq!(json["readiness"], expected, "{name}");
    assert_eq!(
        listed.iter().any(|r| r["id"] == shown_id.as_str()),
        reasons.is_empty(),
        "{name}: ready membership"
    );
    assert_eq!(
        warnings
            .matches(&format!("skipping {shown_id}: unresolved "))
            .count(),
        usize::from(unresolved),
        "{name}: one warning per unresolved issue\n{warnings}"
    );

    let d = scratch.join(row.id.replace('.', "_"));
    copy_tree(base, &d);
    let prefix = if reasons == "status proposed" {
        "awaits approval; "
    } else {
        ""
    };
    let suffix = if unresolved { " (run `blt check`)" } else { "" };
    let refusal = format!("blt: issue {shown_id} {prefix}is not ready: {reasons}{suffix}\n");
    let claim = run(&d, &["claim", &row.id]);
    if reasons.is_empty() {
        assert!(
            claim.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&claim.stderr)
        );
    } else {
        assert_eq!(
            String::from_utf8_lossy(&claim.stderr),
            refusal,
            "{name}: claim"
        );
        let forced = run(&d, &["claim", &row.id, "--force", "--by", "bot"]);
        match &row.forced {
            None => assert_eq!(
                String::from_utf8_lossy(&forced.stderr),
                refusal,
                "{name}: --force"
            ),
            Some(bypassed) => {
                assert!(
                    forced.status.success(),
                    "{name}: {}",
                    String::from_utf8_lossy(&forced.stderr)
                );
                let history = show_value(&d, &row.id)["history"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let last = history.lines().last().unwrap();
                assert_eq!(
                    last.splitn(5, ' ').nth(4).unwrap(),
                    format!(
                        "claimed --force by bot bypassing {}",
                        render_keys(base, bypassed)
                    ),
                    "{name}: --force history"
                );
            }
        }
    }
}

fn check_readiness_group(root: &Path, name: &str, build: fn(&Path) -> Vec<Row>) -> Vec<Value> {
    let base = root.join(name);
    fs::create_dir(&base).unwrap();
    ok(&base, &["init", "--no-hooks"]);
    let rows = build(&base);
    let listed = run(&base, &["--json", "ready"]);
    assert!(listed.status.success(), "{name}");
    let rows_listed: Vec<Value> = serde_json::from_slice(&listed.stdout).unwrap();
    let warnings = String::from_utf8_lossy(&listed.stderr).to_string();
    let scratch = root.join(format!("{name}-claims"));
    fs::create_dir(&scratch).unwrap();
    std::thread::scope(|scope| {
        for row in &rows {
            let (base, scratch, rows_listed, warnings) = (&base, &scratch, &rows_listed, &warnings);
            scope.spawn(move || check_readiness_row(base, scratch, row, rows_listed, warnings));
        }
    });
    if name == "blockers" {
        assert_eq!(
            warnings.lines().filter(|l| l.contains("1bad0f.md")).count(),
            1,
            "one file warning for the inconsistent record\n{warnings}"
        );
    }
    rows.iter().map(|row| row.readiness.clone()).collect()
}

#[test]
fn readiness_table() {
    let root = dir("readiness");
    type Rows = fn(&Path) -> Vec<Row>;
    let groups: [(&str, Rows); 5] = [
        ("status", readiness_rows_status),
        ("deferred", readiness_rows_deferred),
        ("blockers", readiness_rows_blockers),
        ("subtasks", readiness_rows_subtasks),
        ("unresolved", readiness_rows_unresolved),
    ];
    let readiness: Vec<Value> = std::thread::scope(|scope| {
        let workers: Vec<_> = groups
            .into_iter()
            .map(|(name, build)| {
                let root = &root;
                scope.spawn(move || check_readiness_group(root, name, build))
            })
            .collect();
        scope.spawn(|| dependency_edits(&root.join("deps")));
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    let probe = root.join("fields");
    fs::create_dir(&probe).unwrap();
    ok(&probe, &["init", "--no-hooks"]);
    let id = add(&probe, "probe");
    let shown = show_value(&probe, &id);
    for key in shown["readiness"].as_object().unwrap().keys() {
        let exercised = readiness.iter().any(|r| match &r[key.as_str()] {
            Value::Null => false,
            Value::Array(items) => !items.is_empty(),
            Value::Bool(_) => true,
            _ => true,
        });
        assert!(
            key == "ready" || exercised,
            "readiness field {key} has no row that sets it"
        );
    }
}

fn dependency_edits(d: &Path) {
    fs::create_dir(d).unwrap();
    ok(d, &["init", "--no-hooks"]);
    let [a, b, c, x, y] = ["a", "b", "c", "x", "y"].map(|t| add(d, t));
    let refuse = |args: &[&str], message: String| {
        let before = tree_bytes(&d.join(".blt"));
        let o = run(d, args);
        assert_eq!(
            String::from_utf8_lossy(&o.stderr),
            format!("blt: {message}\n"),
            "{args:?}"
        );
        assert_eq!(tree_bytes(&d.join(".blt")), before, "{args:?} changed .blt");
    };
    assert_eq!(
        ok(d, &["dep", "add", &a, &b]),
        format!("{a} now blocked by {b}\n")
    );
    refuse(&["dep", "add", &b, &a], format!("dependency cycle via {a}"));
    ok(d, &["dep", "add", &c, &a]);
    refuse(&["dep", "add", &b, &c], format!("dependency cycle via {c}"));
    ok(d, &["dep", "add", &x, &y]);
    ok(d, &["close", &x, "--reason", "done"]);
    refuse(&["dep", "add", &y, &x], format!("dependency cycle via {x}"));
    refuse(
        &["dep", "add", &a, "1e0000"],
        "issue 1e0000 not found".into(),
    );
    refuse(
        &["dep", "add", &a, &a],
        "issue cannot depend on itself".into(),
    );
    refuse(
        &["dep", "rm", &a, &c],
        format!("{a} does not depend on {c}"),
    );
    let removed = ok(d, &["dep", "rm", &a, &b]);
    assert!(removed.contains(&a) && removed.contains(&b), "{removed}");
    for args in [
        vec!["defer", &a],
        vec!["defer", &a, "--until", LATER, "--clear"],
    ] {
        let o = run(d, &args);
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(
            String::from_utf8_lossy(&o.stderr),
            "blt: defer requires exactly one of --until or --clear\n"
        );
    }

    ok(d, &["dep", "add", &a, &y]);
    let y_key = key_of(d, &y);
    fs::rename(
        d.join(format!(".blt/open/{y_key}.md")),
        d.join("outside.md"),
    )
    .unwrap();
    assert_eq!(
        show_value(d, &a)["readiness"]["unresolved"],
        json!([format!("blocker {y_key} (missing)")]),
        "a missing blocker has no number, so it is shown by its key"
    );
    refuse(&["dep", "rm", &a, &y], format!("issue {y} not found"));
    ok(d, &["dep", "rm", &a, &y_key]);
    assert_eq!(
        show_value(d, &a)["depends_on"],
        json!([]),
        "dep rm tolerates a missing blocker"
    );
    refuse(
        &["dep", "add", &a, &y_key],
        format!("issue {y_key} not found"),
    );
}

const DMG_ROOT: &str = "1a0000";
const DMG_GOOD: &str = "1a0000.1";
const DMG_VICTIM: &str = "1a0000.2";
const DMG_LEAF: &str = "1a0000.2.1";
const DMG_HEALTHY: &str = "1b0000";
const DMG_TWIN: &str = "9d0000";
const DMG_DOUBLE: &str = "9e0000";

fn dmg_text(id: &str, status: &str, parent: Option<&str>) -> String {
    let parent = parent.map_or("null".to_string(), |p| format!("\"{p}\""));
    fm(id, status, "").replace("parent: null", &format!("parent: {parent}"))
}

fn dmg_path(id: &str) -> String {
    format!(".blt/open/{id}.md")
}

#[derive(Clone)]
enum Reaction {
    Skipped(Vec<String>),
    Visible,
    Unresolved(&'static str),
    Refused,
    Unaffected,
    Journal,
    Ambiguous(String),
}

struct Damage {
    name: String,
    apply: Box<dyn Fn(&Path) + Send + Sync>,
    path: String,
    problem: String,
    hint: String,
    reaction: Reaction,
}

fn damage(
    name: impl Into<String>,
    path: impl Into<String>,
    problem: impl Into<String>,
    hint: impl Into<String>,
    reaction: Reaction,
    apply: impl Fn(&Path) + Send + Sync + 'static,
) -> Damage {
    Damage {
        name: name.into(),
        apply: Box::new(apply),
        path: path.into(),
        problem: problem.into(),
        hint: hint.into(),
        reaction,
    }
}

fn victim_with(
    v: &'static str,
    edit: impl Fn(String) -> String + Send + Sync + 'static,
) -> impl Fn(&Path) + Send + Sync {
    move |d: &Path| {
        let path = d.join(dmg_path(v));
        fs::write(&path, edit(fs::read_to_string(&path).unwrap())).unwrap();
    }
}

fn damages(kind: billet::ProblemKind, v: &'static str) -> Result<Vec<Damage>, &'static str> {
    use billet::ProblemKind as K;
    let victim = dmg_path(v);
    let skipped = || Reaction::Skipped(vec![dmg_path(v)]);
    let unreadable = "fix the file or restore it from version control";
    let field = |name: &str, problem: &str, hint: &str, edit: fn(String) -> String| {
        damage(
            name,
            dmg_path(v),
            problem,
            format!("{hint} in {}", dmg_path(v)),
            Reaction::Visible,
            victim_with(v, edit),
        )
    };
    let chmod_dir = |name: &str, rel: &'static str, mode: u32| {
        damage(
            name,
            rel,
            "cannot list directory: ",
            "fix directory permissions",
            Reaction::Unaffected,
            move |d: &Path| set_mode(&d.join(rel), mode),
        )
    };
    Ok(match kind {
        K::IssueDirectory => vec![damage(
            "closed directory is a file",
            ".blt/closed",
            "must be a real directory (not a symbolic link)",
            "restore a real directory at this path",
            Reaction::Refused,
            move |d: &Path| {
                fs::remove_dir_all(d.join(".blt/closed")).unwrap();
                fs::write(d.join(".blt/closed"), "x").unwrap();
            },
        )],
        K::IssueDirectoryUnlistable => vec![chmod_dir(
            "closed directory unreadable",
            ".blt/closed",
            0o000,
        )],
        K::IssueEntryUnlistable => {
            return Err("a directory entry that fails while listing cannot be created on purpose");
        }
        K::IssueNameNotUtf8 => vec![damage(
            "issue file name is not UTF-8",
            ".blt/open/\u{fffd}",
            "unexpected file with a non-UTF-8 name",
            "remove or rename the file",
            Reaction::Unaffected,
            move |d: &Path| write_non_utf8(&d.join(".blt/open")),
        )],
        K::TemporaryFile => vec![damage(
            "abandoned temporary file",
            ".blt/open/.10000a.md.tmp-42-1",
            "abandoned temporary file",
            "remove it after checking no blt process is running",
            Reaction::Unaffected,
            move |d: &Path| fs::write(d.join(".blt/open/.10000a.md.tmp-42-1"), "x").unwrap(),
        )],
        K::UnexpectedFile => vec![damage(
            "unexpected file",
            ".blt/open/notes.txt",
            "unexpected file in an issue directory",
            "remove or rename the file",
            Reaction::Unaffected,
            move |d: &Path| fs::write(d.join(".blt/open/notes.txt"), "x").unwrap(),
        )],
        K::IssueUnreadable => {
            let mut list = vec![
                damage(
                    "invalid UTF-8",
                    &victim,
                    "",
                    unreadable,
                    skipped(),
                    move |d: &Path| fs::write(d.join(dmg_path(v)), b"\xff\xfe garbage").unwrap(),
                ),
                damage(
                    "not an issue",
                    &victim,
                    "frontmatter",
                    unreadable,
                    skipped(),
                    move |d: &Path| fs::write(d.join(dmg_path(v)), "not an issue file").unwrap(),
                ),
                damage(
                    "unknown key",
                    &victim,
                    "unknown frontmatter key",
                    unreadable,
                    skipped(),
                    victim_with(v, |t| t.replace("status: open", "bogus: x\nstatus: open")),
                ),
                damage(
                    "merge conflict",
                    &victim,
                    "merge conflict markers present",
                    unreadable,
                    skipped(),
                    victim_with(v, |t| {
                        t.replace(
                            "## Notes\n",
                            "<<<<<<< A\nx\n=======\ny\n>>>>>>> B\n\n## Notes\n",
                        )
                    }),
                ),
                damage(
                    "missing release key",
                    &victim,
                    "missing frontmatter key \"release\"",
                    unreadable,
                    skipped(),
                    victim_with(v, |t| t.replace("release: null\n", "")),
                ),
                damage(
                    "release with surrounding whitespace",
                    &victim,
                    "release must be a non-empty single line without surrounding whitespace",
                    unreadable,
                    skipped(),
                    victim_with(v, |t| {
                        t.replace("status: open", "status: closed")
                            .replace("release: null", "release: \" 1.0\"")
                    }),
                ),
                damage(
                    "directory instead of a file",
                    &victim,
                    "regular file",
                    unreadable,
                    skipped(),
                    move |d: &Path| {
                        let path = d.join(dmg_path(v));
                        fs::remove_file(&path).unwrap();
                        fs::create_dir(&path).unwrap();
                    },
                ),
            ];
            #[cfg(unix)]
            list.push(damage(
                "symbolic link",
                &victim,
                "symbolic link",
                unreadable,
                skipped(),
                move |d: &Path| {
                    let path = d.join(dmg_path(v));
                    fs::rename(&path, d.join(format!("outside-{v}.md"))).unwrap();
                    std::os::unix::fs::symlink(d.join(format!("outside-{v}.md")), &path).unwrap();
                },
            ));
            list
        }
        K::IssuePriority => {
            let hint = "set the `priority` field to an integer from 0 to 4";
            let mut list: Vec<Damage> = ["null", "5", "-1", "\"2\"", "2.0", "", "\"\"", "[]"]
                .iter()
                .map(|value| {
                    let line = format!("priority: {value}\n");
                    damage(
                        format!("priority {value}"),
                        &victim,
                        "0 to 4",
                        hint,
                        skipped(),
                        victim_with(v, move |t| t.replace("priority: 2\n", &line)),
                    )
                })
                .collect();
            list.push(damage(
                "priority missing",
                &victim,
                "missing frontmatter key \"priority\"",
                hint,
                skipped(),
                victim_with(v, |t| t.replace("priority: 2\n", "")),
            ));
            list
        }
        K::DuplicateId => vec![
            damage(
                "closed copy",
                &victim,
                format!("duplicate issue key {v}"),
                "keep one record for this issue and remove the others",
                Reaction::Skipped(vec![dmg_path(v), format!(".blt/closed/{v}.md")]),
                move |d: &Path| {
                    let text = fs::read_to_string(d.join(dmg_path(v))).unwrap();
                    fs::write(
                        d.join(format!(".blt/closed/{v}.md")),
                        text.replace("status: open", "status: closed"),
                    )
                    .unwrap();
                },
            ),
            damage(
                "closed copy of a top-level issue",
                dmg_path(DMG_DOUBLE),
                format!("duplicate issue key {DMG_DOUBLE}"),
                "keep one record for this issue and remove the others",
                Reaction::Unaffected,
                move |d: &Path| {
                    let text = fm(DMG_DOUBLE, "open", "");
                    fs::write(d.join(dmg_path(DMG_DOUBLE)), &text).unwrap();
                    fs::write(
                        d.join(format!(".blt/closed/{DMG_DOUBLE}.md")),
                        text.replace("status: open", "status: closed"),
                    )
                    .unwrap();
                },
            ),
        ],
        K::FileNameMismatch => vec![damage(
            "id differs from file name",
            &victim,
            "file name does not match issue key 1c0000",
            "rename the file to 1c0000.md or fix the id field",
            skipped(),
            move |d: &Path| {
                fs::write(d.join(dmg_path(v)), dmg_text("1c0000", "open", None)).unwrap()
            },
        )],
        K::WrongDirectory => vec![damage(
            "open issue in closed/",
            format!(".blt/closed/{v}.md"),
            "status open does not belong in closed/",
            "move the file to the other directory or fix the status field",
            Reaction::Skipped(vec![format!(".blt/closed/{v}.md")]),
            move |d: &Path| {
                fs::rename(d.join(dmg_path(v)), d.join(format!(".blt/closed/{v}.md"))).unwrap()
            },
        )],
        K::FieldConflict => vec![
            field(
                "assignee on open",
                "proposed or open issue has an assignee",
                "remove the assignee value",
                |t| t.replace("assignee: null", "assignee: \"x\""),
            ),
            field(
                "claim_id on open",
                "proposed or open issue has a claim_id",
                "remove the claim_id value",
                |t| t.replace("claim_id: null", "claim_id: \"c\""),
            ),
            field(
                "claimed without claim_id",
                "claimed issue has no claim_id",
                "add a claim_id value",
                |t| {
                    t.replace("status: open", "status: claimed")
                        .replace("assignee: null", "assignee: \"bot\"")
                },
            ),
            field(
                "closed_at on active",
                "active issue has closed_at",
                "remove the closed_at value",
                |t| t.replace("closed_at: null", "closed_at: \"2026-01-01\""),
            ),
            field(
                "release on active",
                "active issue has release",
                "remove the release value",
                |t| t.replace("release: null", "release: \"1.0\""),
            ),
            field(
                "closing summary on active",
                "active issue has a closing summary",
                "remove the Closing Summary section",
                |t| t.replace("## History\n", "## Closing Summary\n\n> s\n\n## History\n"),
            ),
        ],
        K::MissingParent => vec![damage(
            "parent missing",
            dmg_path("1f00d0.1"),
            "parent 1f00d0: missing issue 1f00d0",
            format!(
                "restore the issue file for 1f00d0 or edit parent in {}",
                dmg_path("1f00d0.1")
            ),
            Reaction::Unresolved("1f00d0.1"),
            move |d: &Path| {
                fs::write(
                    d.join(dmg_path("1f00d0.1")),
                    dmg_text("1f00d0.1", "open", Some("1f00d0")),
                )
                .unwrap()
            },
        )],
        K::MissingBlocker => vec![damage(
            "blocker missing",
            dmg_path(v),
            "blocker 1dead0: missing issue 1dead0",
            format!("restore the issue file for 1dead0 or run `blt dep rm {v} 1dead0`"),
            Reaction::Unresolved(v),
            move |d: &Path| {
                let path = d.join(dmg_path(v));
                let text = fs::read_to_string(&path).unwrap();
                fs::write(
                    path,
                    text.replace("depends_on: []", "depends_on: [\"1dead0\"]"),
                )
                .unwrap();
            },
        )],
        K::UnresolvedReference => vec![damage(
            "unusable subtask",
            &victim,
            format!("unresolved subtask .blt/open/{DMG_LEAF}.md (unusable)"),
            "fix or restore the referenced issue; see the other reported problems",
            Reaction::Unresolved(v),
            move |d: &Path| fs::write(d.join(dmg_path(DMG_LEAF)), "not an issue").unwrap(),
        )],
        K::DependencyCycle => vec![damage(
            "two-issue cycle",
            dmg_path(DMG_HEALTHY),
            format!("dependency cycle {DMG_HEALTHY} -> 2e0000 -> {DMG_HEALTHY}"),
            format!("remove one edge, e.g. `blt dep rm 2e0000 {DMG_HEALTHY}`"),
            Reaction::Unaffected,
            move |d: &Path| {
                let path = d.join(dmg_path(DMG_HEALTHY));
                let text = fs::read_to_string(&path).unwrap();
                fs::write(
                    path,
                    text.replace("depends_on: []", "depends_on: [\"2e0000\"]"),
                )
                .unwrap();
                fs::write(
                    d.join(dmg_path("2e0000")),
                    dmg_text("2e0000", "open", None).replace(
                        "depends_on: []",
                        &format!("depends_on: [\"{DMG_HEALTHY}\"]"),
                    ),
                )
                .unwrap();
            },
        )],
        K::DocsDirectory => vec![damage(
            "docs is a file",
            ".blt/docs",
            "must be a real directory (not a symbolic link)",
            "restore a real directory at this path or remove it",
            Reaction::Unaffected,
            move |d: &Path| fs::write(d.join(".blt/docs"), "x").unwrap(),
        )],
        K::DocsUnlistable => vec![damage(
            "docs unreadable",
            ".blt/docs",
            "cannot list directory: ",
            "fix directory permissions",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::create_dir_all(d.join(".blt/docs")).unwrap();
                set_mode(&d.join(".blt/docs"), 0o000);
            },
        )],
        K::DocsEntryUnlistable => {
            return Err("a directory entry that fails while listing cannot be created on purpose");
        }
        K::DocsEntryUninspectable => {
            return Err("an entry that disappears between listing and inspection needs a race");
        }
        K::DocsNameNotUtf8 => vec![damage(
            "docs entry name is not UTF-8",
            ".blt/docs/\u{fffd}",
            "name is not valid UTF-8",
            "rename it to the issue ID or remove it",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::create_dir_all(d.join(".blt/docs")).unwrap();
                write_non_utf8(&d.join(".blt/docs"));
            },
        )],
        K::DocsSymlink => vec![damage(
            "docs entry is a symbolic link",
            format!(".blt/docs/{v}"),
            "must be a real directory (not a symbolic link)",
            "rename it to the issue ID or remove it",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::create_dir_all(d.join(".blt/docs")).unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(d.join(".blt/open"), d.join(format!(".blt/docs/{v}")))
                    .unwrap();
            },
        )],
        K::DocsNotDirectory => vec![damage(
            "docs entry is a file",
            format!(".blt/docs/{v}"),
            "entry is not a directory",
            "rename it to the issue ID or remove it",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::create_dir_all(d.join(".blt/docs")).unwrap();
                fs::write(d.join(format!(".blt/docs/{v}")), "x").unwrap();
            },
        )],
        K::DocsOrphan => vec![damage(
            "docs for a missing issue",
            ".blt/docs/1fffff",
            "no issue with id 1fffff exists",
            "rename it to the issue ID or remove it",
            Reaction::Unaffected,
            move |d: &Path| fs::create_dir_all(d.join(".blt/docs/1fffff")).unwrap(),
        )],
        K::ConfigFile => vec![damage(
            "config.json is a directory",
            ".blt/config.json",
            "must be a real regular file (not a symbolic link)",
            "restore .blt/config.json as a regular file",
            Reaction::Refused,
            move |d: &Path| {
                fs::remove_file(d.join(".blt/config.json")).unwrap();
                fs::create_dir(d.join(".blt/config.json")).unwrap();
            },
        )],
        K::ConfigMissing => vec![damage(
            "config.json missing",
            ".blt/config.json",
            "config.json is missing",
            "run `blt init` to write the default configuration",
            Reaction::Refused,
            move |d: &Path| fs::remove_file(d.join(".blt/config.json")).unwrap(),
        )],
        K::ConfigInvalid => vec![damage(
            "invalid prefix",
            ".blt/config.json",
            "invalid prefix \"bad\"",
            "write {\"prefix\": \"B\"} or another valid prefix (use 1 to 8 uppercase letters, optionally followed by '-')",
            Reaction::Refused,
            move |d: &Path| {
                fs::write(d.join(".blt/config.json"), "{\"prefix\": \"bad\"}\n").unwrap()
            },
        )],
        K::DuplicateNumber => vec![damage(
            "copy numbered like the healthy issue",
            dmg_path(DMG_TWIN),
            format!(
                "number {} is also used by {}",
                h(DMG_HEALTHY),
                dmg_path(DMG_HEALTHY)
            ),
            format!(
                "a person gives one of them an unused number: `blt update {DMG_TWIN} --number N`"
            ),
            Reaction::Ambiguous(h(DMG_HEALTHY)),
            move |d: &Path| {
                let twin = fm(DMG_TWIN, "open", "").replace(
                    &format!("number: {}", number_of(DMG_TWIN)),
                    &format!("number: {}", number_of(DMG_HEALTHY)),
                );
                fs::write(d.join(dmg_path(DMG_TWIN)), twin).unwrap();
            },
        )],
        K::MemoryFile => vec![damage(
            "memory.md is a directory",
            ".blt/memory.md",
            "must be a real regular file (not a symbolic link)",
            "restore memory.md as a regular file or run `blt init`",
            Reaction::Refused,
            move |d: &Path| {
                fs::remove_file(d.join(".blt/memory.md")).unwrap();
                fs::create_dir(d.join(".blt/memory.md")).unwrap();
            },
        )],
        K::MemoryMissing => vec![damage(
            "memory.md missing",
            ".blt/memory.md",
            "memory.md is missing",
            "run `blt init` to create it",
            Reaction::Refused,
            move |d: &Path| fs::remove_file(d.join(".blt/memory.md")).unwrap(),
        )],
        K::MemoryUnreadable => vec![damage(
            "memory.md unreadable",
            ".blt/memory.md",
            "cannot read: ",
            "fix file permissions or restore memory.md",
            Reaction::Unaffected,
            move |d: &Path| set_mode(&d.join(".blt/memory.md"), 0o000),
        )],
        K::MemoryMalformed => vec![damage(
            "memory.md malformed",
            ".blt/memory.md",
            "",
            "fix memory.md manually",
            Reaction::Unaffected,
            move |d: &Path| fs::write(d.join(".blt/memory.md"), "junk\n").unwrap(),
        )],
        K::GitignoreFile => vec![damage(
            ".gitignore is a directory",
            ".blt/.gitignore",
            "must be a real regular file (not a symbolic link)",
            "restore .blt/.gitignore as a regular file",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::remove_file(d.join(".blt/.gitignore")).unwrap();
                fs::create_dir(d.join(".blt/.gitignore")).unwrap();
            },
        )],
        K::GitignoreMissing => vec![damage(
            ".gitignore missing",
            ".blt/.gitignore",
            ".gitignore is missing",
            "add the line `.*.tmp-*` to .blt/.gitignore",
            Reaction::Unaffected,
            move |d: &Path| fs::remove_file(d.join(".blt/.gitignore")).unwrap(),
        )],
        K::GitignoreUnreadable => vec![damage(
            ".gitignore unreadable",
            ".blt/.gitignore",
            "cannot read: ",
            "fix file permissions or restore .blt/.gitignore",
            Reaction::Unaffected,
            move |d: &Path| set_mode(&d.join(".blt/.gitignore"), 0o000),
        )],
        K::GitignoreLegacyRule => vec![damage(
            "legacy temporary rule",
            ".blt/.gitignore",
            "legacy `/.*.tmp-*` rule does not ignore nested temporary files",
            "replace `/.*.tmp-*` with `.*.tmp-*` in .blt/.gitignore",
            Reaction::Unaffected,
            move |d: &Path| {
                fs::write(d.join(".blt/.gitignore"), "/.lock\n/.move\n/.*.tmp-*\n").unwrap()
            },
        )],
        K::AgentInstructions => vec![damage(
            "instructions missing",
            "AGENTS.md",
            "Billet instructions are missing",
            "run `blt init` to add the instructions",
            Reaction::Unaffected,
            move |d: &Path| fs::remove_file(d.join("AGENTS.md")).unwrap(),
        )],
        K::PendingMove => vec![damage(
            "pending move",
            ".blt/.move",
            format!("pending move journal for {DMG_HEALTHY}"),
            "finish it with any write command in a writable checkout, or resolve .blt/.move manually",
            Reaction::Journal,
            move |d: &Path| {
                let old = fs::read_to_string(d.join(dmg_path(DMG_HEALTHY))).unwrap();
                let new = old.replace("status: open", "status: closed");
                fs::write(d.join(".blt/.move"), journal(&old, &new)).unwrap();
            },
        )],
        K::MoveJournal => vec![damage(
            "corrupt journal lengths",
            ".blt/.move",
            "",
            "resolve .blt/.move manually",
            Reaction::Journal,
            move |d: &Path| {
                fs::write(
                    d.join(".blt/.move"),
                    "BLT-MOVE-1\n18446744073709551615\n2\nxy",
                )
                .unwrap()
            },
        )],
    })
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

fn restore_modes(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for rel in [
            ".blt/closed",
            ".blt/docs",
            ".blt/memory.md",
            ".blt/.gitignore",
        ] {
            let path = root.join(rel);
            if let Ok(meta) = fs::symlink_metadata(&path) {
                let mode = if meta.is_dir() { 0o755 } else { 0o644 };
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(mode));
            }
        }
    }
}

fn write_non_utf8(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        fs::write(dir.join(std::ffi::OsString::from_vec(vec![0xff])), "x").unwrap();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn permission_seed_works(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => fs::read_dir(path).is_err(),
        Ok(_) => fs::read(path).is_err(),
        Err(_) => true,
    }
}

fn readable_snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, rel: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            out.push((rel.to_string_lossy().to_string(), b"UNLISTABLE".to_vec()));
            return;
        };
        let mut entries: Vec<_> = entries.map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = rel.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&entry.path(), &path, out);
            } else if kind.is_file() {
                let bytes = fs::read(entry.path()).unwrap_or_else(|_| b"UNREADABLE".to_vec());
                out.push((path.to_string_lossy().to_string(), bytes));
            } else {
                out.push((path.to_string_lossy().to_string(), b"LINK".to_vec()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, Path::new(""), &mut out);
    out
}

fn has_id(stdout: &str, key: &str) -> bool {
    stdout.contains(&format!("\"id\":\"{}\"", h(key)))
}

fn check_damage(base: &Path, root: &Path, kind: billet::ProblemKind, index: usize, dmg: &Damage) {
    let label = format!("{kind:?} / {}", dmg.name);
    let d = root.join(format!("{kind:?}-{index}"));
    copy_tree(base, &d);
    (dmg.apply)(&d);
    let file_level = matches!(
        kind,
        billet::ProblemKind::FileNameMismatch
            | billet::ProblemKind::DuplicateId
            | billet::ProblemKind::PendingMove
    );
    let problem_text = if file_level {
        dmg.problem.clone()
    } else {
        render_keys(&d, &dmg.problem)
    };
    let hint_text = if kind == billet::ProblemKind::DuplicateNumber {
        dmg.hint.clone()
    } else {
        render_keys(&d, &dmg.hint)
    };
    let victim = h(DMG_VICTIM);
    let target = d.join(&dmg.path);
    let permission =
        dmg.problem.starts_with("cannot list directory") || dmg.problem.starts_with("cannot read");
    if permission && !permission_seed_works(&target) {
        restore_modes(&d);
        return;
    }
    let abs = d.to_string_lossy().to_string();

    let kinds = billet::problem_kinds(&d);
    assert!(
        kinds
            .iter()
            .any(|(k, path)| *k == kind && path.to_string_lossy() == dmg.path.as_str()),
        "{label}: the seed must produce its kind at {}; got {kinds:?}",
        dmg.path
    );
    if kind != billet::ProblemKind::DuplicateNumber {
        assert!(
            kinds
                .iter()
                .all(|(k, _)| *k != billet::ProblemKind::DuplicateNumber),
            "{label}: only a duplicated number may report one: {kinds:?}"
        );
    }

    let lock = d.join(".blt/.lock");
    let had_lock = lock.exists();
    if had_lock {
        fs::remove_file(&lock).unwrap();
    }
    let before = readable_snapshot(&d);
    let o = run(&d, &["--json", "check"]);
    assert_eq!(o.status.code(), Some(1), "{label}");
    let stdout = String::from_utf8(o.stdout).unwrap();
    assert!(
        !stdout.contains(&abs),
        "{label}: absolute path in check: {stdout}"
    );
    let problems: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    for p in &problems {
        let (path, problem, hint) = (
            p["path"].as_str().unwrap(),
            p["problem"].as_str().unwrap(),
            p["hint"].as_str().unwrap(),
        );
        assert!(Path::new(path).is_relative(), "{label}: {path}");
        assert!(!problem.is_empty() && !hint.is_empty(), "{label}: {p}");
        assert!(
            !problem.contains(path),
            "{label}: problem repeats its path: {problem}"
        );
    }
    let entry = problems
        .iter()
        .find(|p| {
            p["path"] == dmg.path.as_str() && p["problem"].as_str().unwrap().contains(&problem_text)
        })
        .unwrap_or_else(|| {
            panic!(
                "{label}: no problem at {} containing {:?}: {stdout}",
                dmg.path, problem_text
            )
        });
    assert_eq!(entry["hint"], hint_text.as_str(), "{label}: hint");
    let text = String::from_utf8(run(&d, &["check"]).stdout).unwrap();
    let line = format!(
        "{}: {}\n  hint: {}\n",
        dmg.path,
        entry["problem"].as_str().unwrap(),
        hint_text
    );
    assert!(
        text.contains(&line),
        "{label}: text report lacks {line:?}\n{text}"
    );
    assert!(
        !text.contains(&abs),
        "{label}: absolute path in text report"
    );
    assert_eq!(
        readable_snapshot(&d),
        before,
        "{label}: check changed the project"
    );
    assert!(!lock.exists(), "{label}: check created .lock");
    if had_lock {
        fs::write(&lock, "").unwrap();
    }

    let warnings_ok = |args: &[&str], o: &Output| {
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(
            !stderr.contains(&abs),
            "{label} {args:?}: absolute path: {stderr}"
        );
        for line in stderr.lines() {
            assert!(
                line.contains("blt check"),
                "{label} {args:?}: warning without pointer: {line}"
            );
        }
    };
    let unchanged = |args: &[&str]| {
        let before = readable_snapshot(&d);
        let o = run(&d, args);
        assert!(!o.status.success(), "{label}: {args:?} must refuse");
        assert_eq!(
            readable_snapshot(&d),
            before,
            "{label}: refused {args:?} changed files"
        );
        String::from_utf8_lossy(&o.stderr).to_string()
    };
    match &dmg.reaction {
        Reaction::Skipped(paths) => {
            for args in [
                vec!["--json", "list"],
                vec!["--json", "ready"],
                vec!["--json", "tree", DMG_ROOT],
            ] {
                let o = run(&d, &args);
                assert!(
                    o.status.success(),
                    "{label} {args:?}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
                warnings_ok(&args, &o);
                let stderr = String::from_utf8_lossy(&o.stderr);
                let stdout = String::from_utf8_lossy(&o.stdout);
                for path in paths {
                    let expected = usize::from(args[1] != "ready" || path.starts_with(".blt/open"));
                    assert_eq!(
                        stderr.matches(&format!("skipping {path}:")).count(),
                        expected,
                        "{label} {args:?}: warn {path} once\n{stderr}"
                    );
                }
                assert!(
                    !has_id(&stdout, DMG_VICTIM),
                    "{label} {args:?}: damaged issue listed: {stdout}"
                );
                assert_eq!(
                    has_id(&stdout, DMG_ROOT),
                    args[1] != "ready",
                    "{label} {args:?}: the root is listed but waits for its subtasks"
                );
                assert!(
                    has_id(&stdout, DMG_GOOD),
                    "{label} {args:?}: sibling missing"
                );
                if args[1] == "tree" {
                    let tree: Value = serde_json::from_str(&stdout).unwrap();
                    assert!(
                        tree["children"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|c| c["id"] == h(DMG_LEAF).as_str()),
                        "{label}: the orphan reattaches under the nearest visible ancestor: {stdout}"
                    );
                } else {
                    assert!(
                        has_id(&stdout, DMG_HEALTHY),
                        "{label} {args:?}: healthy issue missing"
                    );
                }
            }
            let text_tree = ok(&d, &["tree", DMG_ROOT]);
            assert!(
                text_tree
                    .lines()
                    .any(|l| l == format!("  {} open T {DMG_LEAF}", h(DMG_LEAF))),
                "{label}: the orphan is indented under the root: {text_tree}"
            );
            for args in [
                vec!["show", DMG_VICTIM],
                vec!["tree", DMG_VICTIM],
                vec!["claim", DMG_VICTIM],
                vec!["update", DMG_VICTIM, "-p", "1"],
            ] {
                let stderr = unchanged(&args);
                assert!(
                    stderr.contains(&victim) || stderr.contains(DMG_VICTIM),
                    "{label} {args:?}: {stderr}"
                );
                if args[0] == "show" && !problem_text.is_empty() {
                    assert!(
                        stderr.contains(&problem_text),
                        "{label}: show names the cause: {stderr}"
                    );
                }
            }
        }
        Reaction::Visible => {
            assert!(
                !stdout.contains("unresolved"),
                "{label}: an inconsistent record is not an unresolved reference: {stdout}"
            );
            let path = dmg_path(DMG_VICTIM);
            for args in [vec!["--json", "list"], vec!["--json", "tree", DMG_ROOT]] {
                let o = run(&d, &args);
                assert!(o.status.success(), "{label} {args:?}");
                warnings_ok(&args, &o);
                let stderr = String::from_utf8_lossy(&o.stderr);
                assert!(
                    has_id(&String::from_utf8_lossy(&o.stdout), DMG_VICTIM),
                    "{label} {args:?}"
                );
                assert_eq!(
                    stderr
                        .matches(&format!(
                            "{path}: {}; run `blt check`",
                            entry["problem"].as_str().unwrap()
                        ))
                        .count(),
                    1,
                    "{label} {args:?}: {stderr}"
                );
                assert!(
                    !stderr.contains(&format!("skipping {path}")),
                    "{label} {args:?}: {stderr}"
                );
            }
            let o = run(&d, &["--json", "ready"]);
            assert!(
                !has_id(&String::from_utf8_lossy(&o.stdout), DMG_VICTIM),
                "{label} ready"
            );
            assert!(
                String::from_utf8_lossy(&o.stderr).contains(&format!("skipping {path}")),
                "{label} ready"
            );
            ok(&d, &["show", DMG_VICTIM]);
            let mut writes = vec![
                vec!["claim", DMG_VICTIM],
                vec!["claim", DMG_VICTIM, "--force"],
                vec!["update", DMG_VICTIM, "--title", "x"],
                vec!["close", DMG_VICTIM, "--reason", "r"],
            ];
            if dmg.name == "claimed without claim_id" {
                writes.push(vec!["unclaim", DMG_VICTIM]);
            }
            for args in writes {
                let stderr = unchanged(&args);
                assert!(
                    stderr.contains(&victim) || stderr.contains(DMG_VICTIM),
                    "{label} {args:?}: {stderr}"
                );
            }
        }
        Reaction::Unresolved(id) => {
            let shown = render_key(&d, id);
            let listed = |stdout: &str| stdout.contains(&format!("\"id\":\"{shown}\""));
            assert!(listed(&ok(&d, &["--json", "list"])), "{label}: list");
            let o = run(&d, &["--json", "ready"]);
            assert!(
                !listed(&String::from_utf8_lossy(&o.stdout)),
                "{label}: ready"
            );
            assert!(
                String::from_utf8_lossy(&o.stderr)
                    .contains(&format!("skipping {shown}: unresolved ")),
                "{label}"
            );
            let stderr = unchanged(&["claim", id]);
            assert!(stderr.ends_with("(run `blt check`)\n"), "{label}: {stderr}");
        }
        Reaction::Refused => {
            for args in [
                vec!["list"],
                vec!["ready"],
                vec!["show", DMG_HEALTHY],
                vec!["add", "x"],
            ] {
                let stderr = unchanged(&args);
                assert!(
                    stderr.starts_with("blt: ") && !stderr.contains(&abs),
                    "{label} {args:?}: {stderr}"
                );
            }
        }
        Reaction::Unaffected => {
            for args in [vec!["--json", "list"], vec!["--json", "ready"]] {
                let o = run(&d, &args);
                assert!(
                    o.status.success(),
                    "{label} {args:?}: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
                warnings_ok(&args, &o);
                assert!(
                    has_id(&String::from_utf8_lossy(&o.stdout), DMG_GOOD),
                    "{label} {args:?}"
                );
            }
            if dmg.name == "docs entry is a file" {
                let o = run(&d, &["show", DMG_VICTIM]);
                assert!(o.status.success(), "{label}: show");
                assert!(
                    String::from_utf8_lossy(&o.stdout)
                        .contains(&format!("Documents: .blt/docs/{DMG_VICTIM}/"))
                );
                assert!(
                    String::from_utf8_lossy(&o.stderr).contains(&format!(".blt/docs/{DMG_VICTIM}"))
                );
            }
        }
        Reaction::Journal => check_journal(&d, &label, dmg),
        Reaction::Ambiguous(handle) => {
            let warning = format!("number {handle} is used by");
            for args in [
                vec!["--json", "list"],
                vec!["--json", "ready"],
                vec!["--json", "tree", DMG_ROOT],
            ] {
                let o = run(&d, &args);
                assert!(o.status.success(), "{label} {args:?}");
                warnings_ok(&args, &o);
                assert_eq!(
                    String::from_utf8_lossy(&o.stderr).matches(&warning).count(),
                    1,
                    "{label} {args:?}: one warning per duplicate"
                );
            }
            for args in [
                vec!["show", handle.as_str()],
                vec!["claim", handle.as_str()],
            ] {
                let stderr = unchanged(&args);
                assert!(
                    stderr.contains(&format!("{handle} is ambiguous: "))
                        && stderr.contains(&dmg_path(DMG_TWIN))
                        && stderr.contains(&dmg_path(DMG_HEALTHY)),
                    "{label} {args:?}: names every match: {stderr}"
                );
            }
            ok(&d, &["show", DMG_TWIN]);
        }
    }
    restore_modes(&d);
    fs::remove_dir_all(&d).unwrap();
}

fn check_journal(d: &Path, label: &str, dmg: &Damage) {
    let src = d.join(dmg_path(DMG_HEALTHY));
    let dst = d.join(format!(".blt/closed/{DMG_HEALTHY}.md"));
    let journal_file = d.join(".blt/.move");
    if dmg.name == "corrupt journal lengths" {
        let o = run(d, &["ready"]);
        assert_eq!(o.status.code(), Some(1), "{label}");
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("journal"),
            "{label}"
        );
        assert!(journal_file.exists(), "{label}: journal must be preserved");
        return;
    }
    let pending = fs::read(&journal_file).unwrap();
    let old = fs::read_to_string(&src).unwrap();
    let new = old.replace("status: open", "status: closed");
    let shown = ok(d, &["show", DMG_HEALTHY]);
    assert!(
        shown.contains("closed"),
        "{label}: a read finishes the move: {shown}"
    );
    assert!(!journal_file.exists() && !src.exists(), "{label}");
    assert_eq!(fs::read_to_string(&dst).unwrap(), new, "{label}");
    assert_eq!(ok(d, &["--json", "check"]), "[]\n", "{label}");

    fs::write(&dst, &old).unwrap();
    fs::write(&journal_file, &pending).unwrap();
    ok(d, &["--json", "list", "--status", "closed"]);
    assert_eq!(
        fs::read_to_string(&dst).unwrap(),
        new,
        "{label}: replayed over a stale copy"
    );
    assert!(!journal_file.exists(), "{label}");

    fs::write(&src, "stale").unwrap();
    fs::write(&journal_file, &pending).unwrap();
    let o = run(d, &["ready"]);
    assert_eq!(o.status.code(), Some(1), "{label}: stale source refuses");
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("journal"),
        "{label}"
    );
    assert!(journal_file.exists(), "{label}: journal preserved");
    fs::remove_file(&src).unwrap();
    ok(d, &["ready"]);
    assert!(!journal_file.exists(), "{label}");
}

fn check_entries(d: &Path) -> Vec<(String, String, String)> {
    let o = run(d, &["--json", "check"]);
    let problems: Vec<Value> = serde_json::from_slice(&o.stdout).unwrap();
    problems
        .iter()
        .map(|p| {
            (
                p["path"].as_str().unwrap().to_string(),
                p["problem"].as_str().unwrap().to_string(),
                p["hint"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn separate_damage(kind: billet::ProblemKind) -> Option<&'static str> {
    use billet::ProblemKind as K;
    match kind {
        K::IssueDirectory | K::IssueDirectoryUnlistable => {
            Some("replaces or locks .blt/closed, which the combined cases write to")
        }
        K::DocsDirectory | K::DocsUnlistable => {
            Some("replaces or locks .blt/docs, which the combined cases write to")
        }
        K::MemoryFile | K::MemoryMissing | K::MemoryUnreadable => {
            Some("the same .blt/memory.md as the combined MemoryMalformed case")
        }
        K::GitignoreFile | K::GitignoreMissing | K::GitignoreUnreadable => {
            Some("the same .blt/.gitignore as the combined GitignoreLegacyRule case")
        }
        K::PendingMove => Some("the same .blt/.move as the combined MoveJournal case"),
        K::ConfigFile | K::ConfigMissing | K::ConfigInvalid => {
            Some("without a valid prefix every other case would show keys instead of handles")
        }
        _ => None,
    }
}

fn check_all_damages_at_once(base: &Path, root: &Path) {
    let mut plan: Vec<(billet::ProblemKind, &'static str, Damage)> = Vec::new();
    let mut separated = Vec::new();
    let mut next = 10;
    for kind in billet::ProblemKind::ALL {
        if let Some(reason) = separate_damage(kind) {
            separated.push((kind, reason));
            continue;
        }
        let Ok(list) = damages(kind, DMG_VICTIM) else {
            continue;
        };
        for index in 0..list.len() {
            let victim: &'static str = if kind == billet::ProblemKind::UnresolvedReference {
                DMG_VICTIM
            } else {
                next += 1;
                Box::leak(format!("{DMG_ROOT}.{next}").into_boxed_str())
            };
            let dmg = damages(kind, victim).unwrap().remove(index);
            plan.push((kind, victim, dmg));
        }
    }
    use billet::ProblemKind as K;
    assert_eq!(
        separated.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
        [
            K::IssueDirectory,
            K::IssueDirectoryUnlistable,
            K::DocsUnlistable,
            K::DocsDirectory,
            K::MemoryFile,
            K::MemoryMissing,
            K::MemoryUnreadable,
            K::GitignoreFile,
            K::GitignoreMissing,
            K::GitignoreUnreadable,
            K::PendingMove,
            K::ConfigFile,
            K::ConfigMissing,
            K::ConfigInvalid,
        ],
        "damages kept out of the combined registry: {separated:?}"
    );
    let prepare = |d: &Path, victims: &[&str]| {
        copy_tree(base, d);
        for victim in victims.iter().filter(|v| **v != DMG_VICTIM) {
            fs::write(
                d.join(dmg_path(victim)),
                dmg_text(victim, "open", Some(DMG_ROOT)),
            )
            .unwrap();
        }
    };
    let singles: Vec<Vec<(String, String, String)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = plan
            .iter()
            .enumerate()
            .map(|(index, (_, victim, dmg))| {
                let prepare = &prepare;
                scope.spawn(move || {
                    let d = root.join(format!("single-{index}"));
                    prepare(&d, &[victim]);
                    (dmg.apply)(&d);
                    check_entries(&d)
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    let expected: std::collections::BTreeSet<_> = singles.into_iter().flatten().collect();

    let d = root.join("combined");
    let victims: Vec<&str> = plan.iter().map(|(_, victim, _)| *victim).collect();
    prepare(&d, &victims);
    for (_, _, dmg) in &plan {
        (dmg.apply)(&d);
    }
    let lock = d.join(".blt/.lock");
    if lock.exists() {
        fs::remove_file(&lock).unwrap();
    }
    let before = readable_snapshot(&d);
    let entries = check_entries(&d);
    let unique: std::collections::BTreeSet<_> = entries.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        entries.len(),
        "check reports each problem once"
    );
    let missing: Vec<_> = expected.difference(&unique).collect();
    let extra: Vec<_> = unique.difference(&expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "one check reports exactly the problems of the single cases\nmissing: {missing:#?}\nextra: {extra:#?}"
    );
    let text: String = entries
        .iter()
        .map(|(path, problem, hint)| format!("{path}: {problem}\n  hint: {hint}\n"))
        .collect();
    let o = run(&d, &["check"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        text,
        "the text report lists the same problems"
    );
    assert_eq!(
        readable_snapshot(&d),
        before,
        "check changed the combined registry"
    );
    assert!(!lock.exists(), "check created .lock");
    restore_modes(&d);
}

#[test]
fn damaged_registry_by_problem_kind() {
    let root = dir("damaged");
    let base = root.join("base");
    fs::create_dir(&base).unwrap();
    ok(&base, &["init", "--no-hooks"]);
    for (id, parent) in [
        (DMG_ROOT, None),
        (DMG_GOOD, Some(DMG_ROOT)),
        (DMG_VICTIM, Some(DMG_ROOT)),
        (DMG_LEAF, Some(DMG_VICTIM)),
        (DMG_HEALTHY, None),
    ] {
        fs::write(base.join(dmg_path(id)), dmg_text(id, "open", parent)).unwrap();
    }
    assert_eq!(
        ok(&base, &["--json", "check"]),
        "[]\n",
        "the base registry is healthy"
    );
    let mut unseedable = Vec::new();
    std::thread::scope(|scope| {
        scope.spawn(|| check_all_damages_at_once(&base, &root));
        for kind in billet::ProblemKind::ALL {
            match damages(kind, DMG_VICTIM) {
                Ok(list) => {
                    assert!(!list.is_empty(), "{kind:?} has no damage case");
                    for (index, dmg) in list.into_iter().enumerate() {
                        let (base, root) = (&base, &root);
                        scope.spawn(move || check_damage(base, root, kind, index, &dmg));
                    }
                }
                Err(reason) => unseedable.push((kind, reason)),
            }
        }
    });
    let kinds: Vec<_> = unseedable.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [
            billet::ProblemKind::IssueEntryUnlistable,
            billet::ProblemKind::DocsEntryUnlistable,
            billet::ProblemKind::DocsEntryUninspectable,
        ],
        "only these kinds may lack a seeded case: {unseedable:?}"
    );
}

fn usage_head(syntax: &str) -> String {
    syntax
        .split(' ')
        .take_while(|token| !token.starts_with('[') && !token.starts_with('-'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sample_argv(command: &billet::commands::CommandSpec, id: &str) -> Vec<String> {
    use billet::commands::ArgType;
    use billet::text::TextKind;
    let mut argv = vec![command.name.to_string()];
    let mut positional = Vec::new();
    for arg in command.args.iter().filter(|arg| arg.required) {
        let value = match arg.kind {
            ArgType::Text(TextKind::Id) | ArgType::TextList(TextKind::Id) => id.to_string(),
            ArgType::Text(TextKind::Release) => "1.0".into(),
            ArgType::Text(TextKind::Topic) => "topic".into(),
            ArgType::Text(TextKind::Date) => LATER.into(),
            ArgType::Text(_) | ArgType::TextList(_) => "x".into(),
            ArgType::Enum(values) => values[0].into(),
            ArgType::Integer => "1".into(),
            ArgType::Boolean => continue,
        };
        if arg.positional {
            positional.push(value);
        } else {
            argv.extend([arg.cli.unwrap().to_string(), value]);
        }
    }
    match command.name {
        "update" => argv.extend(["--title".to_string(), "x".to_string()]),
        "defer" => argv.push("--clear".into()),
        _ => {}
    }
    argv.push("--".into());
    argv.extend(positional);
    argv
}

fn check_command_contract(
    s: &Seeded,
    reg: &Path,
    noreg: &Path,
    help: &str,
    command: &billet::commands::CommandSpec,
) {
    use billet::commands::{ArgType, JSON_CMDS};
    let name = command.name;
    let label = |what: &str| format!("{name}: {what}");
    let refusal = |args: &[String], dir: &Path, code: i32, stderr: &str, what: &str| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let before = tree_bytes(dir);
        let o = run(dir, &refs);
        assert_eq!(o.status.code(), Some(code), "{} {refs:?}", label(what));
        assert_eq!(
            String::from_utf8_lossy(&o.stderr),
            stderr,
            "{} {refs:?}",
            label(what)
        );
        assert_eq!(tree_bytes(dir), before, "{} changed files", label(what));
    };
    let with = |argv: &[String], extra: &[&str]| -> Vec<String> {
        let mut out = argv.to_vec();
        out.splice(1..1, extra.iter().map(|e| e.to_string()));
        out
    };

    assert!(
        help.lines()
            .any(|line| line.starts_with(&format!("  {}", command.syntax))),
        "{}",
        label("help has no line for the syntax")
    );

    let argv = sample_argv(command, &s.open);
    let mcp = name == "mcp";
    let mcp_refusal = "blt: mcp does not accept arguments\n";
    if !matches!(name, "init" | "mcp") {
        refusal(
            &sample_argv(command, "1e0001"),
            noreg,
            1,
            "blt: no .blt registry found; run `blt init` in the project root\n",
            "refusal without a registry",
        );
    }
    refusal(
        &with(&argv, &["--bogus"]),
        reg,
        2,
        if mcp {
            mcp_refusal
        } else {
            "blt: unknown option --bogus\n"
        },
        "unknown option",
    );

    let mut prefixed = vec!["--json".to_string()];
    prefixed.extend(argv.iter().cloned());
    let trailing = with(&argv, &["--json"]);
    if JSON_CMDS.contains(&name) {
        let a: Vec<&str> = prefixed.iter().map(String::as_str).collect();
        let b: Vec<&str> = trailing.iter().map(String::as_str).collect();
        let first = ok(reg, &a);
        serde_json::from_str::<Value>(&first)
            .unwrap_or_else(|e| panic!("{}: {e}", label("--json output")));
        assert_eq!(ok(reg, &b), first, "{}", label("--json position"));
    } else {
        refusal(
            &prefixed,
            reg,
            2,
            &format!("blt: --json is not valid for `{name}`\n"),
            "--json prefix",
        );
        refusal(
            &trailing,
            reg,
            2,
            if mcp {
                mcp_refusal
            } else {
                "blt: unknown option --json\n"
            },
            "--json after the command",
        );
    }

    let positional = command.args.iter().filter(|a| a.positional).count();
    let required = command
        .args
        .iter()
        .filter(|a| a.positional && a.required)
        .count();
    let head = usage_head(command.syntax);
    let mut extra = argv.clone();
    extra.extend((0..=positional - required).map(|_| "extra".to_string()));
    let expected_extra = match (mcp, positional, name) {
        (true, _, _) => mcp_refusal.to_string(),
        (_, 0, "init") => "blt: expected no positional arguments\n".to_string(),
        (_, 0, _) => "blt: expected no arguments\n".to_string(),
        (_, _, "recall") => "blt: expected at most one topic\n".to_string(),
        _ => format!("blt: expected {head}\n"),
    };
    refusal(&extra, reg, 2, &expected_extra, "extra positional argument");
    if required > 0 {
        refusal(
            &[name.to_string()],
            reg,
            2,
            &format!("blt: expected {head}\n"),
            "missing argument",
        );
    }

    for arg in command
        .args
        .iter()
        .filter(|a| matches!(a.kind, ArgType::Integer))
    {
        let option = arg.cli.unwrap();
        for flag in [Some(option), arg.alias].into_iter().flatten() {
            for value in ["+2", " 2", "02", "5", "2x", "-1"] {
                let args = with(&argv, &[flag, value]);
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let o = run(reg, &refs);
                assert_eq!(o.status.code(), Some(2), "{} {value:?}", label(flag));
                assert_eq!(
                    String::from_utf8_lossy(&o.stderr),
                    format!("blt: invalid priority {value:?}; expected 0..=4\n"),
                    "{}",
                    label(flag)
                );
            }
        }
        for value in ["0", "1", "2", "3", "4"] {
            let slot = billet::commands::COMMANDS
                .iter()
                .position(|c| c.name == name)
                .unwrap();
            let d = s.fresh(1000 + slot * 10 + value.parse::<usize>().unwrap());
            let args = with(&argv, &[option, value]);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = ok(&d, &refs);
            let id = if name == "add" {
                out.trim().to_string()
            } else {
                s.open.clone()
            };
            assert_eq!(
                show_value(&d, &id)["priority"],
                json!(value.parse::<u8>().unwrap()),
                "{}",
                label(value)
            );
            fs::remove_dir_all(&d).unwrap();
        }
    }
    assert!(
        help.contains(command.description),
        "{}",
        label("help lacks the description")
    );
    let readme =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let reference = readme
        .split("## Command reference\n\n```text\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap();
    assert!(
        reference
            .lines()
            .any(|line| line == format!("blt {name}") || line.starts_with(&format!("blt {name} "))),
        "{}",
        label("README command reference has no line")
    );
}

#[test]
fn every_command_contract() {
    use billet::commands::COMMANDS;
    let s = seeded("commands");
    let reg = &s.base;
    let noreg = s.dir.join("noreg");
    fs::create_dir(&noreg).unwrap();
    let help = ok(reg, &["--help"]);
    std::thread::scope(|scope| {
        for command in COMMANDS {
            let (s, noreg, help) = (&s, &noreg, &help);
            scope.spawn(move || check_command_contract(s, reg, noreg, help, command));
        }
    });

    command_contract_globals(reg, &help);
    grouped_and_flat_lists(&s);
}

fn command_contract_globals(reg: &Path, help: &str) {
    assert_eq!(help, HELP, "help is part of the contract, byte for byte");
    assert!(
        help.lines().all(|line| line.len() <= 80),
        "help fits 80 columns"
    );
    for args in [&[][..], &["help"][..], &["-h"][..]] {
        assert_eq!(ok(reg, args), help, "{args:?}");
    }
    assert_eq!(
        ok(reg, &["--version"]),
        format!("blt {}\n", env!("CARGO_PKG_VERSION"))
    );
    for (args, stderr) in [
        (
            &["--", "--help"][..],
            "blt: unknown command --help; try `blt --help`\n",
        ),
        (&["--"][..], "blt: missing command; try `blt --help`\n"),
        (&["--json"][..], "blt: missing command; try `blt --help`\n"),
        (&["--bogus", "list"][..], "blt: unknown option --bogus\n"),
        (
            &["--json", "--json", "ready"][..],
            "blt: duplicate option --json\n",
        ),
        (
            &["request-changes", "1e0001"][..],
            "blt: unknown command request-changes; try `blt --help`\n",
        ),
    ] {
        let o = run(reg, args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), stderr, "{args:?}");
    }
    assert_eq!(
        ok(reg, &["--", "list", "--json"]),
        ok(reg, &["--json", "list"]),
        "-- before the command"
    );
    let o = run(reg, &["update", "1e0001"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&o.stderr),
        "blt: update requires at least one change\n"
    );

    let d = reg.parent().unwrap().join("wire");
    fs::create_dir(&d).unwrap();
    ok(&d, &["init", "--no-hooks"]);
    let marker = "SECRET-BODY-žluťoučký \"quoted\" `tick`";
    let title = "t \"dq\" pol";
    let id = ok(&d, &["add", title, "--body", marker, "--approved"])
        .trim()
        .to_string();
    ok(&d, &["remember", "topic", "SECRET-MEMORY"]);
    for args in [&["ready"][..], &["start"][..], &["list"][..]] {
        let out = ok(&d, args);
        assert!(
            !out.contains("SECRET-"),
            "{args:?} leaks a body or memory: {out}"
        );
    }
    let shown = ok(&d, &["--json", "show", &id]);
    let v: Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(v["title"], title);
    assert_eq!(v["body"], marker);
    assert!(v.get("claim_id").is_none(), "claim_id is internal: {shown}");
    assert!(ok(&d, &["show", &id]).contains(marker));
    assert!(ok(&d, &["--json", "start"]).starts_with("{\"workflow\":"));
}

fn grouped_and_flat_lists(s: &Seeded) {
    let d = &s.base;
    let grouped = ok(d, &["list"]);
    let headers: Vec<&str> = grouped.lines().filter(|line| line.ends_with(')')).collect();
    assert_eq!(
        headers,
        ["REVIEW (1)", "PROPOSED (1)", "CLAIMED (1)", "OPEN (2)"]
    );
    for (id, status) in [
        (&s.review, "review"),
        (&s.proposed, "proposed"),
        (&s.open, "open"),
        (&s.claimed, "claimed"),
    ] {
        let row = grouped
            .lines()
            .find(|line| line.starts_with(&format!("  {id} ")))
            .unwrap();
        let flat = ok(d, &["list", "--status", status]);
        let flat_row = flat
            .lines()
            .find(|line| line.starts_with(&format!("{id} ")))
            .unwrap();
        assert_eq!(&row[2..], flat_row, "grouped and flat rows are identical");
    }
    assert!(
        !grouped.contains(s.closed.as_str()),
        "closed issues are not grouped"
    );
    let sealed = s.dir.join("lists");
    copy_tree(d, &sealed);
    ok(&sealed, &["release", "1.0"]);
    for args in [
        &["list", "--status", "all"][..],
        &["list", "--status", "open"][..],
        &["list", "--release", "1.0"][..],
        &["list", "--sort", "newest"][..],
    ] {
        let flat = ok(&sealed, args);
        assert!(!flat.is_empty(), "{args:?}");
        assert!(
            flat.lines()
                .all(|line| !line.starts_with(' ') && !line.ends_with(')')),
            "{args:?} is flat: {flat}"
        );
    }
    assert!(ok(&sealed, &["list", "--status", "all"]).contains(&format!("{} closed", s.closed)));
    let o = run(d, &["list", "--status", "active"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&o.stderr),
        "blt: invalid status filter \"active\"; expected proposed|open|claimed|review|closed|all\n"
    );
}

fn crew(dir: &Path, args: &[&str]) -> Output {
    let bin = Path::new(env!("CARGO_BIN_EXE_blt")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("crew/bin/crew"))
        .args(args)
        .current_dir(dir)
        .env("PATH", path)
        .env("XDG_STATE_HOME", dir.join("xdg"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn crew_ok(dir: &Path, args: &[&str]) -> String {
    let o = crew(dir, args);
    assert!(
        o.status.success(),
        "crew {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(
        !out.contains("{{"),
        "crew {args:?} left a placeholder:\n{out}"
    );
    out
}

fn crew_refused(dir: &Path, args: &[&str], message: &str) {
    let o = crew(dir, args);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "crew {args:?} must refuse: {err}");
    assert!(o.stdout.is_empty(), "crew {args:?} refused with stdout");
    assert!(
        err.contains(message),
        "crew {args:?}: {err:?} lacks {message:?}"
    );
}

#[test]
fn crew_review_loop() {
    for tool in ["git", "jq"] {
        assert!(
            Command::new(tool).arg("--version").output().is_ok(),
            "crew needs {tool} on PATH"
        );
    }
    let d = dir("crew");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(&d)
            .output()
            .unwrap();
        assert!(o.status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    fs::write(d.join("code.txt"), "a\n").unwrap();
    git(&["add", "code.txt"]);
    git(&["commit", "-qm", "init"]);
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "Crew issue");
    let sub = d.join("sub");
    fs::create_dir(&sub).unwrap();
    let key = key_of(&d, &id);
    let round = |n: u32| format!(".blt/docs/{key}/review/round-{n:02}");
    let roles = [
        ("developer", "# Role: developer"),
        ("auditor", "# Role: auditor\n"),
        ("cross-check", "# Role: auditor, cross-check"),
        ("lead", "# Role: lead auditor"),
        ("architect", "# Role: architect"),
    ];
    let only_role = |out: &str, role: &str| {
        for (name, heading) in roles {
            assert_eq!(
                out.contains(heading),
                name == role,
                "{role} output and the {name} instructions:\n{out}"
            );
        }
        assert!(out.contains("# blt-crew: rules for every role"));
        assert_eq!(
            out.contains("# Auditing rules"),
            ["auditor", "cross-check", "lead"].contains(&role)
        );
    };

    crew_refused(&sub, &["dev", &id], "usage: crew dev ID LABEL");
    crew_refused(&sub, &["dev", &id, "bad label"], "invalid label");
    crew_refused(&sub, &["audit", &id, "a1"], "nothing to audit");
    crew_refused(&sub, &["audit", &id, "--lead"], "is open");
    crew_refused(&sub, &["lead", &id], "unknown role");
    crew_refused(&sub, &["architect", &id], "already approved");

    let proposal = ok(&d, &["add", "Proposal"]).trim().to_string();
    let out = crew_ok(&sub, &["architect", &proposal]);
    only_role(&out, "architect");
    assert!(!out.contains("Round:"), "{out}");
    let architecture = format!(".blt/docs/{}/architecture.md", key_of(&d, &proposal));
    assert!(
        out.contains(&format!("Write: {architecture} (created)")),
        "{out}"
    );
    assert!(d.join(&architecture).is_file());
    ok(
        &d,
        &[
            "note",
            &proposal,
            "--by",
            "architect",
            "contract ready: split",
        ],
    );
    let part = ok(&d, &["add", "Part", "--parent", &proposal])
        .trim()
        .to_string();
    ok(&d, &["approve", &proposal]);
    ok(&d, &["approve", &part]);
    let out = crew_ok(&sub, &["dev", &part, "dev3"]);
    assert!(
        out.contains(&format!("{architecture} (architecture of {proposal})")),
        "{out}"
    );
    assert!(out.contains("Architect: reviewed ("), "{out}");
    assert!(out.contains("contract ready: split)"), "{out}");

    let out = crew_ok(&sub, &["dev", &id, "dev1"]);
    only_role(&out, "developer");
    assert!(out.contains("Mode:  new work"), "{out}");
    assert!(out.contains("Architect: not reviewed"), "{out}");
    assert!(out.contains(&format!("Write: {}/handoff.md (created)", round(1))));
    assert_eq!(show_value(&d, &id)["assignee"], "dev1");
    crew_refused(&sub, &["dev", &id, "dev2"], "claimed by dev1, not by dev2");

    fs::write(d.join("code.txt"), "b\n").unwrap();
    let fingerprint = crew_ok(&sub, &["fingerprint"]);
    assert!(fingerprint.starts_with("HEAD "), "{fingerprint}");
    let handoff = d.join(round(1)).join("handoff.md");
    let text = fs::read_to_string(&handoff).unwrap();
    let text: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with("Fingerprint:") {
                format!("Fingerprint: {}", fingerprint.trim())
            } else {
                line.to_string()
            }
        })
        .collect();
    fs::write(&handoff, text.join("\n")).unwrap();
    ok(&d, &["review", &id, "--summary", "round 01: done"]);
    crew_refused(&sub, &["dev", &id, "dev1"], "code is frozen");
    crew_refused(
        &sub,
        &["audit", &id, "a1", "--cross-check"],
        "no decision.md yet",
    );
    crew_refused(&sub, &["audit", &id, "lead"], "reserved");
    crew_refused(&sub, &["audit", &id, "architect"], "reserved");

    let out = crew_ok(&sub, &["audit", &id, "a1"]);
    only_role(&out, "auditor");
    assert!(out.contains("Code:  matches the handoff"), "{out}");
    assert!(out.contains(&format!("Write: {}/audit-a1.md (created)", round(1))));
    assert!(crew_ok(&sub, &["audit", &id, "a1"]).contains("audit-a1.md (existing)"));

    fs::write(d.join("new.txt"), "x\n").unwrap();
    let changed = crew_ok(&sub, &["audit", &id, "a2"]);
    assert!(
        changed.contains("WARNING: CHANGED since the handoff"),
        "{changed}"
    );
    fs::remove_file(d.join("new.txt")).unwrap();

    let out = crew_ok(&sub, &["audit", &id, "--lead"]);
    only_role(&out, "lead");
    assert!(out.contains("Code:  matches the handoff"), "{out}");
    for report in ["audit-a1.md", "audit-a2.md"] {
        assert!(out.contains(&format!("{}/{report} (auditor report)", round(1))));
    }
    assert!(out.contains(&format!(
        "blt rework {id} --reason \"round 01: see {}/decision.md\"",
        round(1)
    )));
    let out = crew_ok(&sub, &["audit", &id, "a3", "--cross-check"]);
    only_role(&out, "cross-check");
    assert!(out.contains(&format!("{}/decision.md", round(1))));

    ok(&d, &["rework", &id, "--reason", "round 01: see decision"]);
    let out = crew_ok(&sub, &["dev", &id, "dev1"]);
    assert!(out.contains("Mode:  fixes after round-01"), "{out}");
    assert!(out.contains(&format!("Read:  {}/decision.md", round(1))));
    assert!(out.contains(&format!("Write: {}/handoff.md (created)", round(2))));
    assert!(out.contains("round 02: <one line>"));
    ok(&d, &["review", &id, "--summary", "round 02: fixed"]);
    let out = crew_ok(&sub, &["audit", &id, "a1"]);
    assert!(out.contains(&format!("{}/decision.md (previous decision)", round(1))));
    assert!(
        out.contains("WARNING: the handoff has no fingerprint"),
        "{out}"
    );
}

#[test]
fn crew_install_and_uninstall() {
    let home = dir("crewhome");
    let custom = dir("crewclaude");
    let bin = Path::new(env!("CARGO_BIN_EXE_blt")).parent().unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("crew");
    let install = |args: &[&str], claude_dir: Option<&Path>| {
        let mut command = Command::new(source.join("install.sh"));
        command
            .args(args)
            .env("HOME", &*home)
            .env("PATH", &path)
            .env_remove("CLAUDE_CONFIG_DIR");
        if let Some(claude_dir) = claude_dir {
            command.env("CLAUDE_CONFIG_DIR", claude_dir);
        }
        command.output().unwrap()
    };
    let claude = home.join(".claude/skills/blt-crew");
    let codex = home.join(".agents/skills/blt-crew");

    fs::create_dir_all(&codex).unwrap();
    fs::write(codex.join("notes.md"), "mine\n").unwrap();
    let refused = install(&[], None);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("not a blt-crew skill"));
    assert!(codex.join("notes.md").is_file());
    fs::remove_dir_all(&codex).unwrap();

    std::os::unix::fs::symlink(&source, &codex).unwrap();
    let o = install(&[], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    for skill in [&claude, &codex] {
        assert!(
            !fs::symlink_metadata(skill)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        for file in [
            "SKILL.md",
            "agents/openai.yaml",
            "bin/crew",
            "roles/common.md",
            "roles/auditing.md",
            "roles/dev.md",
            "roles/audit.md",
            "roles/cross-check.md",
            "roles/lead.md",
            "roles/architect.md",
            "templates/architecture.md",
            "templates/handoff.md",
            "templates/audit.md",
            "templates/decision.md",
        ] {
            assert!(
                skill.join(file).is_file(),
                "{} lacks {file}",
                skill.display()
            );
        }
        assert_eq!(
            fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            fs::read_to_string(source.join("SKILL.md")).unwrap()
        );
        let usage = Command::new(skill.join("bin/crew")).output().unwrap();
        assert!(String::from_utf8_lossy(&usage.stdout).contains("crew dev ID LABEL"));
    }
    assert!(
        source.join("SKILL.md").is_file(),
        "the source checkout survives"
    );

    let o = install(&["--claude"], Some(&custom));
    assert!(o.status.success());
    let skill = fs::read_to_string(custom.join("skills/blt-crew/SKILL.md")).unwrap();
    let crew_path = format!("{}/skills/blt-crew/bin/crew", custom.display());
    assert!(
        skill.contains(&format!("allowed-tools: Bash({crew_path}:*)")),
        "{skill}"
    );
    assert!(
        skill.contains(&format!("!`{crew_path} $ARGUMENTS")),
        "{skill}"
    );
    assert!(!skill.contains("~/.claude/skills"), "{skill}");

    let o = install(&["--uninstall"], None);
    assert!(o.status.success());
    assert!(!claude.exists() && !codex.exists());
    assert_eq!(install(&["--bogus"], None).status.code(), Some(1));
}

#[test]
fn numbers_follow_the_configured_prefix() {
    let d = dir("numbers");
    ok(&d, &["init", "--no-hooks"]);
    assert_eq!(
        fs::read_to_string(d.join(".blt/config.json")).unwrap(),
        "{\n  \"prefix\": \"B\"\n}\n"
    );
    let first = add(&d, "first");
    let second = add(&d, "second");
    let part = ok(&d, &["add", "part", "--parent", "1"]).trim().to_string();
    assert_eq!(
        [first.as_str(), second.as_str(), part.as_str()],
        ["B1", "B2", "B1.1"]
    );
    let key = key_of(&d, "B1");
    assert!(billet::issue::valid_key(&key), "{key}");
    let file = fs::read_to_string(d.join(format!(".blt/open/{key}.md"))).unwrap();
    assert!(file.lines().any(|l| l == "number: 1"), "{file}");
    let child = fs::read_to_string(d.join(format!(".blt/open/{}.md", key_of(&d, "B1.1")))).unwrap();
    assert!(child.lines().any(|l| l == "number: null"), "{child}");
    for reference in ["B1", "b1", "1", key.as_str()] {
        assert_eq!(show_value(&d, reference)["id"], "B1", "{reference}");
    }

    ok(&d, &["close", &second, "--reason", "done"]);
    assert_eq!(add(&d, "third"), "B3", "closed issues keep their numbers");
    let e = bad(&d, &["update", "B1", "--number", "3"]);
    assert_eq!(e, "blt: number B3 is already used\n");
    let e = bad(&d, &["update", "B1.1", "--number", "9"]);
    assert_eq!(
        e,
        "blt: B1.1 is a subtask; its number comes from its root\n"
    );
    assert_eq!(ok(&d, &["update", "B1", "--number", "10"]), "updated B10\n");
    assert_eq!(
        ok(&d, &["tree", "B10"]),
        "B10 open first\n  B10.1 proposed part\n"
    );
    let history = show_value(&d, "B10")["history"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(history.ends_with("number: B1 -> B10"), "{history}");
    assert_eq!(bad(&d, &["show", "B1"]), "blt: issue B1 not found\n");
    assert_eq!(
        add(&d, "fourth"),
        "B11",
        "the next number follows the largest"
    );
    let fourth = d.join(format!(".blt/open/{}.md", key_of(&d, "B11")));
    let text = fs::read_to_string(&fourth).unwrap();
    fs::write(
        &fourth,
        format!("{text}<<<<<<< ours\n=======\n>>>>>>> theirs\n"),
    )
    .unwrap();
    assert_eq!(
        add(&d, "fifth"),
        "B12",
        "an unreadable file keeps its number reserved"
    );
    fs::write(&fourth, text).unwrap();

    fs::write(d.join(".blt/config.json"), "{\"prefix\": \"AN-\"}\n").unwrap();
    assert_eq!(show_value(&d, "an-10.1")["id"], "AN-10.1");
    assert!(ok(&d, &["list"]).contains("  AN-11 open 2 - fourth\n"));

    let other = dir("numbersprefix");
    assert_eq!(bad(&other, &["init", "--prefix", "an"]).lines().count(), 1);
    ok(&other, &["init", "--no-hooks", "--prefix", "AN"]);
    assert_eq!(add(&other, "x"), "AN1");
    let e = bad(&other, &["init", "--no-hooks", "--prefix", "B"]);
    assert!(e.contains("this registry uses prefix \"AN\""), "{e}");
}
