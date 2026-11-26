use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);

fn dir(name: &str) -> PathBuf {
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join("tests")
        .join(format!("{name}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
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
    ok(dir, &["add", title]).trim().to_string()
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
        "tmp rule must be recursive, got {gi:?}"
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
    assert!(text.contains("status: in_progress"));
    assert!(text.contains("claim_id: \""));
    let i = entropyjig::issue::decode(&text).unwrap();
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
    let cid = |t: &str| entropyjig::issue::decode(t).unwrap().claim_id.unwrap();
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
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{a}.md"))).unwrap(),
    )
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
    let c = ok(&d, &["add", "child", "--parent", &p]).trim().to_string();
    ok(&d, &["defer", &p, "--until", "2999-01-01"]);
    let r = ok(&d, &["ready"]);
    assert!(!r.contains(&c), "child of deferred parent must hide: {r}");
    let e = bad(&d, &["claim", &c, "--force"]);
    assert!(e.contains("deferred"), "{e}");

    let g2 = add(&d, "gp2");
    let mid2 = ok(&d, &["add", "mid2", "--parent", &g2]).trim().to_string();
    let leaf2 = ok(&d, &["add", "leaf2", "--parent", &mid2])
        .trim()
        .to_string();
    ok(&d, &["defer", &g2, "--until", "2999-01-01"]);
    let r = ok(&d, &["ready"]);
    assert!(!r.contains(&mid2));
    assert!(!r.contains(&leaf2), "grandchild must hide: {r}");

    ok(&d, &["close", &g, "--reason", "n/a"]);
    let e = bad(&d, &["defer", &g, "--clear"]);
    assert!(e.contains("closed"), "{e}");

    let today = {
        let i = entropyjig::issue::decode(
            &fs::read_to_string(d.join(format!(".jig/open/{p}.md"))).unwrap(),
        )
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
    let c1 = ok(&d, &["add", "c1", "--parent", &p]).trim().to_string();
    let c2 = ok(&d, &["add", "c2", "--parent", &p]).trim().to_string();
    assert_eq!(c1, format!("{p}.1"));
    assert_eq!(c2, format!("{p}.2"));
    ok(&d, &["close", &c1, "--reason", "done"]);
    let c3 = ok(&d, &["add", "c3", "--parent", &p]).trim().to_string();
    assert_eq!(c3, format!("{p}.3"), "closed child slot must not be reused");
    let g = ok(&d, &["add", "gc", "--parent", &c2]).trim().to_string();
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
    let id = ok(&d, &["add", "feat", "--body", "the description"])
        .trim()
        .to_string();
    ok(&d, &["claim", &id, "--by", "agent"]);
    ok(&d, &["review", &id, "--summary", "round one"]);
    ok(&d, &["request-changes", &id, "--reason", "fix tests"]);
    let text = fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap();
    let i = entropyjig::issue::decode(&text).unwrap();
    assert_eq!(i.status, entropyjig::issue::Status::InProgress);
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
    let i = entropyjig::issue::decode(&text).unwrap();
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
    let i = entropyjig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(i.status, entropyjig::issue::Status::Open);
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
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
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
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
    .unwrap();
    assert_eq!(i.body, "piped\nbody\n", "body bytes preserved exactly");
    ok(&d, &["update", &id, "--title", "retitled"]);
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
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
    let id = ok(&d, &["add", "t \"dq\" pol", "--body", marker])
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
    let dec = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
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
    let e = bad(&d, &["list"]);
    assert!(e.contains("unknown frontmatter key"), "{e}");
    fs::remove_file(d.join(".jig/open/B-dead00.md")).unwrap();

    let closed_copy = good.replace("status: open", "status: closed");
    fs::write(d.join(".jig/closed").join(format!("{id}.md")), &closed_copy).unwrap();
    let before = fs::read_to_string(&path).unwrap();
    let e = bad(&d, &["list"]);
    assert!(e.contains("duplicate"), "{e}");
    let e = bad(&d, &["ready"]);
    assert!(e.contains("duplicate"), "{e}");
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
    let e = bad(&d, &["list"]);
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
    let id = ok(&d, &["add", "t", "--body", body]).trim().to_string();
    let text = fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap();
    let i = entropyjig::issue::decode(&text).unwrap();
    assert_eq!(
        i.body, body,
        "fenced and blockquoted reserved headings stay body"
    );

    let path = d.join(format!(".jig/open/{id}.md"));
    let plain = text.replace("title: \"t\"", "title: plain #tag");
    fs::write(&path, &plain).unwrap();
    let i = entropyjig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(i.title, "plain", "trailing comment stripped");

    let escaped = text.replace("title: \"t\"", "title: \"a\\u0142b\"");
    fs::write(&path, &escaped).unwrap();
    let i = entropyjig::issue::decode(&fs::read_to_string(&path).unwrap()).unwrap();
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
    let i = entropyjig::issue::decode(&fs::read_to_string(d.join(".jig/open/B440.2.md")).unwrap())
        .unwrap();
    assert_eq!(i.id, "B440.2");
    assert_eq!(i.priority, None);
    assert_eq!(i.created_at.as_deref(), Some("2020-05-04"));

    assert!(entropyjig::issue::valid_id("B-abcdef"));
    assert!(entropyjig::issue::valid_id("B-abcdef.2.10"));
    assert!(entropyjig::issue::valid_id("B440"));
    assert!(!entropyjig::issue::valid_id("B-abcdef.0"));
    assert!(!entropyjig::issue::valid_id("B-abcdef.02"));
    assert!(!entropyjig::issue::valid_id("abcdef"));
    assert!(!entropyjig::issue::valid_id("B-ABC123"));
    assert_eq!(
        entropyjig::issue::id_alias("abcdef").as_deref(),
        Some("B-abcdef")
    );
    assert_eq!(
        entropyjig::issue::id_alias("abcdef.2").as_deref(),
        Some("B-abcdef.2")
    );
}

#[test]
fn body_exact_preservation() {
    let d = dir("bodyexact");
    ok(&d, &["init"]);
    let body = "\n````md\n```\n## History\n```\n````\n\ntrailing\n\n";
    let o = run_stdin(&d, &["add", "t", "--body", "-"], body);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let id = String::from_utf8(o.stdout).unwrap().trim().to_string();
    let read = |dirn: &str| {
        entropyjig::issue::decode(
            &fs::read_to_string(d.join(format!(".jig/{dirn}/{id}.md"))).unwrap(),
        )
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
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
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
    let id = ok(&d, &["add", "t", "--dep", &a, &b, "--dep", &c])
        .trim()
        .to_string();
    let i = entropyjig::issue::decode(
        &fs::read_to_string(d.join(format!(".jig/open/{id}.md"))).unwrap(),
    )
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
    let h = run(&d, &["--help"]);
    assert!(h.status.success());
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
    let summary = "Recommended workaround:\n\n```text\ntranscript line\n";
    ok(
        &d,
        &["close", &id, "--reason", "done", "--summary", summary],
    );
    let closed = d.join(format!(".jig/closed/{id}.md"));
    let i = entropyjig::issue::decode(&fs::read_to_string(&closed).unwrap()).unwrap();
    assert_eq!(i.closing_summary.as_deref(), Some(summary));
    ok(&d, &["reopen", &id, "--reason", "regression"]);
    let open = d.join(format!(".jig/open/{id}.md"));
    let i = entropyjig::issue::decode(&fs::read_to_string(&open).unwrap()).unwrap();
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
    let e = bad(&d, &["ready"]);
    assert!(e.contains(&b), "{e}");
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
    assert_eq!(agents.matches("<!-- BEGIN ENTROPYJIG -->").count(), 1);
    assert_eq!(agents.matches("<!-- END ENTROPYJIG -->").count(), 1);
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
