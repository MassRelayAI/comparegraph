//! Round-5 review probes of the delta model (feat/plan-14 at de1f50c): each read share 1.0 where the branch holds work base
//! does not (or pins a guard). The backstop (docs/decisions.md, "Plan 14 round 5") checks every yes against `git merge-tree`.
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

// ================================================================ round-5 review probes

fn summ(v: &Value) -> String {
    format!("{} residue={} rows={}", v["data"]["summary"], branch(v)["residue"], branch(v))
}

const G5: &str = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n    return total\n";

/// R5-1: the branch makes a file executable AND changes a function in it; base takes the function change, not the mode.
#[cfg(unix)]
#[test]
fn r5_mode_change_with_content_change() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2);
    std::fs::set_permissions(d.join("src/a.py"), std::fs::Permissions::from_mode(0o755)).unwrap();
    commit(&d, "lane: chmod +x and change g");
    eprintln!("feat ls: {}", git_out(&d, &["ls-tree", "-r", "feat"]));
    fx.main();
    write(&d, "src/a.py", &g2);
    std::fs::set_permissions(d.join("src/a.py"), std::fs::Permissions::from_mode(0o644)).unwrap();
    commit(&d, "base takes g only");
    eprintln!("main ls: {}", git_out(&d, &["ls-tree", "-r", "main"]));
    let v = fx.delta();
    eprintln!("MODE: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (mode lost): {}", summ(&v));
}

/// R5-2: a whitespace-only line inside a string literal of a Rust impl method (indented, no enclosing symbol): narrower than
/// the common indent, the raw text strips it to nothing on both sides.
#[test]
fn r5_rust_whitespace_line_in_string_under_common_indent() {
    let m = |blank: &str, n: u32| format!("pub struct S;\n\nimpl S {{\n    pub fn m(&self) -> String {{\n        let s = \"alpha\n{blank}\n        beta\";\n        format!(\"{{}} {{}} {{}} {{}}\", s, 1, 2, {n})\n    }}\n}}\n");
    let fx = Fx::new(&[("src/a.rs", &m("", 3))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.rs", &m("  ", 4));
    commit(&d, "lane: string blank line gets two spaces, 3 -> 4");
    fx.main();
    write(&d, "src/a.rs", &m("", 4));
    commit(&d, "base takes 3 -> 4 only");
    let v = fx.delta();
    eprintln!("RUSTWS: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (string whitespace lost): {}", summ(&v));
}

/// R5-3: tabs to spaces in the indentation of a Rust impl method whose string literal continues over lines.
#[test]
fn r5_rust_tab_indent_in_string_continuation() {
    let m = |ind: &str, n: u32| format!("pub struct S;\n\nimpl S {{\n{ind}pub fn m(&self) -> String {{\n{ind}{ind}let s = \"alpha\n{ind}{ind}beta\";\n{ind}{ind}format!(\"{{}} {{}} {{}} {{}}\", s, 1, 2, {n})\n{ind}}}\n}}\n");
    let fx = Fx::new(&[("src/a.rs", &m("\t", 3))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.rs", &m("    ", 4));
    commit(&d, "lane: tabs to spaces, 3 -> 4");
    fx.main();
    write(&d, "src/a.rs", &m("\t", 4));
    commit(&d, "base takes 3 -> 4 only");
    let v = fx.delta();
    eprintln!("RUSTTAB: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (string content lost): {}", summ(&v));
}

/// R5-4: CRLF conversion plus a real change, in a file holding one function only; base takes the change, not the endings.
#[test]
fn r5_crlf_with_a_change_in_a_one_function_file() {
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2.replace('\n', "\r\n"));
    commit(&d, "lane: crlf + change");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base takes the change");
    let v = fx.delta();
    eprintln!("CRLF5: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (line endings lost): {}", summ(&v));
}

/// R5-5: the branch deletes g; base keeps g but its file no longer parses cleanly (tree-sitter drops g into an ERROR node): the
/// file has a file node, so it is not in U(base), and b(y) = 0 reads as absent.
#[test]
fn r5_base_parse_error_hides_a_kept_symbol() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes g");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\nx = (1,\n{G5}"));
    commit(&d, "base keeps g, breaks the parse above it");
    let v = fx.delta();
    eprintln!("PARSE: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (base still holds g): {}", summ(&v));
}

/// R5-5b: the same with valid Python 3.12 (PEP 701 nested quotes in an f-string) that an older grammar cannot parse.
#[test]
fn r5_base_pep701_fstring_hides_a_kept_symbol() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes g");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\nd = {{'k': 1}}\nmsg = f\"{{d[\"k\"]}}\"\n{G5}"));
    commit(&d, "base keeps g, adds a 3.12 f-string above it");
    let v = fx.delta();
    eprintln!("PEP701: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (base still holds g): {}", summ(&v));
}

/// R5-7: the branch replaces a function file with a symlink to another file holding the same body.
#[cfg(unix)]
#[test]
fn r5_regular_file_becomes_a_symlink() {
    let fx = Fx::new(&[("src/a.py", G5), ("src/b.py", FOO)]);
    let d = fx.d();
    fx.feat();
    std::fs::remove_file(d.join("src/a.py")).unwrap();
    std::os::unix::fs::symlink("b.py", d.join("src/a.py")).unwrap();
    commit(&d, "lane: a.py -> b.py");
    fx.main();
    write(&d, "src/c.py", "def other():\n    return 2\n");
    commit(&d, "base moves on");
    let v = fx.delta();
    eprintln!("SYMLINK: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (symlink lost): {}", summ(&v));
}

/// R5-8: a hunk that straddles a judged row and an unchanged symbol next to it (no blank line between them).
#[test]
fn r5_hunk_straddles_judged_and_unchanged() {
    let a = "def f(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n    return total\ndef h(items):\n    out = []\n    for x in items:\n        out.append(x * 7 + len(items) - 1)\n    return out\n";
    // lane: changes f's return and the first line of h's body, by tokens that keep h's hash? no: h changes too; so use a
    // whitespace-only change in h's string-free body (re-indent of a line keeps tokens): h unchanged by tokens
    let lane = a.replace("    return total\ndef h(items):\n    out = []", "    return total + 1\ndef h(items):\n    out =  []");
    let fx = Fx::new(&[("src/a.py", a)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &lane);
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &a.replace("    return total\n", "    return total + 1\n"));
    commit(&d, "base takes f only");
    let v = fx.delta();
    eprintln!("STRADDLE: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R5-9: a file that becomes empty; an empty file added; a file with no newline at its end.
#[test]
fn r5_empty_and_no_eol_files() {
    let fx = Fx::new(&[("src/a.py", G5), ("src/b.py", &format!("{FOO}\nTOP = 1"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", "");
    write(&d, "src/e.py", "");
    write(&d, "src/b.py", &format!("{FOO}\nTOP = 2"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", "");
    commit(&d, "base empties a only");
    let v = fx.delta();
    eprintln!("EMPTY: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R5-10: binary content with a NUL byte in a parsed-language file, next to a function the base takes.
#[test]
fn r5_nul_bytes_in_a_parsed_file() {
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    std::fs::write(d.join("src/a.py"), format!("{g2}X = '\u{0}'\n")).unwrap();
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base");
    let v = fx.delta();
    eprintln!("NUL: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R5-11: a function whose body mentions its own name, changed in a way the mask hides: `g` replaced by another name the
/// mask does not touch, vs a self-call; the base holds a body where the self-reference is spelled as the name.
#[test]
fn r5_name_mask_hides_a_self_reference_change() {
    let mb = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n    return total + h(items)\n";
    let lane = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n    return total + g(items)\n";
    let fx = Fx::new(&[("src/a.py", mb)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", lane);
    commit(&d, "lane: recursive");
    fx.main();
    // base: same file but the function is renamed k and calls k: masked text is equal to the lane's g calling g
    write(&d, "src/a.py", &lane.replace("g(", "k("));
    commit(&d, "base: k calls k");
    let v = fx.delta();
    eprintln!("MASK: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R5-6: a file the subset excludes, changed by the branch.
#[test]
fn r5_excluded_file_changed_by_the_branch_is_an_item() {
    let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let d = base.path().join("r");
    std::fs::create_dir_all(&d).unwrap();
    git(&d, &["init", "-q", "-b", "main"]);
    write(&d, "LICENSE", MIT);
    write(&d, "src/a.py", G5);
    write(&d, "vendor/v.py", G5);
    commit(&d, "c1");
    ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r", "--exclude", "vendor/**"]);
    git(&d, &["checkout", "-q", "-b", "feat"]);
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2);
    write(&d, "vendor/v.py", &G5.replace("x * 3", "x * 5"));
    commit(&d, "lane");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &g2);
    commit(&d, "base takes a.py only");
    let v = ok(ws.path(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"]);
    eprintln!("EXCL: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (excluded file invisible): {}", summ(&v));
}

/// R5-12: two non-UTF-8 paths that read as one lossy string: the TreeIndex keeps one of them, the other's change is invisible.
#[cfg(unix)]
#[test]
fn r5_lossy_path_collision_hides_a_change() {
    use std::os::unix::ffi::OsStrExt;
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    let p1 = d.join(std::ffi::OsStr::from_bytes(b"notes\xfe.txt"));
    let p2 = d.join(std::ffi::OsStr::from_bytes(b"notes\xff.txt"));
    std::fs::write(&p1, "one\n").unwrap();
    std::fs::write(&p2, "two\n").unwrap();
    commit(&d, "two non-utf8 paths"); // (the review's probe also ran `git branch -f main` on main itself, which git refuses)
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2);
    std::fs::write(&p1, "one CHANGED by the lane\n").unwrap();
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base takes a.py only");
    let v = fx.delta();
    eprintln!("LOSSY: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (non-utf8 path change invisible): {}", summ(&v));
}

/// R5-13: the whole-tree share with a SQL change head holds and base does not.
#[test]
fn r5_whole_tree_share_ignores_non_symbol_work() {
    let fx = Fx::new(&[("src/a.py", G5), ("db/m.sql", "select 1;\n")]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2);
    write(&d, "db/m.sql", "drop table users;\n");
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base takes a.py only");
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--whole-tree"]);
    eprintln!("WHOLE: {} | branch share {:?}", v["data"]["whole_tree"]["summary"], exact_share(&v));
    assert_ne!(v["data"]["whole_tree"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "whole-tree share 1.0 with the SQL change missing");
}

/// R5-5c: the branch deletes g; base keeps g but writes it with PEP 695 type parameters (valid Python 3.12).
#[test]
fn r5_base_pep695_generic_hides_a_kept_symbol() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes g");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{}", G5.replace("def g(items):", "def g[T](items: list[T]) -> T:")));
    commit(&d, "base keeps g, generic");
    let v = fx.delta();
    eprintln!("PEP695: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (base still holds g): {}", summ(&v));
}

/// R5-5d: the same with a Python 3.12 `type` alias statement above g.
#[test]
fn r5_base_type_alias_hides_a_kept_symbol() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes g");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\ntype Pair[T] = tuple[T, T]\n{G5}"));
    commit(&d, "base keeps g, adds a type alias");
    let v = fx.delta();
    eprintln!("TYPEALIAS: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES (base still holds g): {}", summ(&v));
}

/// R5-14: the clone sets diff.interHunkContext: -U0 hunks are fused with context lines.
#[test]
fn r5_inter_hunk_context_config() {
    let mb = format!("X = 1\n{G5}");
    let fx = Fx::new(&[("src/a.py", &mb)]);
    let d = fx.d();
    git(&d, &["config", "diff.interHunkContext", "5"]);
    fx.feat();
    let lane = format!("X = 2\n{}", G5.replace("x * 3", "x * 4"));
    write(&d, "src/a.py", &lane);
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("X = 1\n{}", G5.replace("x * 3", "x * 4")));
    commit(&d, "base takes g only");
    let v = fx.delta();
    eprintln!("IHC: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

/// R5-15: a blank line added inside a module-level string (a hunk of one blank line outside every span), next to work base took.
#[test]
fn r5_blank_line_in_a_module_string() {
    let mb = format!("DOC = \"\"\"\nline one\nline two\n\"\"\"\n\n{G5}");
    let fx = Fx::new(&[("src/a.py", &mb)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &format!("DOC = \"\"\"\nline one\n\nline two\n\"\"\"\n\n{g2}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("DOC = \"\"\"\nline one\nline two\n\"\"\"\n\n{g2}"));
    commit(&d, "base takes g only");
    let v = fx.delta();
    eprintln!("BLANK: {}", summ(&v));
    assert_ne!(exact_share(&v), Some(1.0), "FALSE YES: {}", summ(&v));
}

// ================================================================ round 5: the backstop

/// The backstop never takes an honest yes away: base took the branch's change exactly (an add, a change and a deletion, in
/// three files), so git's merge of head into base is base itself.
#[test]
fn r5_backstop_keeps_an_honest_yes() {
    let fx = Fx::new(&[("src/a.py", FOO), ("src/g.py", G5), ("src/d.py", &format!("{KEEP}\n{}", long_named("gone", true)))]);
    let d = fx.d();
    let lane = |d: &Path| {
        write(d, "src/a.py", &format!("{FOO}\n{BAR}"));
        write(d, "src/g.py", &G5.replace("x * 3", "x * 4"));
        write(d, "src/d.py", KEEP);
    };
    fx.feat();
    lane(&d);
    commit(&d, "lane");
    fx.main();
    lane(&d);
    write(&d, "src/other.py", "def other(q):\n    r = q + 7\n    return r * 3\n");
    commit(&d, "base takes it all, and moves on");
    let v = fx.delta();
    assert_eq!(exact_share(&v), Some(1.0), "{}", summ(&v));
    assert!(v["data"]["summary"]["unknown"]["by_why"].as_object().unwrap().is_empty(), "{}", summ(&v));
}

/// The backstop turns a yes the merge contradicts unknown and says how the model had it: the lane adds bar and base takes bar
/// but not the lane's other edit to the same file.
#[test]
fn r5_backstop_turns_a_contradicted_yes_unknown() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}"))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &format!("{FOO}\n{}\n{BAR}", G5.replace("x * 3", "x * 4")));
    commit(&d, "lane changes g and adds bar");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\n{G5}\n{BAR}"));
    commit(&d, "base takes bar only");
    let v = fx.delta();
    let bar = row_of(&v, "added", "src/a.py::bar");
    assert_eq!((bar["on_base"].as_str(), bar["why"].as_str(), bar["model_via"].as_str()), (Some("unknown"), Some("merge_changes_file"), Some("same_symbol")), "{bar}");
}

// ================================================================ round 5: the specific fixes (each read on its own, under the backstop)

fn by_why(v: &Value, w: &str) -> i64 {
    v["data"]["summary"]["unknown"]["by_why"][w].as_i64().unwrap_or(0)
}

/// B-1: a mode the branch changed, base did not, is an item even when the content changed too.
#[cfg(unix)]
#[test]
fn s5_a_mode_change_with_a_content_change_is_an_item() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2);
    std::fs::set_permissions(d.join("src/a.py"), std::fs::Permissions::from_mode(0o755)).unwrap();
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base takes g only");
    let v = fx.delta();
    assert_eq!(by_why(&v, "mode_changed"), 1, "{}", summ(&v));
}

/// B-2: a whitespace-only line inside a string is text: kept verbatim in the raw hash, so A1 reads `text_differs`.
#[test]
fn s5_a_whitespace_line_in_a_string_is_text() {
    let m = |blank: &str, n: u32| format!("pub struct S;\n\nimpl S {{\n    pub fn m(&self) -> String {{\n        let s = \"alpha\n{blank}\n        beta\";\n        format!(\"{{}} {{}} {{}} {{}}\", s, 1, 2, {n})\n    }}\n}}\n");
    let fx = Fx::new(&[("src/a.rs", &m("", 3))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.rs", &m("  ", 4));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.rs", &m("", 4));
    commit(&d, "base");
    let v = fx.delta();
    let row = row_of(&v, "changed", "src/a.rs::m");
    assert_eq!(row["why"], "text_differs", "{row}");
}

/// B-2: the common indentation is an exact byte prefix: tabs are not spaces.
#[test]
fn s5_tabs_are_not_spaces() {
    let m = |ind: &str, n: u32| format!("pub struct S;\n\nimpl S {{\n{ind}pub fn m(&self) -> String {{\n{ind}{ind}let s = \"alpha\n{ind}{ind}beta\";\n{ind}{ind}format!(\"{{}} {{}} {{}} {{}}\", s, 1, 2, {n})\n{ind}}}\n}}\n");
    let fx = Fx::new(&[("src/a.rs", &m("\t", 3))]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.rs", &m("    ", 4));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.rs", &m("\t", 4));
    commit(&d, "base");
    let v = fx.delta();
    assert_eq!(row_of(&v, "changed", "src/a.rs::m")["why"], "text_differs", "{}", summ(&v));
}

/// B-3: a CR is text: a CRLF body is not base's LF body.
#[test]
fn s5_a_cr_is_text() {
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &g2.replace('\n', "\r\n"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &g2);
    commit(&d, "base");
    let v = fx.delta();
    assert_eq!(row_of(&v, "changed", "src/a.py::g")["why"], "text_differs", "{}", summ(&v));
}

/// B-4: two non-UTF-8 paths are two entries: the branch's change to one of them is an item.
#[cfg(unix)]
#[test]
fn s5_two_non_utf8_paths_are_two_entries() {
    use std::os::unix::ffi::OsStrExt;
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    let p1 = d.join(std::ffi::OsStr::from_bytes(b"notes\xfe.txt"));
    let p2 = d.join(std::ffi::OsStr::from_bytes(b"notes\xff.txt"));
    std::fs::write(&p1, "one\n").unwrap();
    std::fs::write(&p2, "two\n").unwrap();
    commit(&d, "two non-utf8 paths");
    fx.feat();
    std::fs::write(&p1, "one CHANGED by the lane\n").unwrap();
    commit(&d, "lane");
    fx.main();
    let v = fx.delta();
    let res = branch(&v)["residue"].to_string();
    assert!(res.contains("notes\\\\376.txt"), "the changed path is not an item: {}", summ(&v));
}

/// B-5: base keeps g in a file its grammar cannot parse cleanly (g lands in an ERROR node): the file counts as unread on base,
/// so the deletion is not R4's yes.
#[test]
fn s5_a_base_file_with_parse_errors_counts_as_unread() {
    let fx = Fx::new(&[("src/a.py", &format!("{FOO}\n{G5}")), ("src/k.py", KEEP)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", FOO);
    commit(&d, "lane deletes g");
    fx.main();
    write(&d, "src/a.py", &format!("{FOO}\nx = (1,\n{G5}"));
    commit(&d, "base keeps g, breaks the parse above it");
    let v = fx.delta();
    assert_eq!(row_of(&v, "removed", "src/a.py::g")["why"], "base_unread", "{}", summ(&v));
}

/// The `touches` half of the span check: a hunk of one blank line inside a module string is in no judged row: an item.
#[test]
fn s5_a_blank_line_hunk_outside_every_row_is_an_item() {
    let mb = format!("DOC = \"\"\"\nline one\nline two\n\"\"\"\n\n{G5}");
    let fx = Fx::new(&[("src/a.py", &mb)]);
    let d = fx.d();
    fx.feat();
    let g2 = G5.replace("x * 3", "x * 4");
    write(&d, "src/a.py", &format!("DOC = \"\"\"\nline one\n\nline two\n\"\"\"\n\n{g2}"));
    commit(&d, "lane");
    fx.main();
    write(&d, "src/a.py", &format!("DOC = \"\"\"\nline one\nline two\n\"\"\"\n\n{g2}"));
    commit(&d, "base takes g only");
    let v = fx.delta();
    assert_eq!(by_why(&v, "outside_symbols"), 1, "{}", summ(&v));
}

/// Minor: the clone's diff.interHunkContext / diff.context never fuse or widen the -U0 hunks.
#[test]
fn s5_the_clone_diff_context_settings_do_not_apply() {
    let mb = format!("X = 1\n{G5}");
    let fx = Fx::new(&[("src/a.py", &mb)]);
    let d = fx.d();
    git(&d, &["config", "diff.interHunkContext", "5"]);
    git(&d, &["config", "diff.context", "4"]);
    fx.feat();
    write(&d, "src/a.py", &format!("X = 2\n{}", G5.replace("x * 3", "x * 4")));
    commit(&d, "lane");
    fx.main();
    let v = fx.delta();
    // the X hunk is an item; g's hunk lies inside g: one item, not a fused hunk taking g's line along
    assert_eq!(by_why(&v, "outside_symbols"), 1, "{}", summ(&v));
    assert_eq!(branch(&v)["residue"][0]["hunk"], "@@ -1 +1 @@", "{}", summ(&v));
}

/// Minor: the whole-tree share is `share_identical` (how much of head base holds as it is), not a retirement signal: the
/// exact-share names are the branch summary's only.
#[test]
fn s5_the_whole_tree_share_is_share_identical() {
    let fx = Fx::new(&[("src/a.py", G5)]);
    let d = fx.d();
    fx.feat();
    write(&d, "src/a.py", &G5.replace("x * 3", "x * 4"));
    commit(&d, "lane");
    fx.main();
    let v = ok(fx.root(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh", "--whole-tree"]);
    let ws = &v["data"]["whole_tree"]["summary"];
    assert!(ws["share_identical"].is_number() && ws.get("share_on_base_exact").is_none() && ws.get("share_already_on_base").is_none(), "{ws}");
}
