use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

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
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join("tests");
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
    Command::new(env!("CARGO_BIN_EXE_jig"))
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
    let mut ch = Command::new(env!("CARGO_BIN_EXE_jig"))
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

fn fm(id: &str, status: &str, extra: &str) -> String {
    format!(
        "---\nid: \"{id}\"\ntitle: \"T {id}\"\nstatus: {status}\npriority: 2\nparent: null\n\
         depends_on: []\nassignee: null\ndefer_until: null\ncreated_at: null\nupdated_at: null\n\
         closed_at: null\nclaim_id: null\n{extra}---\n\n## History\n\n## Notes\n\n"
    )
}

fn journal(old: &str, new: &str) -> String {
    format!("JIG-MOVE-1\n{}\n{}\n{old}{new}", old.len(), new.len())
}

#[test]
fn init_and_nested_discovery() {
    let d = dir("init");
    let proj = d.join("proj");
    fs::create_dir_all(&proj).unwrap();
    assert_eq!(ok(&proj, &["init"]), "initialized .jig/\n");
    assert!(proj.join(".jig/open").is_dir());
    assert!(proj.join(".jig/closed").is_dir());
    assert!(proj.join(".jig/memory.md").is_file());
    let gi = fs::read_to_string(proj.join(".jig/.gitignore")).unwrap();
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
    assert_eq!(ok(&proj, &["init"]), ".jig/ already initialized\n");

    let mem = proj.join(".jig/memory.md");
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
    assert!(e.contains("no .jig"), "{e}");
    let deep = repo.join("sub");
    fs::create_dir_all(&deep).unwrap();
    let e = bad(&deep, &["ready"]);
    assert!(e.contains("no .jig"), "{e}");
    assert_eq!(ok(&repo, &["init"]), "initialized .jig/\n");
}

#[test]
fn missing_dirs_are_empty_registry() {
    let d = dir("emptydirs");
    let proj = d.join("proj");
    fs::create_dir_all(proj.join(".jig")).unwrap();
    fs::write(proj.join(".jig/memory.md"), "# Project Memory\n").unwrap();
    assert_eq!(ok(&proj, &["ready"]), "");
    let id = add(&proj, "x");
    assert!(proj.join(format!(".jig/open/{id}.md")).is_file());
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
    let text = fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap();
    assert!(text.contains("status: claimed"));
    assert!(text.contains("claim_id: \""));
    let i = jig::issue::decode(&text).unwrap();
    let who = i.assignee.clone().unwrap();
    assert!(who == "one" || who == "two");
}

#[test]
fn same_name_copies_get_distinct_claim_ids() {
    let d1 = dir("claimA");
    let d2 = dir("claimB");
    ok(&d1, &["init"]);
    ok(&d2, &["init"]);
    let shared = fm("B-aaaaaa", "open", "");
    fs::write(d1.join(".jig/open/B-aaaaaa.md"), &shared).unwrap();
    fs::write(d2.join(".jig/open/B-aaaaaa.md"), &shared).unwrap();
    let o1 = ok(&d1, &["claim", "B-aaaaaa", "--by", "bot"]);
    let o2 = ok(&d2, &["claim", "B-aaaaaa", "--by", "bot"]);
    let t1 = fs::read_to_string(d1.join(".jig/open/B-aaaaaa.md")).unwrap();
    let t2 = fs::read_to_string(d2.join(".jig/open/B-aaaaaa.md")).unwrap();
    let cid = |t: &str| jig::issue::decode(t).unwrap().claim_id.unwrap();
    assert_ne!(cid(&t1), cid(&t2));
    assert!(!o1.contains(&cid(&t1)) && !o2.contains(&cid(&t2)));
}

#[test]
fn dependency_rules() {
    let d = dir("deps");
    ok(&d, &["init"]);
    let a = add(&d, "a");
    let b = add(&d, "b");
    ok(&d, &["dep", "add", &a, &b]);
    let ready = ok(&d, &["ready"]);
    assert!(!ready.contains(&a));
    assert!(ready.contains(&b));
    let e = bad(&d, &["dep", "add", &b, &a]);
    assert!(e.contains("cycle"), "{e}");
    let e = bad(&d, &["dep", "add", &a, "B-000000"]);
    assert!(e.contains("not found"), "{e}");
    let e = bad(&d, &["claim", &a]);
    assert!(e.contains("blocked"), "{e}");

    ok(&d, &["claim", &b, "--by", "x"]);
    ok(&d, &["review", &b]);
    ok(&d, &["claim", &a, "--force", "--by", "y"]);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{a}.md"))).unwrap())
        .unwrap();
    assert!(i.history.contains("bypassing"));

    let c = add(&d, "c");
    ok(&d, &["dep", "add", &c, &a]);
    let e = bad(&d, &["dep", "add", &a, &c]);
    assert!(e.contains("cycle"), "{e}");

    ok(&d, &["dep", "rm", &a, &b]);
    let e = bad(&d, &["dep", "rm", &a, &b]);
    assert!(e.contains("does not depend"), "{e}");

    let f = add(&d, "f");
    ok(&d, &["dep", "add", &f, &b]);
    let e = bad(&d, &["claim", &f]);
    assert!(e.contains("blocked"), "{e}");
    ok(&d, &["claim", &f, "--force"]);
}

#[test]
fn closed_edge_still_blocks_cycles() {
    let d = dir("closedcycle");
    ok(&d, &["init"]);
    let x = add(&d, "x");
    let y = add(&d, "y");
    ok(&d, &["dep", "add", &x, &y]);
    ok(&d, &["close", &x, "--reason", "done"]);
    let e = bad(&d, &["dep", "add", &y, &x]);
    assert!(e.contains("cycle"), "{e}");
    let ready = ok(&d, &["ready"]);
    assert!(ready.contains(&y), "y itself has no blockers: {ready}");
}

#[test]
fn defer_and_force_limits() {
    let d = dir("defer");
    ok(&d, &["init"]);
    let g = add(&d, "grand");
    let e = bad(&d, &["defer", &g, "--until", "2026-02-30"]);
    assert!(e.contains("invalid defer date"), "{e}");
    bad(&d, &["defer", &g, "--until", "2026-13-40"]);
    bad(&d, &["defer", &g, "--until", "2999-01-01", "--clear"]);
    ok(&d, &["defer", &g, "--until", "2999-01-01"]);
    let r = ok(&d, &["ready"]);
    assert!(!r.contains(&g));
    let e = bad(&d, &["claim", &g, "--force"]);
    assert!(e.contains("deferred"), "{e}");

    let p = add(&d, "parent");
    let c = ok(&d, &["add", "child", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    ok(&d, &["defer", &p, "--until", "2999-01-01"]);
    let r = ok(&d, &["ready"]);
    assert!(!r.contains(&c), "child of deferred parent must hide: {r}");
    let e = bad(&d, &["claim", &c, "--force"]);
    assert!(e.contains("deferred"), "{e}");

    let g2 = add(&d, "gp2");
    let mid2 = ok(&d, &["add", "mid2", "--parent", &g2, "--approved"])
        .trim()
        .to_string();
    let leaf2 = ok(&d, &["add", "leaf2", "--parent", &mid2, "--approved"])
        .trim()
        .to_string();
    ok(&d, &["defer", &g2, "--until", "2999-01-01"]);
    let r = ok(&d, &["ready"]);
    assert!(!r.contains(&mid2));
    assert!(!r.contains(&leaf2), "grandchild must hide: {r}");

    let dp = add(&d, "dpar");
    let dc = ok(&d, &["add", "dchild", "--parent", &dp, "--approved"])
        .trim()
        .to_string();
    ok(&d, &["defer", &dp, "--until", "2999-01-01"]);
    assert!(!ok(&d, &["ready"]).contains(&dc));
    ok(&d, &["close", &dp, "--reason", "cancelled"]);
    let r = ok(&d, &["ready"]);
    assert!(
        r.contains(&dc),
        "closed ancestor deferral must not hide child: {r}"
    );

    ok(&d, &["close", &g, "--reason", "n/a"]);
    ok(&d, &["defer", &g, "--clear"]);
    let e = bad(&d, &["defer", &g, "--until", "2999-01-01"]);
    assert!(e.contains("closed"), "{e}");

    let today = {
        let i =
            jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{p}.md"))).unwrap())
                .unwrap();
        i.defer_until.clone().unwrap()
    };
    assert_eq!(today, "2999-01-01");
    ok(&d, &["defer", &p, "--clear"]);
    let r = ok(&d, &["ready"]);
    assert!(r.contains(&c), "child returns after clearing: {r}");
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
    let e = bad(&d, &["add", "orphan", "--parent", "B-000000"]);
    assert!(e.contains("not found"), "{e}");
}

#[test]
fn review_rounds_close_reopen() {
    let d = dir("lifecycle");
    ok(&d, &["init"]);
    let id = ok(
        &d,
        &["add", "feat", "--body", "the description", "--approved"],
    )
    .trim()
    .to_string();
    ok(&d, &["claim", &id, "--by", "agent"]);
    ok(&d, &["review", &id, "--summary", "round one"]);
    ok(&d, &["rework", &id, "--reason", "fix tests"]);
    let text = fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap();
    let i = jig::issue::decode(&text).unwrap();
    assert_eq!(i.status, jig::issue::Status::Claimed);
    assert_eq!(i.assignee.as_deref(), Some("agent"));
    assert!(i.claim_id.is_some());
    ok(&d, &["review", &id, "--summary", "round two"]);
    ok(
        &d,
        &[
            "close",
            &id,
            "--reason",
            "verified",
            "--summary",
            "all good",
        ],
    );
    let path = d.join(format!(".jig/closed/{id}.md"));
    let text = fs::read_to_string(&path).unwrap();
    let i = jig::issue::decode(&text).unwrap();
    assert_eq!(i.closing_summary.as_deref(), Some("all good"));
    assert!(i.body.contains("the description"));
    assert_eq!(i.assignee.as_deref(), Some("agent"));
    assert!(i.claim_id.is_none());
    let h = &i.history;
    assert!(h.contains("round one"));
    assert!(h.contains("fix tests"));
    assert!(h.contains("round two"));
    assert!(h.contains("closed: verified"));
    let summary_pos = text.find("## Closing Summary").unwrap();
    let body_pos = text.find("the description").unwrap();
    assert!(summary_pos < body_pos, "summary should precede body");

    let e = bad(&d, &["close", &id, "--reason", "again"]);
    assert!(e.contains("already closed"), "{e}");

    ok(&d, &["note", &id, "audit trail"]);
    ok(&d, &["reopen", &id, "--reason", "regression"]);
    let path = d.join(format!(".jig/open/{id}.md"));
    let i = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(i.status, jig::issue::Status::Open);
    assert!(i.closing_summary.is_none());
    assert!(i.history.contains("all good"));
    assert!(i.notes.contains("audit trail"));
    assert!(i.body.contains("the description"));
    assert!(i.assignee.is_none());
    assert!(i.closed_at.is_none());
    let e = bad(&d, &["reopen", &id, "--reason", "x"]);
    assert!(e.contains("already open"), "{e}");
}

#[test]
fn close_without_review() {
    let d = dir("nodereview");
    ok(&d, &["init"]);
    let id = add(&d, "trivial");
    ok(&d, &["close", &id, "--reason", "obsolete"]);
    assert!(d.join(format!(".jig/closed/{id}.md")).is_file());
}

#[test]
fn move_journal_recovery() {
    let d = dir("journal");
    ok(&d, &["init"]);
    let old = fm("B-abc123", "open", "");
    let new = fm("B-abc123", "closed", "").replacen(
        "## History\n\n",
        "## History\n\n- 2026-01-01 00:00:00 UTC closed: done\n\n",
        1,
    );
    let src = d.join(".jig/open/B-abc123.md");
    let dst = d.join(".jig/closed/B-abc123.md");

    fs::write(&src, &old).unwrap();
    fs::write(d.join(".jig/.move"), journal(&old, &new)).unwrap();
    ok(&d, &["list", "--status", "closed"]);
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), new);
    assert!(!d.join(".jig/.move").exists());

    fs::remove_file(&dst).unwrap();
    fs::write(&dst, &old).unwrap();
    fs::write(d.join(".jig/.move"), journal(&old, &new)).unwrap();
    ok(&d, &["list", "--status", "closed"]);
    assert_eq!(fs::read_to_string(&dst).unwrap(), new);
    assert!(!d.join(".jig/.move").exists());

    fs::write(&src, "stale").unwrap();
    fs::write(d.join(".jig/.move"), journal(&old, &new)).unwrap();
    let e = bad(&d, &["ready"]);
    assert!(e.contains("journal") || e.contains("resolve"), "{e}");
    assert!(d.join(".jig/.move").exists());
    fs::remove_file(&src).unwrap();
    fs::write(d.join(".jig/.move"), journal(&old, &new)).unwrap();
    ok(&d, &["ready"]);
    assert!(!d.join(".jig/.move").exists());
}

#[test]
fn notes_bounds() {
    let d = dir("notes");
    ok(&d, &["init"]);
    let id = add(&d, "n");
    let long = "ą".repeat(500);
    ok(&d, &["note", &id, &long, "--by", "qa"]);
    let over = "x".repeat(501);
    let e = bad(&d, &["note", &id, &over]);
    assert!(e.contains("501") && e.contains("maximum is 500"), "{e}");
    let e = bad(&d, &["note", &id, "a\nb"]);
    assert!(e.contains("single line"), "{e}");
    let e = bad(&d, &["note", &id, ""]);
    assert!(e.contains("empty"), "{e}");
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert!(i.notes.contains(&long));
    assert!(i.notes.contains("qa:"));
}

#[test]
fn body_modes_and_update() {
    let d = dir("body");
    ok(&d, &["init"]);
    let id = ok(&d, &["add", "t", "--body", "literal body"])
        .trim()
        .to_string();
    let o = run_stdin(&d, &["update", &id, "--body", "-"], "piped\nbody\n");
    assert!(o.status.success());
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert_eq!(i.body, "piped\nbody\n", "body bytes preserved exactly");
    ok(&d, &["update", &id, "--title", "retitled"]);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert_eq!(i.title, "retitled");
    assert_eq!(i.body, "piped\nbody\n");
    bad(&d, &["update", &id]);
}

#[test]
fn json_wire_and_no_leaks() {
    let d = dir("json");
    ok(&d, &["init"]);
    let marker = "SECRET-BODY-žluťoučký \"quoted\" `tick`";
    let id = ok(&d, &["add", "t \"dq\" pol", "--body", marker, "--approved"])
        .trim()
        .to_string();
    let r = ok(&d, &["ready"]);
    assert!(!r.contains("SECRET-BODY"));
    let s = ok(&d, &["start"]);
    assert!(!s.contains("SECRET-BODY"));
    let j = ok(&d, &["ready", "--json"]);
    assert!(j.starts_with('[') && j.contains(&format!("\"id\":\"{id}\"")));
    let j = ok(&d, &["--json", "ready"]);
    assert!(j.starts_with('['));
    let sh = ok(&d, &["show", &id, "--json"]);
    assert!(sh.contains("SECRET-BODY"));
    assert!(sh.contains("\\\""), "quotes must be escaped: {sh}");
    assert!(!sh.contains("claim_id"), "{sh}");
    let sh_h = ok(&d, &["show", &id]);
    assert!(sh_h.contains(marker));
    let dec =
        jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
            .unwrap();
    assert_eq!(dec.title, "t \"dq\" pol");
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
fn malformed_and_duplicates_refuse() {
    let d = dir("malformed");
    ok(&d, &["init"]);
    let id = add(&d, "fine");
    let path = d.join(format!(".jig/open/{id}.md"));
    let good = fs::read_to_string(&path).unwrap();

    fs::write(
        d.join(".jig/open/B-dead00.md"),
        good.replace(&id, "B-dead00")
            .replace("status: open", "bogus: x\nstatus: open"),
    )
    .unwrap();
    let o = run(&d, &["list"]);
    assert!(o.status.success(), "list tolerates one bad file");
    assert!(String::from_utf8_lossy(&o.stdout).contains(&id));
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains("unknown frontmatter key") && warn.contains("jig check"),
        "{warn}"
    );
    let e = bad(&d, &["show", "B-dead00"]);
    assert!(e.contains("unknown frontmatter key"), "{e}");
    fs::remove_file(d.join(".jig/open/B-dead00.md")).unwrap();

    let closed_copy = good.replace("status: open", "status: closed");
    fs::write(d.join(".jig/closed").join(format!("{id}.md")), &closed_copy).unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let o = run(&d, &["list"]);
    assert!(o.status.success(), "list tolerates a duplicated file");
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains("duplicate") && warn.contains("jig check"),
        "{warn}"
    );
    let o = run(&d, &["ready"]);
    assert!(o.status.success());
    assert!(o.stdout.is_empty());
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains("duplicate") && warn.contains("jig check"),
        "{warn}"
    );
    let e = bad(&d, &["claim", &id]);
    assert!(e.contains("duplicate"), "{e}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "no mutation on failure"
    );
    fs::remove_file(d.join(".jig/closed").join(format!("{id}.md"))).unwrap();

    let conflicted = good.replace(
        "## Notes\n",
        "<<<<<<< HEAD\nfoo\n=======\nbar\n>>>>>>> other\n\n## Notes\n",
    );
    fs::write(&path, &conflicted).unwrap();
    let o = run(&d, &["list"]);
    assert!(o.status.success(), "list tolerates a conflicted file");
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains("conflict") && warn.contains("jig check"),
        "{warn}"
    );
    let e = bad(&d, &["show", &id]);
    assert!(e.contains("conflict"), "{e}");
    fs::write(&path, &good).unwrap();
    ok(&d, &["list"]);

    let e = bad(
        &d,
        &[
            "update",
            &id,
            "--body",
            "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> branch",
        ],
    );
    assert!(e.contains("conflict"), "{e}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        good,
        "rejected body leaves file unchanged"
    );
    ok(&d, &["list"]);
}

#[test]
fn codec_domain() {
    let d = dir("codec");
    ok(&d, &["init"]);
    let body = "para\n\n```\n## History\n## Notes\n```\n\n> ## Notes\n\ntail";
    let id = ok(&d, &["add", "t", "--body", body, "--approved"])
        .trim()
        .to_string();
    let text = fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap();
    let i = jig::issue::decode(&text).unwrap();
    assert_eq!(
        i.body, body,
        "fenced and blockquoted reserved headings stay body"
    );

    let path = d.join(format!(".jig/open/{id}.md"));
    let plain = text.replace("title: \"t\"", "title: plain #tag");
    fs::write(&path, &plain).unwrap();
    let e = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap_err();
    assert!(e.contains("title"), "plain scalar must name the field: {e}");
    for (field, value) in [
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
    ] {
        let raw = text
            .lines()
            .find(|l| l.starts_with(&format!("{field}:")))
            .unwrap();
        let replaced = text.replacen(raw, &format!("{field}: {value}"), 1);
        fs::write(&path, &replaced).unwrap();
        let e = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap_err();
        assert!(
            e.contains(field),
            "{field}: {value} must name the field, got {e}"
        );
        fs::write(&path, &text).unwrap();
    }
    let depends = text.replace("depends_on: []", "depends_on: [B1]");
    fs::write(&path, &depends).unwrap();
    let e = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap_err();
    assert!(
        e.contains("depends_on"),
        "bare list element must name the field: {e}"
    );
    fs::write(&path, &text).unwrap();

    let escaped = text.replace("title: \"t\"", "title: \"a\\u0142b\"");
    fs::write(&path, &escaped).unwrap();
    let i = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(i.title, "a\u{0142}b");
    fs::write(&path, &text).unwrap();

    let legacy: String = text
        .lines()
        .map(|l| {
            if l.starts_with("created_at:") {
                "created_at: \"2020-05-04\"".to_string()
            } else if l.starts_with("priority:") {
                "priority: null".to_string()
            } else if l == format!("id: \"{id}\"") {
                "id: \"B440.2\"".to_string()
            } else if l == "parent: null" {
                "parent: \"B440\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(d.join(".jig/open/B440.2.md"), &legacy).unwrap();
    let i =
        jig::issue::decode(&fs::read_to_string(d.join(".jig/open/B440.2.md")).unwrap()).unwrap();
    assert_eq!(i.id, "B440.2");
    assert_eq!(i.priority, None);
    assert_eq!(i.created_at.as_deref(), Some("2020-05-04"));

    assert!(jig::issue::valid_id("B-abcdef"));
    assert!(jig::issue::valid_id("B-abcdef.2.10"));
    assert!(jig::issue::valid_id("B440"));
    assert!(!jig::issue::valid_id("B-abcdef.0"));
    assert!(!jig::issue::valid_id("B-abcdef.02"));
    assert!(!jig::issue::valid_id("abcdef"));
    assert!(!jig::issue::valid_id("B-ABC123"));
    assert_eq!(jig::issue::id_alias("abcdef").as_deref(), Some("B-abcdef"));
    assert_eq!(
        jig::issue::id_alias("abcdef.2").as_deref(),
        Some("B-abcdef.2")
    );
}

#[test]
fn body_exact_preservation() {
    let d = dir("bodyexact");
    ok(&d, &["init"]);
    let body = "\n````md\n```\n## History\n```\n````\n\ntrailing\n\n";
    let o = run_stdin(&d, &["add", "t", "--body", "-", "--approved"], body);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let id = String::from_utf8(o.stdout).unwrap().trim().to_string();
    let read = |dirn: &str| {
        jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/{dirn}/{id}.md"))).unwrap())
            .unwrap()
    };
    assert_eq!(read("open").body, body, "add preserves body bytes");
    ok(&d, &["update", &id, "--title", "t2"]);
    assert_eq!(read("open").body, body, "update preserves body bytes");
    ok(&d, &["close", &id, "--reason", "done"]);
    assert_eq!(read("closed").body, body, "close preserves body bytes");

    let count = || fs::read_dir(d.join(".jig/open")).unwrap().count();
    let before = count();
    let o = run_stdin(&d, &["add", "bad1", "--body", "-"], "text\n\n## History\n");
    assert!(
        !o.status.success(),
        "reserved heading outside fence accepted"
    );
    let o = run_stdin(&d, &["add", "bad2", "--body", "-"], "ok\n\n```\nunclosed");
    assert!(!o.status.success(), "unclosed fence accepted");
    assert_eq!(count(), before, "refused issues must not be written");
}

#[test]
fn stdin_body_does_not_hold_lock() {
    let d = dir("stdlock");
    ok(&d, &["init"]);
    let id = add(&d, "t");
    let mut ch = Command::new(env!("CARGO_BIN_EXE_jig"))
        .args(["update", &id, "--body", "-"])
        .current_dir(&d)
        .env("XDG_STATE_HOME", d.join("xdg"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut feed = ch.stdin.take().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut rd = spawn(&d, &["ready"]);
    let mut done = false;
    for _ in 0..40 {
        if rd.try_wait().unwrap().is_some() {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !done {
        let _ = rd.kill();
        feed.write_all(b"\n").unwrap();
        drop(feed);
        let _ = ch.wait();
        panic!("ready blocked for 2s behind a stdin-reading update");
    }
    let rd_out = rd.wait_with_output().unwrap();
    assert!(rd_out.status.success());
    feed.write_all(b"fresh body").unwrap();
    drop(feed);
    let out = ch.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert_eq!(i.body, "fresh body");
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
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert_eq!(i.depends_on, vec![a.clone(), b.clone(), c]);
    let e = bad(&d, &["add", "x", "--dep", &a, &a]);
    assert!(e.contains("duplicate"), "{e}");
    let e = bad(&d, &["add", "x", "--dep", "not-an-id!!"]);
    assert!(e.contains("invalid issue id"), "{e}");
    let e = bad(&d, &["add", "x", "--dep"]);
    assert!(e.contains("missing value"), "{e}");
}

#[test]
fn corrupt_journal_lengths_refuse() {
    let d = dir("badjournal");
    ok(&d, &["init"]);
    let old = fm("B-abc123", "open", "");
    fs::write(d.join(".jig/open/B-abc123.md"), &old).unwrap();
    fs::write(
        d.join(".jig/.move"),
        "JIG-MOVE-1\n18446744073709551615\n2\nxy",
    )
    .unwrap();
    let e = bad(&d, &["ready"]);
    assert!(e.contains("journal") || e.contains("corrupt"), "{e}");
    assert!(d.join(".jig/.move").exists(), "journal must be preserved");
    fs::remove_file(d.join(".jig/.move")).unwrap();
    fs::remove_file(d.join(".jig/open/B-abc123.md")).unwrap();
    ok(&d, &["ready"]);
}

#[test]
fn memory_blank_lines_and_quotes() {
    let d = dir("membl");
    ok(&d, &["init"]);
    let value = "first\n\n> example\nthird";
    ok(&d, &["remember", "k", value]);
    assert_eq!(ok(&d, &["recall", "k"]), format!("{value}\n"));
    let s = ok(&d, &["start"]);
    assert!(!s.contains("first"), "workflow text must not leak memory");
}

#[test]
fn start_json_and_help() {
    let d = dir("meta");
    ok(&d, &["init"]);
    let j = ok(&d, &["start", "--json"]);
    assert!(j.starts_with("{\"workflow\":"));
    let v = run(&d, &["--version"]);
    assert!(v.status.success());
    let help = ok(&d, &["help"]);
    for args in [&[][..], &["--help"][..], &["-h"][..]] {
        assert_eq!(ok(&d, args), help);
    }
    for name in ["approve", "revise", "rework"] {
        assert!(help.contains(name), "help missing {name}");
    }
    let e = bad(&d, &["claim"]);
    assert!(e.contains("claim ID"), "{e}");
    let e = bad(&d, &["ready", "--bogus"]);
    assert!(e.contains("unknown option"), "{e}");
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
    std::os::unix::fs::symlink(real.join(".jig"), linked.join(".jig")).unwrap();
    let e = bad(&linked, &["ready"]);
    assert!(e.contains("symbolic link"), "{e}");
    let e = bad(&linked, &["add", "x"]);
    assert!(e.contains("symbolic link"), "{e}");
    let e = bad(&linked, &["init"]);
    assert!(e.contains("symbolic link"), "{e}");
    assert_eq!(
        fs::read_dir(real.join(".jig/open")).unwrap().count(),
        1,
        "linked registry untouched"
    );

    let d2 = dir("linkopen");
    ok(&d2, &["init"]);
    fs::remove_dir(d2.join(".jig/open")).unwrap();
    let victim = d2.join("victim");
    fs::create_dir(&victim).unwrap();
    std::os::unix::fs::symlink(&victim, d2.join(".jig/open")).unwrap();
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
    let open = d.join(format!(".jig/open/{id}.md"));
    let closed = d.join(format!(".jig/closed/{id}.md"));
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(&open, fs::Permissions::from_mode(0o600)).unwrap();
    ok(&d, &["update", &id, "--title", "p2"]);
    assert_eq!(mode(&open), 0o600, "update must preserve mode");
    ok(&d, &["close", &id, "--reason", "done", "--summary", "s"]);
    assert_eq!(mode(&closed), 0o600, "close must preserve mode");
    ok(&d, &["reopen", &id, "--reason", "again"]);
    assert_eq!(mode(&open), 0o600, "reopen must preserve mode");
}

#[test]
fn reopen_archives_summary_verbatim() {
    let d = dir("reopensum");
    ok(&d, &["init"]);
    let id = add(&d, "s");
    let path = d.join(format!(".jig/open/{id}.md"));
    let before = fs::read_to_string(&path).unwrap();
    let e = bad(
        &d,
        &[
            "close",
            &id,
            "--reason",
            "done",
            "--summary",
            "Recommended workaround:\n\n```text\ntranscript line\n",
        ],
    );
    assert!(
        e.contains("--summary") && e.contains("unclosed code fence"),
        "{e}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        before,
        "rejected summary leaves issue bytes unchanged"
    );
    let summary = "Recommended workaround:\n\n```text\ntranscript line\n```";
    ok(
        &d,
        &["close", &id, "--reason", "done", "--summary", summary],
    );
    let closed = d.join(format!(".jig/closed/{id}.md"));
    let i = jig::issue::decode(&fs::read_to_string(&closed).unwrap()).unwrap();
    assert_eq!(i.closing_summary.as_deref(), Some(summary));
    ok(&d, &["reopen", &id, "--reason", "regression"]);
    let open = d.join(format!(".jig/open/{id}.md"));
    let i = jig::issue::decode(&fs::read_to_string(&open).unwrap()).unwrap();
    assert!(i.closing_summary.is_none());
    let tail = i
        .history
        .split("\n  closing summary:\n")
        .nth(1)
        .expect("closing summary history block missing");
    let quoted = tail
        .split('\n')
        .map(|line| line.strip_prefix("  > ").unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(quoted, summary);
    assert!(i.history.contains("closed: done"));
    assert!(!d.join(".jig/.move").exists(), "journal cleaned up");
}

#[test]
fn dep_rm_tolerates_missing_blocker() {
    let d = dir("deprm");
    ok(&d, &["init"]);
    let a = add(&d, "a");
    let b = add(&d, "b");
    ok(&d, &["dep", "add", &a, &b]);
    fs::rename(
        d.join(format!(".jig/open/{b}.md")),
        d.join("blocker-outside.md"),
    )
    .unwrap();
    let o = run(&d, &["ready"]);
    assert!(o.status.success(), "ready must tolerate a missing blocker");
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains(&a) && warn.contains(&b) && warn.contains("jig check"),
        "{warn}"
    );
    let e = bad(&d, &["claim", &a, "--force"]);
    assert!(e.contains(&b), "{e}");
    ok(&d, &["dep", "rm", &a, &b]);
    let out = ok(&d, &["ready"]);
    assert!(out.contains(&a), "{out}");
    let e = bad(&d, &["dep", "add", &a, &b]);
    assert!(e.contains("not found"), "{e}");
}

#[cfg(target_os = "linux")]
#[test]
fn stdout_write_failure_exits_one() {
    let d = dir("full");
    ok(&d, &["init"]);
    let id = add(&d, "s");
    let devfull = fs::File::options().write(true).open("/dev/full").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_jig"))
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
        json!({"hooks":[{"type":"command","command":"jig start --hook-json","timeout":5}]});

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

    assert_eq!(ok(&proj, &["init"]), "initialized .jig/\n");
    let agents = fs::read_to_string(proj.join("AGENTS.md")).unwrap();
    assert!(agents.starts_with(agents_orig), "original bytes preserved");
    assert_eq!(agents.matches("<!-- BEGIN JIG v:1 hash:").count(), 1);
    assert_eq!(agents.matches("<!-- END JIG -->").count(), 1);
    assert!(agents.contains("jig claim ID") && agents.contains("jig recall"));

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
        !out.contains("classified") && !out.contains("secret body") && !out.contains(&id),
        "hook payload must carry only the static workflow"
    );

    let journal = "JIG-MOVE-1\n18446744073709551615\n2\nxy";
    fs::write(d.join(".jig/.move"), journal).unwrap();
    let out2 = ok(&nested, &["start", "--hook-json"]);
    assert!(out2.contains("SessionStart"));
    assert_eq!(
        fs::read_to_string(d.join(".jig/.move")).unwrap(),
        journal,
        "hook must not recover or mutate the journal"
    );
    fs::remove_file(d.join(".jig/.move")).unwrap();

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
        let text = fm(id, status, "")
            .replace("priority: 2", &format!("priority: {pri}"))
            .replace("created_at: null", &format!("created_at: {created}"));
        fs::write(d.join(format!(".jig/{place}/{id}.md")), text).unwrap();
    };
    seed("B100", "open", "3", "\"2026-04-03\"");
    seed("B101", "open", "1", "\"2026-04-12T06:30:00Z\"");
    seed("B102", "open", "4", "\"2026-04-12T06:30:00Z\"");
    seed("B103", "open", "0", "null");
    seed("B104", "closed", "2", "\"2026-03-01\"");

    let ids = |out: &str| -> Vec<String> {
        out.lines()
            .map(|l| l.split_whitespace().next().unwrap().to_string())
            .collect()
    };
    let def = ok(&d, &["list"]);
    assert_eq!(ids(&def), ["B103", "B101", "B100", "B102"]);
    assert_eq!(def.lines().next().unwrap(), "B103 open 0 - T B103");
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
        json!({"id":"B104","title":"T B104","status":"closed","priority":2,"assignee":null})
    );

    let e = bad(&d, &["list", "--sort", "closing"]);
    assert!(e.contains("oldest or newest"), "{e}");
}

fn seed_status(d: &TestDir, id: &str, status: &str) -> PathBuf {
    let mut text = fm(id, status, "");
    match status {
        "claimed" | "review" => {
            text = text
                .replace("assignee: null", "assignee: \"bot\"")
                .replace("claim_id: null", "claim_id: \"c1\"");
        }
        "closed" => {
            text = text.replace("closed_at: null", "closed_at: \"2026-01-01\"");
        }
        _ => {}
    }
    let place = if status == "closed" { "closed" } else { "open" };
    for stale in ["open", "closed"] {
        let _ = fs::remove_file(d.join(format!(".jig/{stale}/{id}.md")));
    }
    let p = d.join(format!(".jig/{place}/{id}.md"));
    fs::write(&p, &text).unwrap();
    p
}

#[test]
fn lifecycle_transition_matrix() {
    let d = dir("matrix");
    ok(&d, &["init", "--no-hooks"]);
    let statuses = ["proposed", "open", "claimed", "review", "closed"];
    // (command argv suffix, allowed from -> to status, history suffix, dir after)
    type Allowed = (&'static str, &'static str, &'static str);
    let actions: &[(&[&str], &[Allowed])] = &[
        (&["approve"], &[("proposed", "open", "approved")]),
        (
            &["revise", "--reason", "r"],
            &[("open", "proposed", "revised: r")],
        ),
        (&["claim"], &[("open", "claimed", "")]),
        (&["claim", "--force"], &[("open", "claimed", "")]),
        (&["unclaim"], &[("claimed", "open", "")]),
        (
            &["review"],
            &[("claimed", "review", "submitted for review by bot")],
        ),
        (
            &["rework", "--reason", "r"],
            &[("review", "claimed", "rework requested: r")],
        ),
        (
            &["close", "--reason", "r"],
            &[
                ("proposed", "closed", "closed: r"),
                ("open", "closed", "closed: r"),
                ("claimed", "closed", "closed: r"),
                ("review", "closed", "closed: r"),
            ],
        ),
        (
            &["reopen", "--reason", "r"],
            &[("closed", "open", "reopened: r")],
        ),
    ];
    for (argv, allowed) in actions {
        for from in statuses {
            let id = "B-a10000";
            let path = seed_status(&d, id, from);
            let before = fs::read_to_string(&path).unwrap();
            let mut args: Vec<&str> = vec![argv[0], id];
            args.extend(&argv[1..]);
            let outcome = allowed.iter().find(|(s, _, _)| *s == from);
            match outcome {
                Some((_, to, hist)) => {
                    let out = ok(&d, &args);
                    assert!(!out.is_empty());
                    let dir_after = if *to == "closed" { "closed" } else { "open" };
                    let i = jig::issue::decode(
                        &fs::read_to_string(d.join(format!(".jig/{dir_after}/{id}.md"))).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(i.status.as_str(), *to, "{from} + {argv:?} must reach {to}");
                    if hist.is_empty() {
                        assert_eq!(i.history, "", "{from} + {argv:?} must not add history");
                    } else {
                        assert!(
                            i.history.ends_with(hist),
                            "{from} + {argv:?} history must end with {hist:?}, got {:?}",
                            i.history
                        );
                    }
                    assert_eq!(
                        ok(&d, &["check", "--json"]).trim(),
                        "[]",
                        "{from} + {argv:?} -> {to} must leave a clean registry"
                    );
                }
                None => {
                    let e = bad(&d, &args);
                    assert!(!e.is_empty());
                    assert_eq!(
                        fs::read_to_string(&path).unwrap(),
                        before,
                        "{from} + {argv:?} refusal must not change bytes"
                    );
                }
            }
        }
    }
    fs::write(
        d.join(".jig/open/B-a10000.md"),
        fm("B-a10000", "proposed", ""),
    )
    .unwrap();
    let e = bad(&d, &["claim", "B-a10000"]);
    assert!(e.contains("approval"), "{e}");
    let e = bad(&d, &["claim", "B-a10000", "--force"]);
    assert!(e.contains("approval"), "{e}");
}

#[test]
fn proposals_are_not_work_and_do_not_cascade() {
    let d = dir("proposals");
    ok(&d, &["init", "--no-hooks"]);
    let p = ok(&d, &["add", "p"]).trim().to_string();
    let c = ok(&d, &["add", "c", "--parent", &p]).trim().to_string();
    let pc = ok(&d, &["add", "pc", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    let read_status = |id: &str| {
        let name = format!(".jig/open/{id}.md");
        jig::issue::decode(&fs::read_to_string(d.join(name)).unwrap())
            .unwrap()
            .status
    };
    assert_eq!(read_status(&p), jig::issue::Status::Proposed);
    assert_eq!(read_status(&c), jig::issue::Status::Proposed);
    assert_eq!(read_status(&pc), jig::issue::Status::Open);

    let r = ok(&d, &["ready"]);
    let ready_ids: Vec<&str> = r
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(
        ready_ids.contains(&pc.as_str())
            && !ready_ids.contains(&p.as_str())
            && !ready_ids.contains(&c.as_str()),
        "{r}"
    );
    let e = bad(&d, &["claim", &p]);
    assert!(e.contains("approval"), "{e}");
    let e = bad(&d, &["claim", &p, "--force"]);
    assert!(e.contains("approval"), "{e}");

    ok(&d, &["approve", &p]);
    assert_eq!(read_status(&p), jig::issue::Status::Open);
    assert_eq!(
        read_status(&c),
        jig::issue::Status::Proposed,
        "approve must not cascade to children"
    );
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{p}.md"))).unwrap())
        .unwrap();
    assert!(i.history.ends_with("approved"), "{:?}", i.history);

    ok(&d, &["revise", &p, "--reason", "scope unclear"]);
    assert_eq!(read_status(&p), jig::issue::Status::Proposed);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{p}.md"))).unwrap())
        .unwrap();
    assert!(i.history.ends_with("revised: scope unclear"));

    let w = add(&d, "w");
    let b = ok(&d, &["add", "b"]).trim().to_string();
    ok(&d, &["dep", "add", &w, &b]);
    let e = bad(&d, &["claim", &w]);
    assert!(e.contains("blocked"), "{e}");
    ok(&d, &["claim", &w, "--force", "--by", "agent"]);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{w}.md"))).unwrap())
        .unwrap();
    assert!(i.history.contains(&format!("bypassing {b}")));
}

#[test]
fn force_bypasses_every_unfinished_blocker_status() {
    let d = dir("forceall");
    ok(&d, &["init", "--no-hooks"]);
    let w = add(&d, "w");
    for (i, status) in ["proposed", "claimed", "review"].iter().enumerate() {
        let bid = format!("B-f{i:05}");
        let mut text = fm(&bid, status, "");
        if *status != "proposed" {
            text = text
                .replace("assignee: null", "assignee: \"bot\"")
                .replace("claim_id: null", "claim_id: \"c1\"");
        }
        fs::write(d.join(format!(".jig/open/{bid}.md")), text).unwrap();
        ok(&d, &["dep", "add", &w, &bid]);
    }
    let e = bad(&d, &["claim", &w]);
    assert!(e.contains("blocked"), "{e}");
    ok(&d, &["claim", &w, "--force"]);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{w}.md"))).unwrap())
        .unwrap();
    for bid in ["B-f00000", "B-f00001", "B-f00002"] {
        assert!(i.history.contains(bid), "{:?}", i.history);
    }
    assert_eq!(i.status, jig::issue::Status::Claimed);
}

#[test]
fn unclaim_and_status_filters() {
    let d = dir("filters");
    ok(&d, &["init", "--no-hooks"]);
    let prop = ok(&d, &["add", "p"]).trim().to_string();
    let open = add(&d, "o");
    let claimed = add(&d, "c");
    ok(&d, &["claim", &claimed, "--by", "bot"]);
    let out = ok(&d, &["unclaim", &claimed]);
    assert!(out.contains("unclaimed"));
    ok(&d, &["claim", &claimed]);
    let review = add(&d, "r");
    ok(&d, &["claim", &review]);
    ok(&d, &["review", &review]);
    let closed = add(&d, "z");
    ok(&d, &["close", &closed, "--reason", "done"]);

    let ids_of = |args: &[&str]| -> Vec<String> {
        let j: Value = serde_json::from_str(&ok(&d, args)).unwrap();
        j.as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        ids_of(&["list", "--status", "proposed", "--json"]),
        vec![prop.clone()]
    );
    assert_eq!(
        ids_of(&["list", "--status", "open", "--json"]),
        vec![open.clone()]
    );
    assert_eq!(
        ids_of(&["list", "--status", "claimed", "--json"]),
        vec![claimed.clone()]
    );
    assert_eq!(
        ids_of(&["list", "--status", "review", "--json"]),
        vec![review.clone()]
    );
    assert_eq!(
        ids_of(&["list", "--status", "closed", "--json"]),
        vec![closed.clone()]
    );
    let all = ids_of(&["list", "--status", "all", "--json"]);
    assert_eq!(all.len(), 5);
    let active = ids_of(&["list", "--json"]);
    assert!(
        active.contains(&prop) && !active.contains(&closed),
        "default active filter must include proposed: {active:?}"
    );
    let e = bad(&d, &["list", "--status", "in_progress"]);
    assert!(e.contains("invalid status filter"), "{e}");
    let e = bad(&d, &["unclaim", &open]);
    assert!(e.contains("cannot unclaim"), "{e}");
}

#[test]
fn structural_and_marker_roundtrip() {
    let d = dir("roundtrip");
    ok(&d, &["init", "--no-hooks"]);
    let read_body = |id: &str| {
        jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
            .unwrap()
            .body
    };
    let accepted: &[&str] = &[
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
    ];
    for (n, body) in accepted.iter().enumerate() {
        let name = format!("ok{n}");
        let o = run_stdin(&d, &["add", &name, "--body", "-", "--approved"], body);
        assert!(
            o.status.success(),
            "case {n} rejected: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let id = String::from_utf8(o.stdout).unwrap().trim().to_string();
        assert_eq!(&read_body(&id), body, "case {n} bytes must round-trip");
    }
    let rejected: &[&str] = &[
        "real conflict\n\n<<<<<<< HEAD\na\n=======\nb\n>>>>>>> branch\n",
        "indented reserved\n  ## History\n",
        "closing hashes\n## Notes ###\n",
        "premature end then triplet\n>>>>>>> z\n\n<<<<<<< x\n=======\n>>>>>>> y\n",
    ];
    let count = || fs::read_dir(d.join(".jig/open")).unwrap().count();
    for (n, body) in rejected.iter().enumerate() {
        let before = count();
        let name = format!("bad{n}");
        let o = run_stdin(&d, &["add", &name, "--body", "-"], body);
        assert!(
            !o.status.success(),
            "rejected case {n} accepted: {}",
            String::from_utf8_lossy(&o.stdout)
        );
        assert_eq!(count(), before, "rejected case {n} wrote a file");
    }
}

#[test]
fn multiline_reasons_and_summaries() {
    let d = dir("multiline");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "m");

    let reason = "first line\n\nsecond para\nthird";
    let mut i;
    let path = d.join(format!(".jig/open/{id}.md"));
    let before = fs::read_to_string(&path).unwrap();
    let e = bad(&d, &["revise", &id, "--reason", "has\ttab"]);
    assert!(e.contains("--reason") && e.contains("control"), "{e}");
    let e = bad(&d, &["revise", &id, "--reason", "```\nunclosed"]);
    assert!(e.contains("--reason") && e.contains("unclosed"), "{e}");
    let e = bad(&d, &["revise", &id, "--reason", "x\n## Notes"]);
    assert!(e.contains("--reason") && e.contains("reserved"), "{e}");
    assert_eq!(fs::read_to_string(&path).unwrap(), before);

    ok(&d, &["revise", &id, "--reason", reason]);
    i = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(i.status, jig::issue::Status::Proposed);
    let tail = i
        .history
        .split("\n  > ")
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(tail, reason, "multiline reason must decode verbatim");
    ok(&d, &["approve", &id]);

    ok(&d, &["claim", &id]);
    let summary = "line one\n\n```text\nfenced\n```";
    ok(&d, &["review", &id, "--summary", summary]);
    i = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    let part = i
        .history
        .split("\n  summary:\n")
        .nth(1)
        .expect("multiline summary block");
    let recovered = part
        .split('\n')
        .map(|l| l.strip_prefix("  > ").unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(recovered, summary);

    ok(&d, &["rework", &id, "--reason", "more\nlines"]);
    i = jig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(i.history.contains("rework requested:\n  > more\n  > lines"));

    ok(
        &d,
        &["close", &id, "--reason", reason, "--summary", summary],
    );
    let closed_path = d.join(format!(".jig/closed/{id}.md"));
    i = jig::issue::decode(&fs::read_to_string(&closed_path).unwrap()).unwrap();
    assert_eq!(i.closing_summary.as_deref(), Some(summary));
    let part = i
        .history
        .split("\n- ")
        .find(|e| e.contains("closed:"))
        .expect("closed entry");
    let recovered = part.split("\n  > ").skip(1).collect::<Vec<_>>().join("\n");
    assert_eq!(recovered.trim_end(), reason);
}

#[test]
fn option_parsing_edges() {
    let d = dir("optedges");
    ok(&d, &["init", "--no-hooks"]);
    let e = bad(&d, &["--", "--help"]);
    assert!(e.contains("unknown command"), "{e}");
    let out = ok(&d, &["--", "list", "--json"]);
    assert_eq!(out, "[]\n");
    let e = bad(&d, &["--"]);
    assert!(e.contains("missing command"), "{e}");
    let e = bad(&d, &["--json"]);
    assert!(e.contains("missing command"), "{e}");

    for v in ["+2", " 2", "02", "5", "2x"] {
        let e = bad(&d, &["add", "t", "-p", v, "--approved"]);
        assert!(e.contains("priority"), "{v}: {e}");
        let e = bad(&d, &["add", "t", "--priority", v]);
        assert!(e.contains("priority"), "{v}: {e}");
    }
    for (n, v) in ["0", "1", "2", "3", "4"].iter().enumerate() {
        let id = ok(&d, &["add", &format!("p{n}"), "-p", v, "--approved"])
            .trim()
            .to_string();
        let i =
            jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
                .unwrap();
        assert_eq!(i.priority, Some(v.parse::<u8>().unwrap()));
    }
    let id = add(&d, "u");
    let e = bad(&d, &["update", &id, "-p", "9"]);
    assert!(e.contains("priority"), "{e}");
    let e = bad(&d, &["request-changes", &id]);
    assert!(e.contains("unknown command"), "{e}");
}

#[test]
fn note_rejects_separators_and_control() {
    let d = dir("notectl");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "n");
    let file = d.join(format!(".jig/open/{id}.md"));
    let before = fs::read(&file).unwrap();
    for bad_text in ["a\tb", "a\u{7f}b", "a\u{85}b", "a\u{2028}b", "a\u{2029}b"] {
        let e = bad(&d, &["note", &id, bad_text]);
        assert!(e.contains("single line"), "{bad_text:?}: {e}");
    }
    let e = bad(&d, &["note", &id, "ok", "--by", "a\u{2028}b"]);
    assert!(e.contains("--by"), "{e}");
    assert_eq!(
        fs::read(&file).unwrap(),
        before,
        "refused notes write nothing"
    );
    let i = jig::issue::decode(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(i.notes.is_empty());
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
    let o = Command::new(env!("CARGO_BIN_EXE_jig"))
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
    let jig = d.join(".jig");
    fs::create_dir_all(jig.join("open")).unwrap();
    fs::create_dir_all(jig.join("closed")).unwrap();
    fs::write(jig.join("memory.md"), "# Project Memory\n").unwrap();
    fs::write(jig.join(".gitignore"), "/.lock\n/.move\n.*.tmp-*\n").unwrap();
    jig
}

#[test]
fn check_reports_every_problem_without_changes() {
    let d = dir("checkall");
    let jig = seed_bare_registry(&d);
    fs::write(jig.join("memory.md"), "junk\n").unwrap();
    fs::write(jig.join(".gitignore"), "/.lock\n/.move\n/.*.tmp-*\n").unwrap();

    fs::write(jig.join("open/B-00a001.md"), b"\xff\xfe garbage").unwrap();
    fs::write(jig.join("open/B-00a002.md"), "not an issue file").unwrap();
    fs::write(jig.join("open/B-00a003.md"), fm("B-00b003", "open", "")).unwrap();
    fs::write(jig.join("open/B-00a004.md"), fm("B-00a004", "closed", "")).unwrap();
    fs::write(jig.join("open/B-00a005.md"), fm("B-00a005", "open", "")).unwrap();
    fs::write(jig.join("closed/B-00a005.md"), fm("B-00a005", "closed", "")).unwrap();
    let conflicted = fm("B-00a006", "open", "").replace(
        "## Notes\n",
        "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> z\n\n## Notes\n",
    );
    fs::write(jig.join("open/B-00a006.md"), conflicted).unwrap();
    let refs = fm("B-f00d00.1", "open", "")
        .replace("parent: null", "parent: \"B-f00d00\"")
        .replace("depends_on: []", "depends_on: [\"B-dead00\"]");
    fs::write(jig.join("open/B-f00d00.1.md"), refs).unwrap();
    for (id, dep) in [
        ("B-c00001", "B-c00002"),
        ("B-c00002", "B-c00003"),
        ("B-c00003", "B-c00001"),
    ] {
        let text =
            fm(id, "open", "").replace("depends_on: []", &format!("depends_on: [\"{dep}\"]"));
        let text = if id == "B-c00002" {
            text.replace("assignee: null", "assignee: \"x\"")
        } else {
            text
        };
        fs::write(jig.join(format!("open/{id}.md")), text).unwrap();
    }
    let fields = fm("B-f1ed01", "open", "")
        .replace("assignee: null", "assignee: \"x\"")
        .replace("claim_id: null", "claim_id: \"c\"")
        .replace("closed_at: null", "closed_at: \"2026-01-01\"")
        .replace("depends_on: []", "depends_on: [\"B-f00d0a\"]")
        .replace("## History\n", "## Closing Summary\n\n> s\n\n## History\n");
    fs::write(jig.join("open/B-f1ed01.md"), fields).unwrap();
    fs::write(jig.join("open/notes.txt"), "hello").unwrap();
    fs::write(jig.join("open/.B-0000aa.md.tmp-42-1"), "x").unwrap();
    fs::create_dir(jig.join("open/B-d1r001.md")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("B-00a004.md", jig.join("closed/B-5ymb01.md")).unwrap();
    fs::write(
        jig.join(".move"),
        journal(&fm("B-0ff001", "open", ""), &fm("B-0ff001", "closed", "")),
    )
    .unwrap();

    let before = tree_bytes(&jig);
    let o = run(&d, &["check", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let stdout = String::from_utf8(o.stdout).unwrap();
    let problems: Vec<Value> = serde_json::from_str(&stdout).unwrap();
    assert!(problems.len() >= 16, "{stdout}");
    let abs = d.to_string_lossy().to_string();
    assert!(
        !stdout.contains(&abs),
        "absolute path leaked into diagnostics: {stdout}"
    );
    for p in &problems {
        let path = p["path"].as_str().unwrap();
        let problem = p["problem"].as_str().unwrap();
        let hint = p["hint"].as_str().unwrap();
        assert!(
            path.starts_with(".jig/") || path == "AGENTS.md",
            "bad path {path:?} in {stdout}"
        );
        assert!(!PathBuf::from(path).is_absolute(), "absolute {path:?}");
        assert!(
            !problem.contains(&abs),
            "absolute path in problem: {problem}"
        );
        assert!(
            !problem.contains(path),
            "problem repeats its own path: {problem}"
        );
        assert!(!hint.contains(&abs), "absolute path in hint: {hint}");
        assert!(!problem.is_empty());
        assert!(!hint.is_empty(), "empty hint for {path}");
    }
    let has = |path: &str, needle: &str| {
        problems.iter().any(|p| {
            p["path"].as_str() == Some(path)
                && p["problem"].as_str().unwrap().contains(needle)
                && !p["hint"].as_str().unwrap().is_empty()
        })
    };
    assert!(has(".jig/open/B-00a001.md", "read"), "{stdout}");
    assert!(has(".jig/open/B-00a002.md", "frontmatter"), "{stdout}");
    assert!(has(".jig/open/B-00a003.md", "file name"), "{stdout}");
    assert!(has(".jig/open/B-00a004.md", "closed"), "{stdout}");
    assert!(has(".jig/open/B-00a005.md", "duplicate"), "{stdout}");
    assert!(has(".jig/closed/B-00a005.md", "duplicate"), "{stdout}");
    assert!(has(".jig/open/B-00a006.md", "conflict"), "{stdout}");
    assert!(
        has(".jig/open/B-f00d00.1.md", "parent B-f00d00"),
        "{stdout}"
    );
    assert!(
        has(".jig/open/B-f00d00.1.md", "blocker B-dead00"),
        "{stdout}"
    );
    assert!(has(".jig/open/B-c00001.md", "cycle"), "{stdout}");
    assert!(has(".jig/open/B-c00002.md", "assignee"), "{stdout}");
    assert!(has(".jig/open/B-f1ed01.md", "blocker B-f00d0a"), "{stdout}");
    assert!(
        problems
            .iter()
            .filter(|p| p["path"].as_str() == Some(".jig/open/B-f1ed01.md"))
            .count()
            >= 4,
        "field inconsistencies: {stdout}"
    );
    assert!(has(".jig/open/notes.txt", "unexpected"), "{stdout}");
    assert!(has(".jig/open/B-d1r001.md", "regular file"), "{stdout}");
    #[cfg(unix)]
    assert!(has(".jig/closed/B-5ymb01.md", "symbolic link"), "{stdout}");
    assert!(
        has(".jig/open/.B-0000aa.md.tmp-42-1", "temporary"),
        "{stdout}"
    );
    assert!(has(".jig/memory.md", "memory.md"), "{stdout}");
    assert!(has(".jig/.gitignore", "legacy"), "{stdout}");
    assert!(has(".jig/.move", ""), "{stdout}");
    assert!(has("AGENTS.md", "missing"), "{stdout}");
    assert_eq!(tree_bytes(&jig), before, "check must not change .jig");
    assert!(!jig.join(".lock").exists(), "check must not create .lock");
}

#[test]
fn tmp_ignore_rule_covers_nested_files() {
    let d = dir("tmpignore");
    let jig = seed_bare_registry(&d);
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&d)
        .output()
        .unwrap();
    assert!(init.status.success(), "git init must succeed for this test");
    fs::write(jig.join("open/.B-0000aa.md.tmp-42-1"), "x").unwrap();
    let tmp = ".jig/open/.B-0000aa.md.tmp-42-1";
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
    fs::write(jig.join(".gitignore"), "/.lock\n/.move\n/.*.tmp-*\n").unwrap();
    assert!(
        !ignored(),
        "legacy anchored `/.*.tmp-*` must not ignore nested tmp files"
    );
}

#[test]
fn check_pending_move_then_read_recovers() {
    let d = dir("checkmove");
    ok(&d, &["init", "--no-hooks"]);
    let jig = d.join(".jig");
    fs::remove_file(jig.join(".lock")).unwrap();
    let old = fm("B-0ff002", "open", "");
    let new = fm("B-0ff002", "closed", "");
    fs::write(jig.join("open/B-0ff002.md"), &old).unwrap();
    fs::write(jig.join(".move"), journal(&old, &new)).unwrap();

    let o = run(&d, &["check", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let stdout = String::from_utf8(o.stdout).unwrap();
    assert!(
        stdout.contains("pending move journal for B-0ff002"),
        "{stdout}"
    );
    assert!(jig.join(".move").exists(), "check must not recover");
    assert!(!jig.join(".lock").exists());

    fs::write(jig.join(".lock"), "").unwrap();
    let out = ok(&d, &["show", "B-0ff002"]);
    assert!(out.contains("closed"), "{out}");
    assert!(!jig.join(".move").exists(), "read must finish the move");
    assert!(jig.join("closed/B-0ff002.md").is_file());
    assert!(!jig.join("open/B-0ff002.md").exists());
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn ready_warns_once_per_broken_reference() {
    let d = dir("readywarn");
    ok(&d, &["init", "--no-hooks"]);
    let good = add(&d, "good work");
    fs::write(
        d.join(".jig/open/B-de0001.md"),
        fm("B-de0001", "open", "").replace("depends_on: []", "depends_on: [\"B-dead00\"]"),
    )
    .unwrap();
    fs::write(
        d.join(".jig/open/B-bad0f0.md"),
        fm("B-bad0f0", "open", "").replace("claim_id: null", "claim_id: \"c\""),
    )
    .unwrap();
    fs::write(
        d.join(".jig/open/B-de0002.md"),
        fm("B-de0002", "open", "").replace("depends_on: []", "depends_on: [\"B-bad0f0\"]"),
    )
    .unwrap();
    fs::write(
        d.join(".jig/open/B-ca0000.1.md"),
        fm("B-ca0000.1", "open", "").replace("parent: null", "parent: \"B-ca0000\""),
    )
    .unwrap();

    let o = run(&d, &["ready", "--json"]);
    assert!(o.status.success());
    let rows: Vec<Value> = serde_json::from_str(&String::from_utf8(o.stdout).unwrap()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"].as_str().unwrap(), good);
    let warn = String::from_utf8_lossy(&o.stderr);
    let refs: Vec<_> = warn.lines().filter(|l| l.contains("unresolved")).collect();
    assert_eq!(refs.len(), 2, "{warn}");
    assert!(
        warn.lines().filter(|l| l.contains("B-bad0f0.md")).count() == 1,
        "one file warning for the inconsistent record: {warn}"
    );
    assert!(
        warn.contains("B-de0001") && warn.contains("B-ca0000.1"),
        "{warn}"
    );
    let e = bad(&d, &["claim", "B-de0002"]);
    assert!(
        e.contains("blocked") && e.contains("B-bad0f0"),
        "inconsistent blocker is an ordinary blocker: {e}"
    );
    let out = ok(&d, &["claim", "B-de0002", "--force"]);
    assert!(out.contains("B-de0002"), "{out}");
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
    fs::remove_file(d.join(".jig/.lock")).unwrap();
    let xdg = d.join("xdg");
    fs::create_dir_all(&xdg).unwrap();
    fs::set_permissions(&xdg, fs::Permissions::from_mode(0o777)).unwrap();

    let locked: Vec<PathBuf> =
        [d.join(".jig"), d.join(".jig/open"), d.join(".jig/closed")].to_vec();
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
        !d.join(".jig/.lock").exists(),
        "read commands must not create .lock"
    );

    let probe = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(d.join(".jig/.probe"));
    let as_root = probe.is_ok();
    if as_root {
        fs::remove_file(d.join(".jig/.probe")).unwrap();
    }
    let jig_cmd = |args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_jig"));
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
    let o = jig_cmd(&["add", "must fail"]);
    assert!(
        !o.status.success(),
        "write succeeded in a read-only registry"
    );
    assert!(!o.stderr.is_empty());
    assert_eq!(fs::read_dir(d.join(".jig/open")).unwrap().count(), 1);
    drop(restore);

    let old = fm("B-0ff003", "open", "");
    let new = fm("B-0ff003", "closed", "");
    fs::write(d.join(".jig/open/B-0ff003.md"), &old).unwrap();
    fs::write(d.join(".jig/.move"), journal(&old, &new)).unwrap();
    fs::File::create(d.join(".jig/.lock")).unwrap();
    let _relock = Restore(locked.clone());
    for p in &locked {
        fs::set_permissions(p, fs::Permissions::from_mode(0o555)).unwrap();
    }
    let o = jig_cmd(&["show", "B-0ff003"]);
    assert!(
        !o.status.success(),
        "read must not silently recover in a read-only registry"
    );
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("jig check") || e.contains(".move"), "{e}");
    assert!(d.join(".jig/.move").exists(), "journal must remain");
    assert!(d.join(".jig/open/B-0ff003.md").exists());
}

#[test]
fn init_repairs_missing_components() {
    let d = dir("initfix");
    ok(&d, &["init", "--no-hooks"]);
    fs::remove_dir_all(d.join(".jig/closed")).unwrap();
    fs::remove_file(d.join(".jig/memory.md")).unwrap();
    fs::write(d.join(".jig/.gitignore"), "custom-only\n").unwrap();
    assert_eq!(ok(&d, &["init", "--no-hooks"]), "initialized .jig/\n");
    assert!(d.join(".jig/closed").is_dir());
    assert_eq!(
        fs::read_to_string(d.join(".jig/memory.md")).unwrap(),
        "# Project Memory\n",
        "repair writes fresh memory.md"
    );
    assert_eq!(
        fs::read_to_string(d.join(".jig/.gitignore")).unwrap(),
        "custom-only\n",
        "existing .gitignore bytes preserved"
    );
    fs::write(
        d.join(".jig/memory.md"),
        "# Project Memory\n\n## kept\n> v\n",
    )
    .unwrap();
    assert_eq!(
        ok(&d, &["init", "--no-hooks"]),
        ".jig/ already initialized\n"
    );
    assert_eq!(
        fs::read_to_string(d.join(".jig/memory.md")).unwrap(),
        "# Project Memory\n\n## kept\n> v\n"
    );
    let _id = add(&d, "post repair");

    #[cfg(unix)]
    {
        fs::remove_dir_all(d.join(".jig/open")).unwrap();
        std::os::unix::fs::symlink("closed", d.join(".jig/open")).unwrap();
        let e = bad(&d, &["init", "--no-hooks"]);
        assert!(e.contains("open"), "{e}");
        fs::remove_file(d.join(".jig/open")).unwrap();
        fs::create_dir(d.join(".jig/open")).unwrap();
    }

    let outer = dir("initouter");
    ok(&outer, &["init", "--no-hooks"]);
    let inner = outer.join("inner");
    fs::create_dir_all(inner.join(".jig/open")).unwrap();
    let e = bad(&inner, &["list"]);
    assert!(
        e.contains("jig init"),
        "partial registry must point at init, not the ancestor: {e}"
    );
    assert_eq!(ok(&inner, &["init", "--no-hooks"]), "initialized .jig/\n");
    assert!(inner.join(".jig/closed").is_dir());
    assert!(inner.join(".jig/memory.md").is_file());
    assert_eq!(ok(&inner, &["list", "--json"]).trim(), "[]");
}

fn assert_splice(orig: &str, result: &str) {
    let ob = orig.as_bytes();
    let rb = result.as_bytes();
    let pre = (0..ob.len().min(rb.len()))
        .find(|&i| ob[i] != rb[i])
        .unwrap_or(ob.len());
    let mut suf = 0usize;
    while suf < (ob.len() - pre).min(rb.len() - pre)
        && ob[ob.len() - 1 - suf] == rb[rb.len() - 1 - suf]
    {
        suf += 1;
    }
    assert_eq!(&result[..pre], &orig[..pre], "prefix changed");
    assert_eq!(
        &result[result.len() - suf..],
        &orig[orig.len() - suf..],
        "suffix changed"
    );
    let inserted = &result[pre..result.len() - suf];
    let v: Value = serde_json::from_str(result).unwrap();
    let groups = v["hooks"]["SessionStart"].as_array().unwrap();
    let standard =
        json!({"hooks":[{"command":"jig start --hook-json","timeout":5,"type":"command"}]});
    assert_eq!(
        groups.last().unwrap(),
        &standard,
        "inserted fragment {inserted:?} lacks the hook group"
    );
}

#[test]
fn init_config_splice_matrix() {
    let cases: &[&str] = &[
        "",
        "   \n\n",
        "{}",
        "{\"model\": \"opus\"}",
        "{\"hooks\":{}}",
        "{\n  \"hooks\": {\n    \"SessionStart\": []\n  }\n}\n",
        "{\"hooks\":{\"SessionStart\":[{\"matcher\":\"x\",\"hooks\":[{\"type\":\"command\",\"command\":\"old\"}]}],\"Stop\":[{\"hooks\":[]}]},\"other\":{\"esc\":\"a\\\"b\",\"deep\":[[1,2],{\"k\":null}]}}",
        "{\r\n  \"hooks\": {\r\n    \"SessionStart\": []\r\n  }\r\n}\r\n",
    ];
    for (n, orig) in cases.iter().enumerate() {
        let d = dir(&format!("splice{n}"));
        let claude = d.join(".claude/settings.json");
        fs::create_dir_all(d.join(".claude")).unwrap();
        fs::write(&claude, orig).unwrap();
        ok(&d, &["init"]);
        let result = fs::read_to_string(&claude).unwrap();
        assert_splice(orig, &result);
        ok(&d, &["init"]);
        assert_eq!(
            fs::read_to_string(&claude).unwrap(),
            result,
            "case {n} not idempotent"
        );
    }

    let refusals: &[(&str, &str)] = &[
        ("{\"hooks\":{},\"hooks\":{}}", "duplicate key \"hooks\""),
        (
            "{\"hooks\":{\"SessionStart\":[],\"SessionStart\":[]}}",
            "duplicate key \"SessionStart\"",
        ),
        (
            "{\"hooks\":{\"\\u0053essionStart\":[],\"SessionStart\":[]}}",
            "duplicate key \"SessionStart\"",
        ),
        (
            "{\"hooks\":{\"SessionStart\":\"nope\"}}",
            "SessionStart to be an array",
        ),
        (
            "{\"hooks\":{\"SessionStart\":[{\"matcher\":\"x\",\"hooks\":[{\"type\":\"command\",\"command\":\"jig start --hook-json\",\"timeout\":9}]}]}}",
            "differs or is duplicated",
        ),
        ("{bad json", "invalid JSON"),
    ];
    for (n, (orig, needle)) in refusals.iter().enumerate() {
        let d = dir(&format!("refuse{n}"));
        let claude = d.join(".claude/settings.json");
        fs::create_dir_all(d.join(".claude")).unwrap();
        fs::write(&claude, orig).unwrap();
        let e = bad(&d, &["init"]);
        assert!(e.contains(needle), "case {n}: {e}");
        assert_eq!(fs::read_to_string(&claude).unwrap(), *orig);
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() >> 33) as usize % xs.len()]
    }
}

#[test]
fn seeded_body_variations_roundtrip_or_refuse() {
    let d = dir("seededbody");
    ok(&d, &["init", "--no-hooks"]);
    let mut rng = Rng(0x9e3779b97f4a7c15);
    const SAFE: &[&str] = &[
        "plain text line",
        "zażółć gęślą jaźń — 🎲 ünïcode",
        "\"quoted\" 'apostrophe' `tick`",
        "",
        "---",
        "### deeper heading",
        "# one hash",
        "  indented  ",
        "<<<<<<< HEAD\nno equals\n>>>>>>> other",
        "=======",
        ">>>>>>> branch",
        "|\ttabs",
        "text with ## midline",
        "* bullet",
    ];
    const POISON: &[&str] = &[
        "## History",
        "## Notes",
        "## Closing Summary",
        "   ## History",
        "## Notes ###",
        "<<<<<<< A\nx\n=======\ny\n>>>>>>> B",
        "```\nunclosed",
    ];
    const FENCED: &[&str] = &[
        "## History",
        "## Notes x",
        "## Closing Summary",
        "<<<<<<< A\nx\n=======\ny\n>>>>>>> B",
        "   ## Notes ###",
    ];
    for case in 0..24 {
        let id = add(&d, &format!("s{case}"));
        let file = d.join(format!(".jig/open/{id}.md"));
        let orig = fs::read_to_string(&file).unwrap();
        let mut body = String::from("seed intro\n\n");
        let mut accept = true;
        for _ in 0..1 + (rng.next() % 4) {
            match rng.next() % 8 {
                0..=4 => {
                    body.push_str(rng.pick::<&str>(SAFE));
                    body.push('\n');
                }
                5 => {
                    body.push_str("```\n");
                    body.push_str(rng.pick::<&str>(FENCED));
                    body.push_str("\n```\n");
                }
                6 => {
                    body.push_str("~~~~\n");
                    body.push_str(rng.pick::<&str>(FENCED));
                    body.push_str("\n~~~~\n");
                }
                _ => {
                    accept = false;
                    body.push_str(rng.pick::<&str>(POISON));
                    body.push('\n');
                }
            }
        }
        if case % 3 == 0 && accept {
            accept = false;
            body.push_str(rng.pick::<&str>(POISON));
            body.push('\n');
        }
        let before = fs::read_to_string(&file).unwrap();
        assert_eq!(before, orig);
        if accept {
            ok(&d, &["update", &id, "--body", &body]);
            let i = jig::issue::decode(&fs::read_to_string(&file).unwrap()).unwrap();
            assert_eq!(i.body, body, "case {case} body must round-trip exactly");
        } else {
            let e = bad(&d, &["update", &id, "--body", &body]);
            assert!(!e.is_empty());
            assert_eq!(
                fs::read_to_string(&file).unwrap(),
                before,
                "case {case} refused body must not write"
            );
        }
    }
}

#[test]
fn memory_remember_roundtrips_difficult_values() {
    let d = dir("memvals");
    ok(&d, &["init", "--no-hooks"]);
    let values = [
        "Unicode zażółć 🎲 é",
        "line1\r\nline2\nline3",
        "```\nfenced ## History\n```",
        "## Notes\nlooks like a heading",
        "<<<<<<< HEAD\na\n=======\nb\n>>>>>>> x",
        "",
        "trailing newline\n",
    ];
    for (n, value) in values.iter().enumerate() {
        let topic = format!("t{n}");
        ok(&d, &["remember", &topic, value]);
        let out = ok(&d, &["recall", &topic, "--json"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["value"].as_str().unwrap(),
            *value,
            "memory value must round-trip exactly"
        );
    }
    ok(&d, &["remember", "flagged", "--", "-x"]);
    let out = ok(&d, &["recall", "flagged", "--json"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["value"].as_str().unwrap(), "-x");
}

#[test]
fn reopen_multiline_reason_and_defer_today() {
    let d = dir("reopenml");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "ml");
    ok(&d, &["close", &id, "--reason", "closed\nfor cleanup"]);
    ok(&d, &["reopen", &id, "--reason", "first line\nsecond line"]);
    let i = jig::issue::decode(&fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap())
        .unwrap();
    assert!(
        i.history
            .ends_with("reopened:\n  > first line\n  > second line"),
        "{:?}",
        i.history
    );

    let today = jig::clock::today();
    let id2 = add(&d, "defer me");
    ok(&d, &["defer", &id2, "--until", &today]);
    let out = ok(&d, &["ready"]);
    assert!(
        out.contains(&id2),
        "defer-until-today must not defer: {out}"
    );
}

const G_STRUCT: u8 = 1;
const G_NL: u8 = 2;
const G_CTRL: u8 = 4;
const G_SEP: u8 = 8;
const G_BLANK: u8 = 16;
const G_EMPTY: u8 = 32;
const G_LONG: u8 = 64;

const GEN_TOKENS: &[&str] = &[
    "plain token",
    "zażółć 🎲 ünïcode — dash",
    "\"quoted\" 'apos' `tick`",
];

fn generated_text(rng: &mut Rng, case: usize) -> String {
    let token = rng.pick(GEN_TOKENS);
    match case % 19 {
        0 => format!("word {token} {case}"),
        1 => String::new(),
        2 => "   ".to_string(),
        3 => format!("zażółć 🎲 {token} \"quoted\" 'apos' `tick`"),
        4 => "---".to_string(),
        5 => format!("{token}\nsecond line {case}\ntrailing\n"),
        6 => format!("```\n## History\n<<<<<<< A\n{token}\n=======\ny\n>>>>>>> B\n```\nok {case}"),
        7 => "   ## History ###".to_string(),
        8 => "## Notes".to_string(),
        9 => format!("<<<<<<< A\n{token}\n=======\ny\n>>>>>>> B"),
        10 => format!("```\nunclosed {token}"),
        11 => format!("a\r\n{token}\r\n"),
        12 => format!("tab\t{token}"),
        13 => format!("esc\u{1b}{token}"),
        14 => format!("nel\u{85}{token}"),
        15 => format!("sep\u{2028}{token}\u{2029}y"),
        16 => format!("```rust\n{token}\n```"),
        17 => format!("~~~~\n{token}\n~~~~"),
        _ => format!("{token}{}", "x".repeat(600)),
    }
}

fn generated_tag(case: usize) -> u8 {
    match case % 19 {
        1 => G_BLANK | G_EMPTY,
        2 => G_BLANK,
        5 | 6 | 16 | 17 => G_NL,
        7 | 8 => G_STRUCT,
        9 | 10 => G_STRUCT | G_NL,
        11 => G_CTRL | G_NL,
        12..=14 => G_CTRL,
        15 => G_SEP,
        18 => G_LONG,
        _ => 0,
    }
}

fn ok_title(t: u8) -> bool {
    t & (G_NL | G_CTRL | G_SEP | G_BLANK) == 0
}
fn ok_body(t: u8) -> bool {
    t & G_STRUCT == 0
}
fn ok_reason(t: u8) -> bool {
    t & (G_STRUCT | G_CTRL | G_BLANK) == 0
}
fn ok_summary(t: u8) -> bool {
    t & (G_STRUCT | G_CTRL) == 0
}
fn ok_note(t: u8) -> bool {
    t & (G_NL | G_CTRL | G_SEP | G_EMPTY | G_LONG) == 0
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

fn generated_case(d: &Path, args: &[&str], accept: bool, case: usize) -> Output {
    let before = tree_bytes(&d.join(".jig"));
    let o = run(d, args);
    if accept {
        assert!(
            o.status.success(),
            "case {case} {args:?} must succeed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    } else {
        assert!(!o.status.success(), "case {case} {args:?} must fail");
        assert_eq!(
            tree_bytes(&d.join(".jig")),
            before,
            "case {case} refused write changed .jig"
        );
    }
    o
}

#[test]
fn generated_add_title_cases() {
    let d = dir("genadd");
    ok(&d, &["init", "--no-hooks"]);
    assert!(!d.join("xdg").exists(), "init must not create state home");
    let e = bad(&d, &["add", "--approved", "--", "   "]);
    assert!(e.contains("single line"), "{e}");
    assert!(
        !d.join("xdg").exists(),
        "rejected title must not install salt"
    );
    let jig = d.join(".jig");
    let mut rng = Rng(0x8f3a1);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_title(generated_tag(case));
        let o = generated_case(
            &d,
            &["add", "--approved", "--", text.as_str()],
            accept,
            case,
        );
        if accept {
            ok_n += 1;
            let id = String::from_utf8(o.stdout).unwrap().trim().to_string();
            let i = show_value(&d, &id);
            assert_eq!(i["title"].as_str().unwrap(), text, "case {case} title");
            fs::remove_file(jig.join("open").join(format!("{id}.md"))).unwrap();
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "add title ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert!(
        d.join("xdg/jig/install-id").is_file(),
        "install salt must live under XDG_STATE_HOME/jig"
    );
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_update_title_cases() {
    let d = dir("genutitle");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    let mut rng = Rng(0x8f3a2);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_title(generated_tag(case));
        generated_case(&d, &["update", &id, "--title", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            assert_eq!(i["title"].as_str().unwrap(), text, "case {case} title");
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "update title ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_update_body_cases() {
    let d = dir("genubody");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    let mut rng = Rng(0x8f3a3);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_body(generated_tag(case));
        generated_case(&d, &["update", &id, "--body", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            assert_eq!(i["body"].as_str().unwrap(), text, "case {case} body");
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "update body ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_review_summary_cases() {
    let d = dir("genreviews");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    ok(&d, &["claim", &id, "--by", "gen"]);
    let mut rng = Rng(0x8f3a4);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_summary(generated_tag(case));
        generated_case(&d, &["review", &id, "--summary", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let history = i["history"].as_str().unwrap();
            assert_eq!(
                hist_last_value(history, "summary:").as_deref(),
                Some(text.as_str()),
                "case {case} review summary in {history:?}"
            );
            ok(&d, &["rework", &id, "--reason", "cycle"]);
        } else {
            bad_n += 1;
        }
    }
    assert!(
        ok_n > 0 && bad_n > 0,
        "review summary ok={ok_n} bad={bad_n}"
    );
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_rework_reason_cases() {
    let d = dir("genrework");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    ok(&d, &["claim", &id, "--by", "gen"]);
    ok(&d, &["review", &id]);
    let mut rng = Rng(0x8f3a5);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_reason(generated_tag(case));
        generated_case(&d, &["rework", &id, "--reason", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let history = i["history"].as_str().unwrap();
            assert_eq!(
                hist_last_value(history, "rework requested:").as_deref(),
                Some(text.as_str()),
                "case {case} rework reason in {history:?}"
            );
            ok(&d, &["review", &id]);
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "rework reason ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_revise_reason_cases() {
    let d = dir("genrevise");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    let mut rng = Rng(0x8f3a6);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_reason(generated_tag(case));
        generated_case(&d, &["revise", &id, "--reason", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let history = i["history"].as_str().unwrap();
            assert_eq!(
                hist_last_value(history, "revised:").as_deref(),
                Some(text.as_str()),
                "case {case} revise reason in {history:?}"
            );
            ok(&d, &["approve", &id]);
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "revise reason ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_close_reason_cases() {
    let d = dir("genclose");
    ok(&d, &["init", "--no-hooks"]);
    let jig = d.join(".jig");
    let mut rng = Rng(0x8f3a7);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_reason(generated_tag(case));
        let id = add(&d, "g");
        generated_case(&d, &["close", &id, "--reason", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let history = i["history"].as_str().unwrap();
            assert_eq!(
                hist_last_value(history, "closed:").as_deref(),
                Some(text.as_str()),
                "case {case} close reason in {history:?}"
            );
            fs::remove_file(jig.join("closed").join(format!("{id}.md"))).unwrap();
        } else {
            bad_n += 1;
            fs::remove_file(jig.join("open").join(format!("{id}.md"))).unwrap();
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "close reason ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_close_summary_cases() {
    let d = dir("genclosesum");
    ok(&d, &["init", "--no-hooks"]);
    let jig = d.join(".jig");
    let mut rng = Rng(0x8f3a8);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_summary(generated_tag(case));
        let id = add(&d, "g");
        generated_case(
            &d,
            &["close", &id, "--reason", "r", "--summary", &text],
            accept,
            case,
        );
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            assert_eq!(
                i["closing_summary"].as_str().unwrap(),
                text,
                "case {case} closing_summary"
            );
            fs::remove_file(jig.join("closed").join(format!("{id}.md"))).unwrap();
        } else {
            bad_n += 1;
            fs::remove_file(jig.join("open").join(format!("{id}.md"))).unwrap();
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "close summary ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_reopen_reason_cases() {
    let d = dir("genreopen");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    ok(&d, &["close", &id, "--reason", "seed"]);
    let mut rng = Rng(0x8f3a9);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_reason(generated_tag(case));
        generated_case(&d, &["reopen", &id, "--reason", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let history = i["history"].as_str().unwrap();
            assert_eq!(
                hist_last_value(history, "reopened:").as_deref(),
                Some(text.as_str()),
                "case {case} reopen reason in {history:?}"
            );
            ok(&d, &["close", &id, "--reason", "reclose"]);
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "reopen reason ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_note_cases() {
    let d = dir("gennote");
    ok(&d, &["init", "--no-hooks"]);
    let id = add(&d, "seed");
    let mut rng = Rng(0x8f3aa);
    let (mut ok_n, mut bad_n) = (0usize, 0usize);
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        let accept = ok_note(generated_tag(case));
        generated_case(&d, &["note", &id, "--", &text], accept, case);
        if accept {
            ok_n += 1;
            let i = show_value(&d, &id);
            let last = i["notes"].as_str().unwrap().lines().last().unwrap();
            assert_eq!(
                last.splitn(5, ' ').nth(4).unwrap_or_default(),
                text,
                "case {case} note line {last:?}"
            );
        } else {
            bad_n += 1;
        }
    }
    assert!(ok_n > 0 && bad_n > 0, "note ok={ok_n} bad={bad_n}");
    assert!(ok_n + bad_n >= 100);
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn generated_remember_cases() {
    let d = dir("genmemory");
    ok(&d, &["init", "--no-hooks"]);
    let mut rng = Rng(0x8f3ab);
    let mut ok_n = 0usize;
    for case in 0..110 {
        let text = generated_text(&mut rng, case);
        generated_case(&d, &["remember", "gen.topic", "--", &text], true, case);
        ok_n += 1;
        let out = ok(&d, &["recall", "gen.topic", "--json"]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["value"].as_str().unwrap(), text, "case {case} memory");
    }
    assert!(ok_n >= 100, "memory accepts every value, got {ok_n}");
    assert_eq!(ok(&d, &["check", "--json"]).trim(), "[]");
}

#[test]
fn init_config_splice_styles_match_surroundings() {
    let two = concat!(
        "{\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"SessionStart\": [\n      {\n",
        "        \"hooks\": [\n          {\n            \"type\": \"command\",\n",
        "            \"command\": \"jig start --hook-json\",\n            \"timeout\": 5\n",
        "          }\n        ]\n      }\n    ]\n  }\n}\n"
    );
    let four = concat!(
        "{\n    \"hooks\": {\n        \"PostToolUse\": [{\"hooks\": []}],\n",
        "        \"SessionStart\": [\n            {\n                \"hooks\": [\n",
        "                    {\n                        \"type\": \"command\",\n",
        "                        \"command\": \"jig start --hook-json\",\n",
        "                        \"timeout\": 5\n                    }\n                ]\n",
        "            }\n        ]\n    }\n}"
    );
    let tab = concat!(
        "{\n\t\"hooks\": {\n\t\t\"SessionStart\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"old\"}]},\n",
        "\t\t\t{\n\t\t\t\t\"hooks\": [\n\t\t\t\t\t{\n",
        "\t\t\t\t\t\t\"type\": \"command\",\n",
        "\t\t\t\t\t\t\"command\": \"jig start --hook-json\",\n",
        "\t\t\t\t\t\t\"timeout\": 5\n\t\t\t\t\t}\n\t\t\t\t]\n",
        "\t\t\t}\n\t\t]\n\t}\n}"
    );
    let crlf = concat!(
        "{\r\n  \"hooks\": {\r\n    \"SessionStart\": [\r\n      {\r\n",
        "        \"hooks\": [\r\n          {\r\n            \"type\": \"command\",\r\n",
        "            \"command\": \"jig start --hook-json\",\r\n            \"timeout\": 5\r\n",
        "          }\r\n        ]\r\n      }\r\n    ]\r\n  }\r\n}\r\n"
    );
    let mixed = concat!(
        "{\n    \"model\": 1, \"hooks\": {\n        \"SessionStart\": [\n            {\n",
        "                \"hooks\": [\n                    {\n",
        "                        \"type\": \"command\",\n",
        "                        \"command\": \"jig start --hook-json\",\n",
        "                        \"timeout\": 5\n                    }\n                ]\n",
        "            }\n        ]\n    }\n}"
    );
    let cases: &[(&str, &str, &str)] = &[
        (
            "mixed_inline_sibling",
            "{\n    \"model\": 1, \"hooks\": {}\n}",
            mixed,
        ),
        ("two_space_infer_root", "{\n  \"model\": \"opus\"\n}\n", two),
        (
            "four_space_missing_sessionstart",
            "{\n    \"hooks\": {\n        \"PostToolUse\": [{\"hooks\": []}]\n    }\n}",
            four,
        ),
        (
            "tab_populated_array",
            "{\n\t\"hooks\": {\n\t\t\"SessionStart\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"old\"}]}]\n\t}\n}",
            tab,
        ),
        (
            "crlf_empty_array",
            "{\r\n  \"hooks\": {\r\n    \"SessionStart\": []\r\n  }\r\n}\r\n",
            crlf,
        ),
        (
            "compact_empty",
            "{}",
            "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"jig start --hook-json\",\"timeout\":5}]}]}}",
        ),
        (
            "compact_final_lf",
            "{}\n",
            "{\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"jig start --hook-json\",\"timeout\":5}]}]}}\n",
        ),
        (
            "compact_member",
            "{\"model\": \"opus\"}",
            "{\"model\": \"opus\",\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"jig start --hook-json\",\"timeout\":5}]}]}}",
        ),
    ];
    for (name, orig, expected) in cases {
        let d = dir(&format!("style{name}"));
        for cfg in [".claude/settings.json", ".codex/hooks.json"] {
            let path = d.join(cfg);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, orig).unwrap();
        }
        ok(&d, &["init"]);
        for cfg in [".claude/settings.json", ".codex/hooks.json"] {
            let result = fs::read_to_string(d.join(cfg)).unwrap();
            assert_eq!(&result, expected, "{name} {cfg}");
            let v: Value = serde_json::from_str(&result).unwrap();
            assert_eq!(
                v["hooks"]["SessionStart"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["hooks"][0],
                json!({"type":"command","command":"jig start --hook-json","timeout":5}),
                "{name} {cfg} group"
            );
        }
        ok(&d, &["init"]);
        for cfg in [".claude/settings.json", ".codex/hooks.json"] {
            assert_eq!(
                fs::read_to_string(d.join(cfg)).unwrap(),
                *expected,
                "{name} {cfg} idempotent"
            );
        }
    }
}

#[test]
fn list_tree_tolerate_bad_files_and_reattach_orphans() {
    let d = dir("treebad");
    ok(&d, &["init", "--no-hooks"]);
    let p = add(&d, "root");
    let mid = ok(&d, &["add", "mid", "--parent", &p, "--approved"])
        .trim()
        .to_string();
    let leaf = ok(&d, &["add", "leaf", "--parent", &mid, "--approved"])
        .trim()
        .to_string();
    let other = add(&d, "other");

    let mid_file = d.join(format!(".jig/open/{mid}.md"));
    fs::write(&mid_file, "not an issue").unwrap();
    fs::write(d.join(".jig/open/B-dead00.md"), "not an issue").unwrap();

    let o = run(&d, &["tree", &p]);
    assert!(o.status.success(), "tree tolerates bad files");
    let out = String::from_utf8(o.stdout).unwrap();
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(
        warn.contains(".jig/open/B-dead00.md") && warn.contains("jig check"),
        "{warn}"
    );
    assert!(
        warn.contains(&format!(".jig/open/{mid}.md")),
        "warning must name the unusable record: {warn}"
    );
    assert!(out.contains(&format!("{p} open root")), "{out}");
    assert!(
        out.contains(&format!("  {leaf} open leaf")),
        "orphan must reattach under nearest visible ancestor: {out}"
    );
    assert!(!out.contains(&format!("{mid} ")), "{out}");

    let o = run(&d, &["tree", &p, "--json"]);
    assert!(o.status.success());
    let j: Value = serde_json::from_str(&String::from_utf8(o.stdout).unwrap()).unwrap();
    let kids = j["children"].as_array().unwrap();
    assert!(
        kids.iter().any(|k| k["id"].as_str() == Some(leaf.as_str())),
        "{j}"
    );
    assert!(
        !serde_json::to_string(&j)
            .unwrap()
            .contains(&format!("\"id\":\"{mid}\"")),
        "{j}"
    );

    let o = run(&d, &["list", "--json"]);
    assert!(o.status.success());
    let warn = String::from_utf8_lossy(&o.stderr);
    assert!(warn.contains("jig check"), "{warn}");
    let rows: Value = serde_json::from_str(&String::from_utf8(o.stdout).unwrap()).unwrap();
    let ids: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&p.as_str()) && ids.contains(&leaf.as_str()) && ids.contains(&other.as_str()),
        "{ids:?}"
    );
    assert!(
        !ids.contains(&mid.as_str()) && !ids.contains(&"B-dead00"),
        "{ids:?}"
    );

    let o = run(&d, &["ready"]);
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("jig check"));

    let e = bad(&d, &["tree", &mid]);
    assert!(e.contains(&mid), "{e}");
    let e = bad(&d, &["show", &mid]);
    assert!(e.contains(&mid), "{e}");
    let t = ok(&d, &["tree", &other]);
    assert!(t.contains(&format!("{other} open other")), "{t}");
}

#[test]
fn read_lists_skip_each_bad_file_once() {
    for kind in ["unreadable", "conflict", "duplicate", "wrong_directory"] {
        let d = dir(&format!("skip{kind}"));
        ok(&d, &["init", "--no-hooks"]);
        let root = add(&d, "root");
        let good = ok(&d, &["add", "good", "--parent", &root, "--approved"])
            .trim()
            .to_string();
        let bad = ok(&d, &["add", "bad", "--parent", &root, "--approved"])
            .trim()
            .to_string();
        let healthy = add(&d, "healthy");
        let open_bad = d.join(format!(".jig/open/{bad}.md"));
        let open_text = fs::read_to_string(&open_bad).unwrap();
        let closed_bad = d.join(format!(".jig/closed/{bad}.md"));
        let mut extra_bad: Option<String> = None;

        match kind {
            "unreadable" => fs::write(&open_bad, b"\xff\xfe garbage").unwrap(),
            "conflict" => {
                let pos = open_text.find("---\n\n").unwrap() + 5;
                fs::write(
                    &open_bad,
                    format!(
                        "{}<<<<<<< A\nx\n=======\ny\n>>>>>>> B\n\n{}",
                        &open_text[..pos],
                        &open_text[pos..]
                    ),
                )
                .unwrap();
            }
            "duplicate" => {
                fs::write(
                    &closed_bad,
                    open_text.replace("status: open", "status: closed"),
                )
                .unwrap();
            }
            "wrong_directory" => {
                let bad2 = ok(&d, &["add", "bad2", "--parent", &root, "--approved"])
                    .trim()
                    .to_string();
                let open_bad2 = d.join(format!(".jig/open/{bad2}.md"));
                let open_text2 = fs::read_to_string(&open_bad2).unwrap();
                fs::write(&closed_bad, &open_text).unwrap();
                fs::remove_file(&open_bad).unwrap();
                fs::write(
                    &open_bad2,
                    open_text2.replace("status: open", "status: closed"),
                )
                .unwrap();
                extra_bad = Some(bad2);
            }
            _ => unreachable!(),
        }

        let warned_list: Vec<String> = match kind {
            "unreadable" | "conflict" => vec![format!(".jig/open/{bad}.md")],
            "duplicate" => vec![
                format!(".jig/open/{bad}.md"),
                format!(".jig/closed/{bad}.md"),
            ],
            _ => vec![
                format!(".jig/closed/{bad}.md"),
                format!(".jig/open/{}.md", extra_bad.as_ref().unwrap()),
            ],
        };
        let warned_ready: Vec<&str> = warned_list
            .iter()
            .filter(|p| p.starts_with(".jig/open"))
            .map(String::as_str)
            .collect();
        let bad_ids: Vec<&str> = [Some(bad.as_str()), extra_bad.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        let abs = d.to_string_lossy().to_string();

        let assert_run = |args: &[&str], warned: &[String]| -> Output {
            let o = run(&d, args);
            assert!(
                o.status.success(),
                "{kind} {args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            let stderr = String::from_utf8_lossy(&o.stderr);
            assert!(!stderr.contains(&abs), "{kind} {args:?} abs path: {stderr}");
            for line in stderr.lines().filter(|l| !l.is_empty()) {
                assert!(line.contains("jig check"), "{kind} warning: {line}");
            }
            for path in warned {
                assert_eq!(
                    stderr.matches(path.as_str()).count(),
                    1,
                    "{kind} {args:?} must warn {path} exactly once: {stderr}"
                );
            }
            o
        };

        let warned_ready_owned: Vec<String> = warned_ready.iter().map(|s| s.to_string()).collect();
        for args in [
            vec!["ready"],
            vec!["list"],
            vec!["list", "--json"],
            vec!["tree", &root],
            vec!["tree", &root, "--json"],
        ] {
            let warned = if args[0] == "ready" {
                &warned_ready_owned
            } else {
                &warned_list
            };
            let o = assert_run(&args, warned);
            let stdout = String::from_utf8(o.stdout).unwrap();
            assert!(
                stdout.contains(&root),
                "{kind} {args:?} root missing: {stdout}"
            );
            if args[0] != "tree" {
                assert!(
                    stdout.contains(&healthy),
                    "{kind} {args:?} healthy issue missing: {stdout}"
                );
            }
            if args[0] != "ready" {
                assert!(
                    stdout.contains(&good),
                    "{kind} {args:?} good child missing: {stdout}"
                );
            }
            for b in &bad_ids {
                if args.contains(&"--json") {
                    assert!(
                        !stdout.contains(&format!("\"id\":\"{b}\"")),
                        "{kind} {args:?} bad id leaked: {stdout}"
                    );
                } else {
                    assert!(
                        !stdout.contains(&format!("{b} ")),
                        "{kind} {args:?} bad id leaked: {stdout}"
                    );
                }
            }
        }

        for b in &bad_ids {
            assert!(!run(&d, &["tree", b]).status.success(), "{kind} tree {b}");
            assert!(!run(&d, &["show", b]).status.success(), "{kind} show {b}");
        }

        let o = run(&d, &["check"]);
        assert_eq!(o.status.code(), Some(1));
        let text = String::from_utf8(o.stdout).unwrap();
        assert!(!text.contains(&abs), "{kind} check text abs: {text}");
        let o = run(&d, &["check", "--json"]);
        assert_eq!(o.status.code(), Some(1));
        let stdout = String::from_utf8(o.stdout).unwrap();
        assert!(!stdout.contains(&abs), "{kind} check json abs: {stdout}");
        let problems: Vec<Value> = serde_json::from_str(&stdout).unwrap();
        for path in &warned_list {
            let p = problems
                .iter()
                .find(|p| p["path"].as_str() == Some(path.as_str()))
                .unwrap_or_else(|| panic!("{kind} missing problem for {path}: {stdout}"));
            let problem = p["problem"].as_str().unwrap();
            assert!(Path::new(path).is_relative());
            assert!(
                !problem.contains(path.as_str()),
                "{kind} problem repeats its path: {problem}"
            );
            assert!(!p["hint"].as_str().unwrap().is_empty());
            let line = text
                .lines()
                .find(|l| l.starts_with(&format!("{path}: ")))
                .unwrap_or_else(|| panic!("{kind} text report lacks {path}: {text}"));
            assert!(
                !line[path.len() + 2..].contains(path.as_str()),
                "{kind} report line repeats path: {line}"
            );
        }
        if kind == "conflict" {
            assert!(
                problems.iter().any(|p| {
                    p["path"].as_str() == Some(format!(".jig/open/{bad}.md").as_str())
                        && p["problem"]
                            .as_str()
                            .unwrap()
                            .contains("merge conflict markers present")
                }),
                "{kind} conflict cause missing: {stdout}"
            );
        }
    }
}

fn fnv1a(text: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in text.replace("\r\n", "\n").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

#[test]
fn agent_instruction_block_states_preserve_outside_bytes() {
    let base = dir("agcanon");
    ok(&base, &["init", "--no-hooks"]);
    let written = fs::read_to_string(base.join("AGENTS.md")).unwrap();
    let start = written.find("<!-- BEGIN JIG v:1 hash:").unwrap();
    let stop = written.find("<!-- END JIG -->").unwrap() + "<!-- END JIG -->".len();
    let canonical = written[start..stop].to_string();
    let body =
        &canonical[canonical.find('\n').unwrap() + 1..canonical.rfind("<!-- END JIG -->").unwrap()];
    assert!(
        canonical.contains(&format!("hash:{:016x}", fnv1a(body))),
        "fresh block hash must cover the normalized body: {canonical}"
    );

    for nl in ["\n", "\r\n"] {
        let canon = canonical.replace('\n', nl);
        let pre = format!("pre-marker text{nl}{nl}");
        let post = format!("{nl}post-marker text{nl}");

        let fixture = |name: &str, agents: String| -> TestDir {
            let d = dir(name);
            ok(&d, &["init", "--no-hooks"]);
            fs::write(d.join("AGENTS.md"), agents).unwrap();
            d
        };
        let check_unchanged = |d: &Path, code: i32| -> String {
            let agents = fs::read(d.join("AGENTS.md")).unwrap();
            let tree = tree_bytes(&d.join(".jig"));
            let o = run(d, &["check"]);
            assert_eq!(o.status.code(), Some(code));
            assert_eq!(fs::read(d.join("AGENTS.md")).unwrap(), agents);
            assert_eq!(tree_bytes(&d.join(".jig")), tree);
            String::from_utf8(o.stdout).unwrap()
        };

        // missing block: check reports, init appends without touching prior bytes
        let d = fixture("ag-miss", format!("{pre}custom rules{nl}"));
        let out = check_unchanged(&d, 1);
        assert!(
            out.contains("AGENTS.md: Jig instructions are missing"),
            "{out}"
        );
        assert!(out.contains("run `jig init`"), "{out}");
        let old = fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\n"
        );
        let new = fs::read_to_string(d.join("AGENTS.md")).unwrap();
        assert_eq!(new, format!("{old}{nl}{canon}{nl}"), "append: {new}");

        // current intact: clean check, init is a no-op
        let file = format!("{pre}{canon}{post}");
        let d = fixture("ag-cur", file.clone());
        let out = check_unchanged(&d, 0);
        assert!(out.contains("clean"), "{out}");
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\n"
        );
        assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), file);

        // older intact: check reports outdated, init upgrades surgically
        let older = canon.replace("<!-- BEGIN JIG v:1 ", "<!-- BEGIN JIG v:0 ");
        let d = fixture("ag-old", format!("{pre}{older}"));
        let out = check_unchanged(&d, 1);
        assert!(out.contains("Jig instructions are outdated"), "{out}");
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\n"
        );
        assert_eq!(
            fs::read_to_string(d.join("AGENTS.md")).unwrap(),
            format!("{pre}{canon}"),
            "upgrade must preserve outside bytes and the file ending at END"
        );
        let out = check_unchanged(&d, 0);
        assert!(out.contains("clean"), "{out}");
        let before = fs::read(d.join("AGENTS.md")).unwrap();
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\n"
        );
        assert_eq!(
            fs::read(d.join("AGENTS.md")).unwrap(),
            before,
            "re-init idempotent"
        );

        // older intact with genuinely different body but its own valid hash
        let old_body = format!("## Issue tracking with Jig{nl}{nl}old rules{nl}");
        let older2 = format!(
            "<!-- BEGIN JIG v:0 hash:{:016x} -->{nl}{old_body}<!-- END JIG -->",
            fnv1a(&old_body)
        );
        let d = fixture("ag-old2", format!("{pre}{older2}{post}"));
        let out = check_unchanged(&d, 1);
        assert!(out.contains("Jig instructions are outdated"), "{out}");
        assert!(!out.contains("customized"), "{out}");
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\n"
        );
        assert_eq!(
            fs::read_to_string(d.join("AGENTS.md")).unwrap(),
            format!("{pre}{canon}{post}")
        );

        // current customized: not a problem, init reports and leaves bytes
        let custom = canon.replace(
            "<!-- END JIG -->",
            &format!("- our local rule{nl}<!-- END JIG -->"),
        );
        let file = format!("{pre}{custom}{post}");
        let d = fixture("ag-ccu", file.clone());
        let out = check_unchanged(&d, 0);
        assert!(out.contains("clean"), "{out}");
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            ".jig/ already initialized\nAGENTS.md: Jig instructions are customized; left unchanged\n"
        );
        assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), file);

        // older customized: check reports it, init leaves bytes and reports
        let custom_old = older.replace(
            "<!-- END JIG -->",
            &format!("- our local rule{nl}<!-- END JIG -->"),
        );
        let file = format!("{pre}{custom_old}{post}");
        let d = fixture("ag-ocu", file.clone());
        let report = check_unchanged(&d, 1);
        assert!(
            report.contains("customized Jig instructions are outdated"),
            "{report}"
        );
        assert!(report.contains("reapply"), "{report}");
        assert_eq!(
            ok(&d, &["init", "--no-hooks"]),
            format!(".jig/ already initialized\n{report}"),
            "init must repeat the check report for NeedsAttention"
        );
        assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), file);

        // newer version (intact and customized): same unsupported report
        for (name, seed) in [("ag-new", &canon), ("ag-newc", &custom)] {
            let newer = seed.replace("<!-- BEGIN JIG v:1 ", "<!-- BEGIN JIG v:2 ");
            let file = format!("{pre}{newer}{post}");
            let d = fixture(name, file.clone());
            let report = check_unchanged(&d, 1);
            assert!(
                report.contains("version 2") && report.contains("newer"),
                "{report}"
            );
            assert_eq!(
                ok(&d, &["init", "--no-hooks"]),
                format!(".jig/ already initialized\n{report}"),
                "{name}: init never downgrades"
            );
            assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), file);
        }

        // malformed and duplicated markers: check reports, init refuses
        for (name, file) in [
            (
                "ag-mal",
                format!("{pre}<!-- BEGIN JIG v:1 -->{nl}body{nl}<!-- END JIG -->{nl}"),
            ),
            ("ag-dup", format!("{pre}{canon}{nl}{canon}{nl}")),
            ("ag-ind", format!("{pre}  {canon}{nl}")),
        ] {
            let d = fixture(name, file.clone());
            let out = check_unchanged(&d, 1);
            assert!(
                out.contains("malformed or duplicated Jig instruction markers"),
                "{name}: {out}"
            );
            let o = run(&d, &["init", "--no-hooks"]);
            assert_eq!(o.status.code(), Some(1), "{name}");
            assert!(
                String::from_utf8_lossy(&o.stderr)
                    .contains("AGENTS.md: malformed or duplicated Jig instruction markers"),
                "{name}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            assert_eq!(fs::read_to_string(d.join("AGENTS.md")).unwrap(), file);
        }
    }
}

#[test]
fn agent_block_and_start_cover_workflow_rules_in_order() {
    let d = dir("agentrules");
    ok(&d, &["init", "--no-hooks"]);
    let agents = fs::read_to_string(d.join("AGENTS.md")).unwrap();
    let block = &agents[agents.find("<!-- BEGIN JIG").unwrap()..];
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
        "Jig workflow",
        "tracks work with `jig`",
        "Do not keep task lists",
        concat!(
            "cargo install --locked --git https://github.com/EntropyLogix/Jig --tag v",
            env!("CARGO_PKG_VERSION")
        ),
        "https://github.com/EntropyLogix/Jig",
        "jig start` for the workflow",
        "jig ready` lists approved, unblocked work",
        "jig show ID` reads one issue",
        "authorizes only its described scope",
        "jig claim ID --by NAME",
        "your agent name",
        "jig review ID --summary TEXT",
        "for people unless a person explicitly tells you otherwise",
        "jig add TITLE --body TEXT",
        "jig note ID TEXT",
        "jig unclaim ID",
        "jig recall [TOPIC]",
        "jig remember TOPIC TEXT",
        "your own memory",
        "jig check` and report to a person",
    ];
    for hay in [
        workflow,
        block.strip_prefix("<!-- BEGIN JIG").unwrap_or(block),
    ] {
        let mut at = 0;
        for token in tokens.iter().skip(if hay.starts_with("Jig workflow") {
            0
        } else {
            1
        }) {
            let i = hay[at..]
                .find(token)
                .unwrap_or_else(|| panic!("{token:?} missing at {at} in {hay:?}"));
            at += i + token.len();
        }
        for gone in [
            ["Entro", "pyJig"].concat(),
            ["ENTROPY", "JIG"].concat(),
            "not approval to implement a contract".to_string(),
            "Readiness is not approval".to_string(),
        ] {
            assert!(!hay.contains(&gone), "{gone} leaked into {hay}");
        }
    }
}

#[test]
fn inconsistent_fields_are_readable_but_not_writable_or_claimable() {
    for (kind, patch) in [
        (
            "assignee",
            fm("X", "open", "").replace("assignee: null", "assignee: \"x\""),
        ),
        (
            "claim_id",
            fm("X", "open", "").replace("claim_id: null", "claim_id: \"c\""),
        ),
        (
            "no_claim_id",
            fm("X", "claimed", "").replace("assignee: null", "assignee: \"bot\""),
        ),
        (
            "closed_at",
            fm("X", "open", "").replace("closed_at: null", "closed_at: \"2026-01-01\""),
        ),
        (
            "closing_summary",
            fm("X", "open", "")
                .replace("## History\n", "## Closing Summary\n\n> s\n\n## History\n"),
        ),
    ] {
        let d = dir("incon");
        ok(&d, &["init", "--no-hooks"]);
        let root = "B-a00000";
        let target = "B-a00000.1";
        let child = "B-a00000.1.1";
        let dep = "B-de0000";
        fs::write(d.join(".jig/open/B-a00000.md"), fm(root, "open", "")).unwrap();
        let target_text = patch
            .replace("id: \"X\"", &format!("id: \"{target}\""))
            .replace("title: \"T X\"", &format!("title: \"T {target}\""))
            .replace("parent: null", &format!("parent: \"{root}\""));
        fs::write(d.join(format!(".jig/open/{target}.md")), &target_text).unwrap();
        fs::write(
            d.join(format!(".jig/open/{child}.md")),
            fm(child, "open", "").replace("parent: null", &format!("parent: \"{target}\"")),
        )
        .unwrap();
        fs::write(
            d.join(format!(".jig/open/{dep}.md")),
            fm(dep, "open", "").replace("depends_on: []", &format!("depends_on: [\"{target}\"]")),
        )
        .unwrap();
        let path = format!(".jig/open/{target}.md");

        // readable: list, list --json, tree, show all include the record
        for args in [
            vec!["list"],
            vec!["list", "--json"],
            vec!["tree", root],
            vec!["tree", target],
        ] {
            let o = run(&d, &args);
            assert!(o.status.success(), "{kind} {args:?}");
            let stdout = String::from_utf8(o.stdout).unwrap();
            let (want_t, want_c) = if args.contains(&"--json") {
                (
                    format!("\"id\":\"{target}\""),
                    format!("\"id\":\"{child}\""),
                )
            } else {
                (format!("{target} "), format!("{child} "))
            };
            assert!(stdout.contains(&want_t), "{kind} {args:?}: {stdout}");
            assert!(stdout.contains(&want_c), "{kind} {args:?}: {stdout}");
            let stderr = String::from_utf8_lossy(&o.stderr);
            let line = format!("{path}: ");
            assert!(
                stderr
                    .lines()
                    .any(|l| l.contains(&line) && l.contains("jig check")),
                "{kind} {args:?} must warn about {path} without skipping it: {stderr}"
            );
            assert!(
                !stderr.contains(&format!("skipping {path}")),
                "{kind} {args:?} record is listed, not skipped: {stderr}"
            );
        }
        let show: Value = serde_json::from_str(&ok(&d, &["show", target, "--json"])).unwrap();
        match kind {
            "assignee" => assert_eq!(show["assignee"], "x", "{kind}"),
            "claim_id" => {
                assert!(
                    ok(&d, &["show", target]).contains("claim_id: \"c\""),
                    "{kind}"
                )
            }
            "no_claim_id" => {
                assert_eq!(show["status"], "claimed", "{kind}");
                assert!(show.get("claim_id").is_none(), "{kind}");
                assert!(
                    ok(&d, &["show", target]).contains("claim_id: null"),
                    "{kind}"
                );
            }
            "closed_at" => assert_eq!(show["closed_at"], "2026-01-01", "{kind}"),
            "closing_summary" => assert_eq!(show["closing_summary"], "s", "{kind}"),
            _ => unreachable!(),
        }

        // not claimable, not in ready; dependents behave as with an ordinary blocker
        let o = run(&d, &["ready"]);
        assert!(o.status.success());
        let stdout = String::from_utf8(o.stdout).unwrap();
        assert!(
            !stdout.contains(&format!("{target} ")),
            "{kind} ready: {stdout}"
        );
        assert!(
            stdout.contains(&format!("{child} ")),
            "{kind} ready child: {stdout}"
        );
        assert!(!stdout.contains(dep), "{kind} ready dep: {stdout}");
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(
            stderr
                .lines()
                .any(|l| l.contains(&format!("skipping {path}"))),
            "{kind} ready must skip {path} with a warning: {stderr}"
        );
        for args in [
            vec!["claim", target],
            vec!["claim", target, "--force"],
            vec!["update", target, "--title", "x"],
            vec!["close", target, "--reason", "r"],
        ] {
            let e = bad(&d, &args);
            assert!(
                e.contains(target) && (e.contains("jig check") || e.contains("issue")),
                "{kind} {args:?}: {e}"
            );
        }
        if kind == "no_claim_id" {
            let e = bad(&d, &["unclaim", target]);
            assert!(e.contains("jig check"), "{kind} unclaim: {e}");
        }
        let before = tree_bytes(&d.join(".jig"));
        let e = bad(&d, &["claim", dep]);
        assert!(
            e.contains("blocked") && e.contains(target) && !e.contains("unresolved"),
            "{kind} dep claim: {e}"
        );
        let out = ok(&d, &["claim", dep, "--force"]);
        assert!(out.contains(dep), "{kind} {out}");
        let after = tree_bytes(&d.join(".jig"));
        let changed: Vec<_> = before
            .iter()
            .zip(after.iter())
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a.0.clone())
            .collect();
        assert_eq!(
            changed,
            vec![format!("open/{dep}.md")],
            "{kind} only the dependent may change: {changed:?}"
        );

        let o = run(&d, &["check", "--json"]);
        let problems: Value = serde_json::from_str(&String::from_utf8(o.stdout).unwrap()).unwrap();
        let at: Vec<_> = problems
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["path"].as_str() == Some(path.as_str()))
            .collect();
        assert_eq!(at.len(), 1, "{kind} check problems for {path}: {problems}");
        let problem = at[0]["problem"].as_str().unwrap();
        let needle = match kind {
            "assignee" => "assignee",
            "claim_id" => "claim_id",
            "no_claim_id" => "claim_id",
            "closed_at" => "closed_at",
            "closing_summary" => "closing summary",
            _ => unreachable!(),
        };
        assert!(problem.contains(needle), "{kind}: {problem}");
        assert!(
            !problems
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["problem"].as_str().unwrap().contains("unresolved")),
            "{kind} no unresolved references: {problems}"
        );
    }
}
