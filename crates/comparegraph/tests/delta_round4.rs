//! Round-4 review probes of the delta model (feat/plan-14 at b07e4c7): each was a false yes (red) on that code. The invariant
//! they pin (docs/decisions.md, "Plan 14 round 4"): every line the branch changed (merge base -> head) is accounted for by a
//! judged row; any change not accounted for is an unknown item. Fixture repos are real git repositories in temp dirs.
#![allow(dead_code)]
mod common;
use common::{git, ok};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const BAR: &str = "def bar(x):\n    y = x * 2\n    return y + 1\n";
const FOO: &str = "def foo():\n    return 1\n";
const KEEP: &str = "def keep():\n    return 1\n";

fn commit(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", msg]);
}
fn write(dir: &Path, rel: &str, body: &str) {
    let f = dir.join(rel);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(f, body).unwrap();
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    /// A clone on branch `main` with the given files, registered as `r` in a fresh store.
    fn new(files: &[(&str, &str)]) -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        write(&d, "LICENSE", MIT);
        for (p, c) in files {
            write(&d, p, c);
        }
        commit(&d, "c1");
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn d(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    fn delta(&self) -> Value {
        ok(self.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"])
    }
    fn feat(&self) {
        git(&self.d(), &["checkout", "-q", "-b", "feat"]);
    }
    fn main(&self) {
        git(&self.d(), &["checkout", "-q", "main"]);
    }
}

/// `def process(items)`: 30 lines with their own names, optionally a 2-line guard and a tweak on the return.
fn long_process(guard: bool, tweak: &str) -> String {
    let mut s = String::from("def process(items):\n    total = 0\n");
    if guard {
        s.push_str("    if not items:\n        raise Denied()\n");
    }
    for i in 0..30 {
        s.push_str(&format!("    total = total + items[{i}] * {} - len(items) + k{i}\n", i + 2));
    }
    s.push_str(&format!("    return total{tweak}\n"));
    s
}
fn long_named(name: &str, guard: bool) -> String {
    long_process(guard, "").replacen("def process(", &format!("def {name}("), 1)
}
fn exact_share(v: &Value) -> Option<f64> {
    v["data"]["summary"]["share_on_base_exact"].as_f64()
}
fn branch(v: &Value) -> &Value {
    &v["data"]["branch"]
}
fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}

/// A row is landed work: `on_base: yes`.
fn exact(row: &Value) -> bool {
    row["on_base"] == "yes"
}
fn row_of<'a>(v: &'a Value, section: &str, qn: &str) -> &'a Value {
    branch(v)[section].as_array().unwrap().iter().find(|r| r["qualified_name"] == qn).unwrap_or_else(|| panic!("no {section} row {qn}: {}", branch(v)))
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}
fn git_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    use std::io::Write;
    let mut c = std::process::Command::new("git").arg("-C").arg(dir).args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

// ================================================================ round-4 review probes

fn summ(v: &Value) -> String {
    format!("{} residue={}", v["data"]["summary"], branch(v)["residue"])
}

const G_LOOP: &str = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n    return total\n";
const G_DEDENT: &str = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n    log(total, x, items)\n    return total\n";

/// R4-1: the branch adds bar (base takes it) and moves g's log call out of the loop (a re-indent: Python semantics change, the
/// token stream does not). Base does not have the re-indent.
#[test]
fn r4_python_reindent_inside_an_unchanged_symbol() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G_LOOP}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{G_DEDENT}\n{BAR}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{G_LOOP}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (re-indent lost): {}", summ(&v));
}

/// R4-1b: a whitespace fix inside a string literal of an unchanged symbol.
#[test]
fn r4_string_whitespace_inside_an_unchanged_symbol() {
    let before = "def greet(name):\n    msg = \"Hello,world and welcome\"\n    out = msg + name + suffix(name, 1)\n    return out\n";
    let after = "def greet(name):\n    msg = \"Hello, world and welcome\"\n    out = msg + name + suffix(name, 1)\n    return out\n";
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{before}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{after}\n{BAR}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{before}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (string whitespace lost): {}", summ(&v));
}

/// R4-1c: A1 on a body whose tokens base took but whose indentation base did not.
#[test]
fn r4_a1_same_tokens_other_indentation() {
    let lane = G_DEDENT.replace("* 3", "* 4");
    let base = G_LOOP.replace("* 3", "* 4");
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G_LOOP}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{lane}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{base}\n"));
    commit(&d, "base");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (A1 ignores indentation): {}", summ(&v));
}

/// R4-2: the s4 top-level probe with `diff.noprefix = true` in the clone's git config (a common user setting).
#[test]
fn r4_noprefix_config_hides_top_level_work() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.d();
    git(&d, &["config", "diff.noprefix", "true"]);
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}\nRETRIES = compute_retries(3, backoff=2)\n"));
    commit(&d, "lane adds bar and a module constant");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (noprefix): {}", summ(&v));
}

/// R4-3: the s4 top-level probe with `*.py -diff` in the checked-out .gitattributes (generated files are often marked so).
#[test]
fn r4_minus_diff_attribute_hides_top_level_work() {
    let fx = Fx::new(&[("src/a.py", FOO), (".gitattributes", "*.py -diff\n")]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}\nRETRIES = compute_retries(3, backoff=2)\n"));
    commit(&d, "lane adds bar and a module constant");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (-diff attribute): {}", summ(&v));
}

/// R4-4: A5 checks unread blobs but not gitlinks. The merge base pins a submodule `lib` (never read); the branch adds a copy of
/// helper; base drops the submodule and adds helper in-tree. The helper may have been in the submodule at the merge base (the P3
/// shape: a copy of old code with base moving it), so the branch's copy is not known to be on base.
#[test]
fn r4_a5_ignores_a_gitlink_base_removed() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.d();
    let tip = git_out(&d, &["rev-parse", "main"]);
    let mut entries = std::process::Command::new("git").arg("-C").arg(&d).args(["ls-tree", "main"]).output().unwrap().stdout;
    entries.extend_from_slice(format!("160000 commit {tip}\tlib\n").as_bytes());
    let tree = git_in(&d, &["mktree"], &entries);
    let c = git_in(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit-tree", &tree, "-p", "main", "-m", "pin lib"], b"");
    git(&d, &["update-ref", "refs/heads/main", &c]);
    git(&d, &["read-tree", "main"]);
    fx.feat();
    write(&d, "src/b.py", &long_named("helper", false));
    git(&d, &["add", "src/b.py"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "lane copies helper"]);
    fx.main();
    git(&d, &["rm", "-q", "--cached", "lib"]);
    write(&d, "src/lib.py", &long_named("helper", false));
    git(&d, &["add", "src/lib.py"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "base inlines lib"]);
    let v = fx.delta();
    let row = row_of(&v, "added", "src/b.py::helper");
    assert_ne!(row["on_base"], "yes", "FALSE YES (gitlink ignored by A5): {row} {}", summ(&v));
    // round 5: the backstop also stops this yes; the model's own reason is pinned so the specific guard cannot go unseen
    assert_eq!(row["why"], "merge_base_unread", "{row}");
}

/// R4-5: a removed line starting with `-- ` (SQL in a module string) prints as `--- ...` in the diff and is read as a file header.
#[test]
fn r4_removed_dash_dash_line_is_read_as_a_header() {
    let q = "Q = \"\"\"\nSELECT 1\n-- only active users\nFROM users\n\"\"\"\n";
    let q2 = "Q = \"\"\"\nSELECT 1\nFROM users\n\"\"\"\n";
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{q}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{q2}\n{BAR}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{q}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (-- line read as a header): {}", summ(&v));
}

/// R4-6: an empty branch, and a branch whose only change is a re-indent: the share must never be 1.0.
#[test]
fn r4_empty_and_whitespace_only_branches() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G_LOOP}"))]);
    let d = fx.d();
    fx.feat();
    let v = fx.delta();
    eprintln!("EMPTY: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0));
    write(&d, "src/a.py", &format!("{FOO}\n{G_DEDENT}"));
    commit(&d, "lane re-indents");
    let v = fx.delta();
    eprintln!("REINDENT ONLY: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0));
    assert!(v["data"]["summary"]["branch_items"].as_u64().unwrap_or(0) > 0, "WORK INVISIBLE: {}", summ(&v));
}

/// R4-7: the branch deletes f's file; base renames that same path to a new path with other content (same new path both sides).
#[test]
fn r4_both_sides_rename_to_one_path_with_other_contents() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{}", long_named("f", false)))]);
    let d = fx.d();
    fx.feat();
    git(&d, &["mv", "src/a.py", "src/n.py"]);
    write(&d, "src/n.py", &format!("{FOO}\n{}", long_named("f", true)));
    commit(&d, "lane moves and changes f");
    fx.main();
    git(&d, &["mv", "src/a.py", "src/n.py"]);
    commit(&d, "base moves only");
    let v = fx.delta();
    eprintln!("RENAME: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R4-8: a CRLF-only conversion of a file by the branch (blob oids change, tokens do not).
#[test]
fn r4_crlf_conversion_only() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G_LOOP}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{G_LOOP}").replace('\n', "\r\n"));
    commit(&d, "lane converts to crlf");
    let v = fx.delta();
    eprintln!("CRLF: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0));
    // the CRLF-only change is the branch's work: at least one item (the coordinator's round-4 note)
    assert!(v["data"]["summary"]["branch_items"].as_u64().unwrap_or(0) > 0, "WORK INVISIBLE: {}", summ(&v));
}

/// R4-9: base keeps bar, but base's cached tree lost src/a.py (by hand, an interrupted gc, antivirus). The symbols of that file
/// become "unreadable" and are dropped from the copy counts and the key lookup: the branch's deletion of bar reads absent_on_base.
#[test]
fn r4_unreadable_base_symbols_make_a_deletion_yes() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{BAR}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes bar");
    fx.main();
    write(&d, "src/k.py", &format!("{KEEP}\n# base touches k\n"));
    commit(&d, "base keeps bar");
    let _ = fx.delta();
    let bc = git_out(&d, &["rev-parse", "main"]);
    let f = fx.root().join(".comparegraph").join("refs").join("r").join(&bc).join("src/a.py");
    assert!(f.is_file(), "{}", f.display());
    std::fs::remove_file(&f).unwrap();
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::bar");
    assert_ne!(row["on_base"], "yes", "FALSE YES (unreadable dropped): {row} {} {}", summ(&v), limits(&v));
}

/// R4-10: the repo's `add` subset excludes legacy/. Base moves bar into legacy/ (outside the subset, so in no count and not in
/// U(base)); the branch deletes bar. Base still holds bar.
#[test]
fn r4_base_moves_a_deleted_symbol_outside_the_subset() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{BAR}"))]);
    ok(fx.root(), &["add", "--name", "r", "--exclude", "legacy/**"]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes bar");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n"));
    write(&d, "legacy/old.py", BAR);
    commit(&d, "base moves bar to legacy");
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::bar");
    assert_ne!(row["on_base"], "yes", "FALSE YES (base kept bar outside the subset): {row} {}", summ(&v));
    // round 5: the backstop also stops this yes; the model's own reason is pinned so the specific guard cannot go unseen
    assert_eq!(row["why"], "base_unread", "{row}");
}

/// R4-11: the P3 shape through an excluded folder: the merge base holds helper only in legacy/ (outside the subset); the branch
/// adds a copy in src/; base moves legacy/helper into src/. Base made no new copy; the branch did.
#[test]
fn r4_a4_credit_from_code_outside_the_subset() {
    let fx = Fx::new(&[("src/a.py", FOO), ("legacy/h.py", &long_named("helper", false))]);
    ok(fx.root(), &["add", "--name", "r", "--exclude", "legacy/**"]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/b.py", &long_named("helper", false));
    commit(&d, "lane copies helper");
    fx.main();
    git(&d, &["mv", "legacy/h.py", "src/h.py"]);
    commit(&d, "base moves helper into src");
    let v = fx.delta();
    let row = row_of(&v, "added", "src/b.py::helper");
    assert_ne!(row["on_base"], "yes", "FALSE YES (credit from an excluded copy): {row} {}", summ(&v));
    // round 5: the backstop also stops this yes; the model's own reason is pinned so the specific guard cannot go unseen
    assert_eq!(row["why"], "merge_base_unread", "{row}");
}

/// R4-12: a submodule bump by the branch, with `ignore = all` in .gitmodules: the bump is the branch's work.
#[test]
fn r4_submodule_bump_with_ignore_all() {
    let fx = Fx::new(&[("src/a.py", FOO), (".gitmodules", "[submodule \"lib\"]\n\tpath = lib\n\turl = ./x\n\tignore = all\n")]);
    let d = fx.d();
    let pin = |d: &Path, c: &str, msg: &str| {
        let mut entries = std::process::Command::new("git").arg("-C").arg(d).args(["ls-tree", "HEAD"]).output().unwrap().stdout;
        let s = String::from_utf8(entries.clone()).unwrap();
        let kept: String = s.lines().filter(|l| !l.ends_with("\tlib")).map(|l| format!("{l}\n")).collect();
        entries = kept.into_bytes();
        entries.extend_from_slice(format!("160000 commit {c}\tlib\n").as_bytes());
        let tree = git_in(d, &["mktree"], &entries);
        let nc = git_in(d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit-tree", &tree, "-p", "HEAD", "-m", msg], b"");
        git(d, &["reset", "-q", "--soft", &nc]);
        git(d, &["read-tree", "HEAD"]);
    };
    let c1 = git_out(&d, &["rev-parse", "HEAD"]);
    pin(&d, &c1, "pin lib at c1");
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    git(&d, &["add", "src/a.py"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "lane adds bar"]);
    let c2 = git_out(&d, &["rev-parse", "HEAD"]);
    pin(&d, &c2, "lane bumps lib");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    git(&d, &["add", "src/a.py"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "base takes bar"]);
    let v = fx.delta();
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (submodule bump unseen): {}", summ(&v));
    // round 5: the backstop also stops this yes; the model's own reason is pinned so the specific guard cannot go unseen
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["gitlink_changed"].as_i64(), Some(1), "{}", summ(&v));
}

// ================================================================ round 4: guards pinned against their mutations

/// Remove one loose object of the clone (a fixture repo: every object is loose).
fn drop_object(d: &Path, oid: &str) {
    let obj = d.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
    let mut p = std::fs::metadata(&obj).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    p.set_readonly(false);
    std::fs::set_permissions(&obj, p).unwrap();
    std::fs::remove_file(&obj).unwrap();
}

/// R3 `branch_moved`: the branch deletes f from a.py and adds its very body under another name; base deletes f outright. Head
/// still holds as many copies of f's body as the merge base (h >= m): the branch only moved it, so the deletion is unknown.
#[test]
fn g4_a_deletion_the_branch_only_moved_is_unknown() {
    let fx = Fx::new(&[("src/a.py", &format!("{KEEP}\n{}", long_named("f", true)))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", KEEP);
    write(&d, "src/b.py", &long_named("g", true));
    commit(&d, "lane moves f's body to g");
    fx.main();
    write(&d, "src/a.py", &format!("{KEEP}\n"));
    commit(&d, "base deletes f");
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::f");
    assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("branch_moved")), "{row}");
}

/// R3 `base_copies`: the merge base holds f's body twice (f and f2); the branch deletes f; base deletes f and keeps f2. Base has
/// fewer copies than the merge base but not none: unknown.
#[test]
fn g4_a_deletion_with_fewer_base_copies_is_unknown() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{}", long_named("f", true), long_named("f2", true)))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &long_named("f2", true));
    commit(&d, "lane deletes f");
    fx.main();
    write(&d, "src/a.py", &format!("{}\n", long_named("f2", true)));
    commit(&d, "base deletes f too, its file otherwise");
    let v = fx.delta();
    let row = row_of(&v, "removed", "src/a.py::f");
    assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("base_copies")), "{row}");
}

/// `sub()` with an unknown unread set: base's tree can no longer be listed (its root tree object is gone from the clone), so
/// which files base's run did not read is not known, and a deletion base made too is not a yes.
#[test]
fn g4_an_unlisted_base_tree_blocks_a_removal_yes() {
    let fx = Fx::new(&[("src/a.py", &format!("{}\n{KEEP}", long_process(false, ""))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", KEEP);
    commit(&d, "lane deletes process");
    fx.main();
    write(&d, "src/a.py", &format!("{KEEP}\n"));
    commit(&d, "base deletes it too");
    let v = fx.delta();
    assert_eq!(row_of(&v, "removed", "src/a.py::process")["via"], "absent_on_base", "{}", branch(&v));
    let tree = git_out(&d, &["rev-parse", "main^{tree}"]);
    drop_object(&d, &tree);
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    let row = row_of(&v, "removed", "src/a.py::process");
    assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("base_unread")), "{row}");
    assert!(limits(&v).contains("git ls-tree failed for base"), "{}", limits(&v));
}

/// A5's U(mb) within U(head) half: the merge base's run could not read big.py (over max_file_bytes), which holds g; head trims
/// it (head's run reads g) and adds a copy h; base keeps big.py as it was and adds two copies. Head's g is old code: no credit.
#[test]
fn g4_a_credit_needs_the_merge_base_read_whole_on_head_too() {
    let pad = format!("# {}\n", "x".repeat(1_100_000));
    let fx = Fx::new(&[("src/big.py", &format!("{}\n{pad}", long_named("g", true))), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/big.py", &long_named("g", true));
    write(&d, "src/n.py", &long_named("h", true));
    commit(&d, "lane trims big.py and adds h");
    fx.main();
    write(&d, "src/o.py", &format!("{}\n{}", long_named("p", true), long_named("q", true)));
    commit(&d, "base adds two copies");
    let v = fx.delta();
    for qn in ["src/big.py::g", "src/n.py::h"] {
        let row = row_of(&v, "added", qn);
        assert_eq!((row["on_base"].as_str(), row["why"].as_str()), (Some("unknown"), Some("merge_base_unread")), "{row}");
    }
}

/// The residue's removed-lines side: the branch deletes a top-level statement (and adds bar, which base takes); base keeps the
/// statement. The deletion is in no judged row: one unknown item.
#[test]
fn g4_an_unlanded_deletion_of_top_level_code_is_an_item() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\nRETRIES = compute_retries(3, backoff=2)\n"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "lane drops the constant and adds bar");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\nRETRIES = compute_retries(3, backoff=2)\n\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    // round 5: bar is same_symbol to the model, and the merge backstop turns it unknown (the merged a.py drops the constant)
    assert_eq!(row_of(&v, "added", "src/a.py::bar")["model_via"], "same_symbol", "{}", branch(&v));
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["outside_symbols"].as_i64(), Some(1), "{}", summ(&v));
    assert_eq!(exact_share(&v), Some(0.0), "{}", summ(&v));
}

/// The whole-tree comparison subtracts the head symbols base holds as they are: head's new g copies f's body, and base's only
/// copy of that body is f itself.
#[test]
fn g4_whole_tree_credit_leaves_out_what_base_holds_as_it_is() {
    let fx = Fx::new(&[("src/a.py", &long_named("f", true))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/b.py", &long_named("g", true));
    commit(&d, "lane adds g, a copy of f");
    fx.main();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--whole-tree"]);
    let g = v["data"]["whole_tree"]["added"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/b.py::g").unwrap().clone();
    assert_eq!(g["on_base"], "no", "{g}");
}

/// A diff git cannot give (head's blob of a touched file is gone from the clone) is one unknown item, never a silent pass.
#[test]
fn g4_a_diff_that_fails_is_an_item() {
    let fx = Fx::new(&[("src/a.py", FOO)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}"));
    commit(&d, "lane adds bar");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{BAR}\n"));
    commit(&d, "base takes bar, its file otherwise");
    let v = fx.delta();
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"].get("diff_failed"), None, "{}", summ(&v));
    let blob = git_out(&d, &["rev-parse", "feat:src/a.py"]);
    drop_object(&d, &blob);
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat"]);
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["diff_failed"].as_i64(), Some(1), "{}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "{}", summ(&v));
}
