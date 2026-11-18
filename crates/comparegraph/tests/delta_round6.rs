//! Round-6 review probes of the delta model (feat/plan-14 at 5e976b4): two false 1.0s (a conflict on a quoted path; work
//! base merged and then reverted), the pins the review asked for, and its tsx/jsx syntax probes (printed, not asserted).
#![allow(dead_code)]
mod common;
use common::{git, ok};
use serde_json::Value;
use std::path::{Path, PathBuf};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const G5: &str = "def g(items):\n    total = 0\n    for x in items:\n        total = total + x * 3 - len(items)\n        log(total, x, items)\n    return total\n";
const H5: &str = "def h(items):\n    acc = 1\n    for y in items:\n        acc = acc * y + 7 - len(items)\n        trace(acc, y, items)\n    return acc\n";
const K5: &str = "def k(items):\n    s = 2\n    for z in items:\n        s = s - z * 5 + len(items)\n        note(s, z, items)\n    return s\n";

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
    fn delta(&self) -> Value {
        ok(self.ws.path(), &["delta", "r", "--base", "main", "--head", "feat", "--refresh"])
    }
}
fn s(v: &Value) -> String {
    format!("SHARE={} unknown={} residue={} rows={}", v["data"]["summary"]["share_on_base_exact"], v["data"]["summary"]["unknown"], v["data"]["branch"]["residue"], v["data"]["branch"])
}

/// The branch deletes a file; base removed its function but kept the file with new content. modify/delete conflict:
/// git keeps base's version in the merged tree, so the merged tree IS base's tree. Only the conflict list guards it.
fn modify_delete(path: &str) -> Value {
    let fx = Fx::new(&[(path, G5), ("src/keep.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    git(&d, &["rm", "-q", "--", path]);
    commit(&d, "del");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, path, H5);
    commit(&d, "mod");
    let v = fx.delta();
    eprintln!("{path}: {}", s(&v));
    v
}
#[test]
fn r6_modify_delete_plain_path() {
    let v = modify_delete("src/ab.py");
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
}
#[test]
fn r6_modify_delete_quoted_path() {
    let v = modify_delete("src/a\"b.py");
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
}
#[test]
fn r6_modify_delete_tab_path() {
    let v = modify_delete("src/a\tb.py");
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
}

/// Base merged the branch, then reverted its work; the branch went on, and base took the later work.
/// head still holds g, base does not: retiring the branch loses g.
#[test]
fn r6_merged_then_reverted_on_base() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "revert", "--no-edit", "-m", "1", "HEAD"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    eprintln!("merged-then-reverted: {}", s(&v));
    let has_g = std::fs::read_to_string(d.join("src/a.py")).unwrap().contains("def g");
    assert!(!has_g);
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
}

/// The clone's .git is not written by delta (merge-tree included).
#[cfg(unix)]
#[test]
fn r6_delta_writes_nothing_in_the_clone_git_dir() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}\n{H5}"));
    commit(&d, "base");
    // build everything first (builds may legitimately touch nothing in .git either)
    fx.delta();
    let snap = |p: &Path| -> Vec<String> {
        let o = std::process::Command::new("bash").arg("-c").arg(format!("cd '{}' && find .git -printf '%p %s %T@\\n' | sort", p.display())).output().unwrap();
        String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect()
    };
    let before = snap(&d);
    let v = fx.delta();
    let after = snap(&d);
    let diff: Vec<_> = after.iter().filter(|l| !before.contains(l)).chain(before.iter().filter(|l| !after.contains(l))).collect();
    eprintln!("gitdir diff: {diff:?} share {}", v["data"]["summary"]["share_on_base_exact"]);
    assert!(diff.is_empty(), "{diff:?}");
    let leftovers: Vec<_> = std::fs::read_dir(fx.ws.path().join(".comparegraph")).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("tmp-merge")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn r6_tsx_and_jsx_syntax_probes() {
    use comparegraph::providers::reference::{has_parse_error, parse_file};
    let cases: &[(&str, &str)] = &[
        ("tsx", "export function A() {\n  return <><b>x</b></>;\n}\n"),
        ("tsx", "export const id = <T,>(x: T): T => x;\n"),
        ("tsx", "export const id2 = <T extends unknown>(x: T): T => x;\n"),
        ("tsx", "const cfg = { a: 1 } satisfies Record<string, number>;\nexport function f() { return cfg; }\n"),
        ("tsx", "export function B<T>(p: { v: T }) {\n  return <List<T> items={[p.v]} render={(i) => <i>{String(i)}</i>} />;\n}\n"),
        ("tsx", "export function C() {\n  return <svg:rect a=\"1\" />;\n}\n"),
        ("tsx", "export function D(p: any) {\n  return <div {...p} data-x={p!.x as string}>{p?.c ?? null}</div>;\n}\n"),
        ("tsx", "enum E { A }\nexport abstract class K<T> implements I { private x!: T; constructor(public y: number) {} abstract m(): void; }\n"),
        ("typescript", "enum E { A }
export abstract class K<T> implements I { private x!: T; constructor(public y: number) {} abstract m(): void; }
"),
        ("tsx", "export class K2 { m() { return <b/>; } }
"),
        ("tsx", "export function F() { const v = <any>window; return v; }\n"),
        ("jsx", "export function A() {\n  return <><b>x</b></>;\n}\n"),
        ("jsx", "export function C() {\n  return <svg:rect a=\"1\" />;\n}\n"),
        ("jsx", "export function D(p) {\n  return <div {...p} data-x={p.x}>{p?.c ?? null}</div>;\n}\n"),
        ("jsx", "@decorator\nexport class G extends React.Component { state = {}; #priv = 1; static s = 2; render() { return <div/>; } }\n"),
        ("jsx", "export const H = () => <Ctx.Provider value={1}><a.b.c /></Ctx.Provider>;\n"),
        ("javascript", "export function A() {\n  return <><b>x</b></>;\n}\n"),
        ("typescript", "const cfg = { a: 1 } satisfies Record<string, number>;\nexport function f() { return (<any>cfg); }\n"),
    ];
    for (lang, src) in cases {
        let e = has_parse_error(src, lang);
        let syms: Vec<String> = parse_file("x", src, lang).map(|r| r.symbols.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
        eprintln!("PROBE {lang:10} err={e:?} syms={syms:?} :: {}", src.lines().next().unwrap());
    }
}

// ================================================================ round 6: ported additions and pins

/// B1: a modify/delete conflict on a non-UTF-8 path (git C-quotes it without `-z`).
#[cfg(unix)]
#[test]
fn r6_modify_delete_non_utf8_path() {
    use std::os::unix::ffi::OsStrExt;
    let fx = Fx::new(&[("src/keep.py", K5)]);
    let d = fx.d();
    let p = d.join(std::ffi::OsStr::from_bytes(b"src/caf\xe9.py"));
    std::fs::write(&p, G5).unwrap();
    commit(&d, "add a latin-1 named file");
    git(&d, &["checkout", "-q", "-b", "feat"]);
    std::fs::remove_file(&p).unwrap();
    commit(&d, "del");
    git(&d, &["checkout", "-q", "main"]);
    std::fs::write(&p, H5).unwrap();
    commit(&d, "mod");
    let v = fx.delta();
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
    // round 7: the conflict is read from `merge-tree -z` and keyed raw (C-quoted): the conflicts limitation names it so
    let lim = v["evidence"]["limitations"].as_array().unwrap().iter().find(|l| l["code"] == "merge_conflicts").cloned().unwrap_or_default();
    assert_eq!(lim["data"]["paths"], serde_json::json!(["\"src/caf\\351.py\""]), "{}", v["evidence"]["limitations"]);
}

/// B1 pin: the plain modify/delete row is turned unknown by the per-row conflict check itself (a conflict path is always a
/// path the branch touched, so its row or item is there for the per-row check; the share-level `merge_conflict` item backs a
/// per-row key that misses, as the quoted paths did, and is reached through them: see the quoted, tab and non-UTF-8 probes).
/// (First version also asserted a `merge_conflict` item here; the per-row check already makes the share 0.0, so it cannot be.)
#[test]
fn r6_modify_delete_row_check() {
    let v = modify_delete("src/ab.py");
    let row = v["data"]["branch"]["removed"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/ab.py::g").unwrap().clone();
    assert_eq!((row["why"].as_str(), row["model_via"].as_str()), (Some("merge_changes_file"), Some("absent_on_base")), "{row}");
}

/// B2: a criss-cross history (each side merged the other) where base then reverted the branch's g.
#[test]
fn r6_criss_cross_with_a_revert_on_base() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    let gc = |args: &[&str]| {
        let mut a = vec!["-c", "user.email=t@t", "-c", "user.name=t"];
        a.extend_from_slice(args);
        git(&d, &a);
    };
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "f1: add g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/m.py", H5);
    commit(&d, "m1: add h");
    let f1 = git_out(&d, &["rev-parse", "feat"]);
    let m1 = git_out(&d, &["rev-parse", "main"]);
    gc(&["merge", "-q", "--no-ff", "-m", "M1", &f1]);
    git(&d, &["checkout", "-q", "feat"]);
    gc(&["merge", "-q", "--no-ff", "-m", "F1", &m1]);
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", K5);
    commit(&d, "base reverts g");
    let v = fx.delta();
    assert!(!std::fs::read_to_string(d.join("src/a.py")).unwrap().contains("def g"));
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
    // the ruling of round 6: one item per delta, the paths in a limitation
    assert_eq!(b2(&v), Some((1, vec!["src/a.py".to_string()])), "{}", s(&v));
}

fn paths_len(lim: &Value) -> usize {
    lim["data"]["paths"].as_array().map_or(0, Vec::len)
}

/// The `merged_into_base_before` reading: (the number of such items, the paths its limitation lists), None without one.
fn b2(v: &Value) -> Option<(i64, Vec<String>)> {
    let n = v["data"]["summary"]["unknown"]["by_why"]["merged_into_base_before"].as_i64()?;
    let lim = v["evidence"]["limitations"].as_array()?.iter().find(|l| l["code"] == "merged_into_base_before_paths")?.clone();
    let paths = lim["data"]["paths"].as_array()?.iter().map(|p| p.as_str().unwrap_or("").to_string()).collect();
    let count = lim["data"]["count"].as_u64().unwrap_or(0) as usize;
    assert!(count >= paths_len(&lim) && paths_len(&lim) <= 20, "{lim}");
    Some((n, paths))
}

/// B2 pin: the merged-then-reverted shape names the reason.
#[test]
fn r6_merged_then_reverted_is_an_item() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "revert", "--no-edit", "-m", "1", "HEAD"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", H5);
    commit(&d, "add h");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", H5);
    commit(&d, "take h");
    let v = fx.delta();
    assert_eq!(b2(&v), Some((1, vec!["src/a.py".to_string()])), "{}", s(&v));
}

/// MAJOR pin, the 1.0 tree rule: the branch changes f in A; base renames A to B keeping the old f and holds the new f in D.
/// The row's path A is gone on both sides; only the merged tree (B gets the change) is not base's.
#[test]
fn r6_the_tree_rule_catches_a_change_base_renamed_away() {
    let f_old = G5;
    let f_new = G5.replace("x * 3", "x * 4");
    let fx = Fx::new(&[("src/A.py", f_old)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/A.py", &f_new);
    commit(&d, "lane changes g");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["mv", "src/A.py", "src/B.py"]);
    write(&d, "src/D.py", &f_new.replace("def g(", "def g2("));
    commit(&d, "base renames A to B and holds the new body in D");
    let v = fx.delta();
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"]["merge_changes_tree"].as_i64(), Some(1), "{}", s(&v));
}

/// MAJOR: the clone's merge driver and attributes never shape the merge: `*.py merge=ours` with an `ours` driver would keep
/// base's a.py and hide the branch's change.
#[test]
fn r6_a_merge_driver_in_the_clone_does_not_apply() {
    let f_new = G5.replace("x * 3", "x * 4");
    let fx = Fx::new(&[("src/a.py", &format!("{G5}\n{K5}")), (".gitattributes", "*.py merge=ours\n")]);
    let d = fx.d();
    git(&d, &["config", "merge.ours.driver", "true"]);
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{f_new}\n{K5}"));
    commit(&d, "lane changes g");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/a.py", &format!("{G5}\n{}", K5.replace("z * 5", "z * 6")));
    write(&d, "src/d.py", &f_new.replace("def g(", "def g2("));
    commit(&d, "base changes k, holds the new g in d.py");
    let v = fx.delta();
    assert_ne!(v["data"]["summary"]["share_on_base_exact"].as_f64(), Some(1.0), "{}", s(&v));
}

/// MINOR: the provider version carries the grammar overrides.
#[test]
fn r6_the_provider_version_names_the_tsx_override() {
    use comparegraph::providers::GraphProvider;
    let v = comparegraph::providers::reference::ReferenceProvider.version();
    assert!(v.ends_with("+tsx"), "{v}");
}

/// MINOR: the inheritance pass reads .tsx with the TSX grammar too: a JSX class's `extends` is found.
#[test]
fn r6_a_jsx_class_extends_edge_is_found_in_tsx() {
    let fx = Fx::new(&[
        ("src/Base.tsx", "export class Base {\n  hello() { return 1; }\n}\n"),
        ("src/Card.tsx", "import { Base } from './Base';\nexport class Card extends Base {\n  render() { return <div className=\"c\">{this.hello()}</div>; }\n}\n"),
        // JSX ahead of a class, in the shapes the TypeScript grammar cannot read (the class after it must still be found)
        ("src/C1.tsx", "import { Base } from './Base';\nexport const top = <div className=\"top\"><Base /></div>;\nexport class C1 extends Base {}\n"),
        ("src/C2.tsx", "import { Base } from './Base';\nexport function W() { return <><span>{1}</span></>; }\nexport class C2 extends Base {}\n"),
        ("src/C3.tsx", "import { Base } from './Base';\nexport const id = <T,>(x: T): T => x;\nexport const el = <div {...{ a: 1 }}>{id(2)}</div>;\nexport class C3 extends Base {}\n"),
    ]);
    ok(fx.ws.path(), &["build", "r"]);
    let db = rusqlite::Connection::open(fx.ws.path().join(".comparegraph/comparegraph.db")).unwrap();
    for sub in ["src/Card.tsx::Card", "src/C1.tsx::C1", "src/C2.tsx::C2", "src/C3.tsx::C3"] {
        let n: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM edge e JOIN node a ON a.id = e.source_node_id JOIN node b ON b.id = e.target_node_id WHERE e.kind = 'extends' AND a.qualified_name = ?1 AND b.qualified_name = 'src/Base.tsx::Base'",
                [sub],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "no extends edge {sub} -> Base");
    }
}

/// MINOR: no stale `tmp-merge-*` folder survives in the store (a crashed run's is swept by the next delta).
#[test]
fn r6_a_stale_tmp_merge_folder_is_swept() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    fx.delta();
    let stale = fx.ws.path().join(".comparegraph").join("tmp-merge-1-1_1-0");
    std::fs::create_dir_all(stale.join("ab")).unwrap();
    std::fs::write(stale.join("ab").join("cdef"), "x").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options().write(true).open(stale.join("ab").join("cdef")).unwrap().set_modified(old).unwrap();
    std::fs::File::open(&stale).unwrap().set_modified(old).ok();
    fx.delta();
    assert!(!stale.exists(), "a stale tmp-merge folder survived");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

// ================================================================ round 6, the tighter B2: a merge from F

/// feat adds g; base merges feat (--no-ff) and keeps g, then changes a.py elsewhere; feat goes on with h, which base takes.
fn merged_then(base_after: impl Fn(&Path)) -> Value {
    let fx = Fx::new(&[("src/a.py", K5), ("src/c.py", "X = 1\n")]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    write(&d, "src/c.py", &format!("X = 1\n\n{H5}"));
    commit(&d, "add g and h");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    base_after(&d);
    commit(&d, "base goes on");
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/b.py", "def later(q):\n    r = q + 7\n    return r * 3\n");
    commit(&d, "add later");
    git(&d, &["checkout", "-q", "main"]);
    write(&d, "src/b.py", "def later(q):\n    r = q + 7\n    return r * 3\n");
    commit(&d, "take later");
    fx.delta()
}

/// Base merged the branch and kept its work (it changed a.py elsewhere since): no `merged_into_base_before` for that work.
#[test]
fn r6_merged_and_kept_is_no_item() {
    let v = merged_then(|d| write(d, "src/a.py", &format!("# base note\n{K5}\n{G5}")));
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"].get("merged_into_base_before"), None, "{}", s(&v));
}

/// Base merged the branch, kept a.py's g and reverted c.py's h: an item for c.py only.
#[test]
fn r6_merged_kept_part_and_reverted_part() {
    let v = merged_then(|d| {
        write(d, "src/a.py", &format!("# base note\n{K5}\n{G5}"));
        write(d, "src/c.py", "X = 1\n");
    });
    // the ruling of round 6: one item per delta, the paths in a limitation (was one item per path)
    assert_eq!(b2(&v), Some((1, vec!["src/c.py".to_string()])), "{}", s(&v));
}

/// The branch's ordinary work after the merge base, on a path base merged before (and kept), is the model's and the regular
/// backstop's to judge: no `merged_into_base_before` item for it.
#[test]
fn r6_post_merge_base_work_on_a_path_base_merged_before_is_no_b2_item() {
    let fx = Fx::new(&[("src/a.py", K5)]);
    let d = fx.d();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}"));
    commit(&d, "add g");
    git(&d, &["checkout", "-q", "main"]);
    git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "merge", "-q", "--no-ff", "-m", "merge feat", "feat"]);
    git(&d, &["checkout", "-q", "feat"]);
    write(&d, "src/a.py", &format!("{K5}\n{G5}\n{H5}"));
    commit(&d, "add h, after the merge base");
    git(&d, &["checkout", "-q", "main"]);
    let v = fx.delta();
    assert_eq!(v["data"]["summary"]["unknown"]["by_why"].get("merged_into_base_before"), None, "{}", s(&v));
    let h = v["data"]["branch"]["added"].as_array().unwrap().iter().find(|r| r["qualified_name"] == "src/a.py::h").unwrap().clone();
    assert_eq!(h["on_base"], "no", "{h}");
}
