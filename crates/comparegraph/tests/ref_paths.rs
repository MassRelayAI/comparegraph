//! CG-BR1 path safety (review round 2): a ref's tree is written by the engine, so every tree path passes git's `verify_path`
//! rules (core.protectNTFS and core.protectHFS, on every platform) before anything is written. The hostile trees are made with
//! git plumbing (`mktree`, `commit-tree`) inside temp dirs only: nothing is checked out, no such path is ever created on disk.
mod common;
use common::{err, git, ok};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";

fn git_in(dir: &Path, args: &[&str], stdin: &[u8]) -> String {
    let mut c = Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(stdin).unwrap();
    let o = c.wait_with_output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

struct Fx {
    ws: tempfile::TempDir,
    base: tempfile::TempDir,
}
impl Fx {
    fn new() -> Fx {
        let (ws, base) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let d = base.path().join("r");
        std::fs::create_dir_all(d.join("src")).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("LICENSE"), MIT).unwrap();
        std::fs::write(d.join("src/a.py"), "def foo(x):\n    y = x * 2\n    return y + 1\n").unwrap();
        git(&d, &["add", "-A"]);
        git(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "c1"]);
        ok(ws.path(), &["add", d.to_str().unwrap(), "--name", "r"]);
        Fx { ws, base }
    }
    fn clone(&self) -> PathBuf {
        self.base.path().join("r")
    }
    fn root(&self) -> &Path {
        self.ws.path()
    }
    /// A branch `name` whose tree is main's plus the given (raw byte) paths, each a blob `x\n`. Built with plumbing: the paths
    /// exist only as git objects.
    fn hostile(&self, name: &str, paths: &[&[u8]]) -> String {
        let d = self.clone();
        let blob = git_in(&d, &["hash-object", "-w", "--stdin"], b"x\n");
        // a nested map of components -> subtree, built bottom-up into tree objects
        #[derive(Default)]
        struct Node(std::collections::BTreeMap<Vec<u8>, Node>, bool);
        let mut top = Node::default();
        for p in paths {
            let comps: Vec<&[u8]> = p.split(|b| *b == b'/').collect();
            let mut n = &mut top;
            for (i, c) in comps.iter().enumerate() {
                n = n.0.entry(c.to_vec()).or_default();
                if i == comps.len() - 1 {
                    n.1 = true;
                }
            }
        }
        fn write_tree(d: &Path, n: &Node, blob: &str, extra: &[u8]) -> String {
            let mut input: Vec<u8> = extra.to_vec();
            for (name, child) in &n.0 {
                if child.1 && child.0.is_empty() {
                    input.extend_from_slice(format!("100644 blob {blob}\t").as_bytes());
                } else {
                    let t = write_tree(d, child, blob, b"");
                    input.extend_from_slice(format!("040000 tree {t}\t").as_bytes());
                }
                input.extend_from_slice(name);
                input.push(0);
            }
            git_in(d, &["mktree", "-z"], &input)
        }
        let main_entries = Command::new("git").arg("-C").arg(&d).args(["ls-tree", "-z", "main"]).output().unwrap().stdout;
        let tree = write_tree(&d, &top, &blob, &main_entries);
        let commit = git_in(&d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit-tree", &tree, "-p", "main", "-m", name], b"");
        git(&d, &["update-ref", &format!("refs/heads/{name}"), &commit]);
        commit
    }
    fn refs_dir(&self) -> PathBuf {
        self.root().join(".comparegraph/refs/r")
    }
}

/// Every file under `d` (symlinks not followed), relative to it; the clone's own `.git` is left out.
fn files_under(d: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        let r = rel.join(e.file_name());
        if e.file_name() == ".git" && rel.as_os_str().is_empty() {
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            files_under(&p, &r, out);
        } else {
            out.push(r);
        }
    }
}

/// Nothing of a refused tree is left: no tree folder, no temporary sibling, no completion marker, and no file named after the
/// hostile leaf anywhere in the store's root or the clone's parent (outside the clone's own `.git`).
fn assert_nothing_written(fx: &Fx, commit: &str, leaf: &str) {
    let refs = fx.refs_dir();
    let siblings: Vec<String> = std::fs::read_dir(&refs).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(commit)).collect();
    assert!(siblings.is_empty(), "a refused tree left {siblings:?} in {}", refs.display());
    for top in [fx.root().to_path_buf(), fx.base.path().to_path_buf()] {
        let mut all = vec![];
        files_under(&top, Path::new(""), &mut all);
        let clone_git = Path::new("r").join(".git");
        let hits: Vec<&PathBuf> = all.iter().filter(|p| !p.starts_with(&clone_git) && p.file_name().is_some_and(|n| n == leaf)).collect();
        assert!(hits.is_empty(), "a file named {leaf} landed under {}: {hits:?}", top.display());
    }
}

#[test]
fn a_dotgit_alias_in_any_spelling_refuses_the_tree_and_writes_nothing() {
    let fx = Fx::new();
    for (i, (path, rule)) in [
        (".git./config", "ntfs_dotgit"),
        (".GIT/x", "dotgit"),
        ("git~1/x", "ntfs_8dot3_dotgit"),
        ("a/.git /x", "ntfs_dotgit"),
        ("a/.g\u{200c}it/x", "hfs_dotgit"),
        ("a/b:stream", "ntfs_stream"),
        ("a/ends-with-dot./x", "trailing_dot_or_space"),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("evil{i}");
        let commit = fx.hostile(&name, &[path.as_bytes()]);
        let v = err(fx.root(), &["build", "r", "--ref", &name], "unsafe_tree");
        assert_eq!(v["error"]["data"]["path"], path, "{v}");
        assert_eq!(v["error"]["data"]["rule"], rule, "{path}: {v}");
        let leaf = path.rsplit('/').next().unwrap();
        assert_nothing_written(&fx, &commit, leaf);
    }
    // a safe ref of the same clone still builds
    ok(fx.root(), &["build", "r", "--ref", "main"]);
}

fn limits(v: &Value) -> String {
    v["evidence"]["limitations"].to_string()
}

/// Two paths that fold to one on a case-insensitive file system (Windows, macOS) would land on one file: both are skipped, with
/// a limitation naming them, on every platform (the same tree gives the same graph everywhere).
#[test]
fn case_folded_duplicate_paths_are_both_skipped_with_a_limitation() {
    let fx = Fx::new();
    let commit = fx.hostile("dup", &[b"A.txt", b"a.txt", b"Dir/x.py", b"dir/y.py"]);
    let v = ok(fx.root(), &["build", "r", "--ref", "dup"]);
    let l = limits(&v);
    assert!(l.contains("case_collision_skipped") && l.contains("A.txt") && l.contains("a.txt"), "{l}");
    let tree = fx.refs_dir().join(&commit);
    for p in ["A.txt", "a.txt", "Dir/x.py", "dir/y.py"] {
        assert!(!tree.join(p).exists(), "{p} was written");
    }
    let mut all = vec![];
    files_under(&tree, Path::new(""), &mut all);
    let names: Vec<String> = all.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    assert_eq!(names.iter().filter(|n| n.eq_ignore_ascii_case("a.txt") || n.to_lowercase().starts_with("dir/")).count(), 0, "{names:?}");
    assert!(names.contains(&"src/a.py".to_string()), "the rest of the tree is extracted: {names:?}");
}

/// A path that is not UTF-8 is skipped with a limitation, never renamed lossily (no U+FFFD file appears).
#[test]
fn a_non_utf8_path_is_skipped_with_a_limitation_not_renamed() {
    let fx = Fx::new();
    let commit = fx.hostile("latin1", &[b"src/caf\xe9.py"]);
    let v = ok(fx.root(), &["build", "r", "--ref", "latin1"]);
    assert!(limits(&v).contains("non_utf8_path"), "{}", limits(&v));
    let mut all = vec![];
    files_under(&fx.refs_dir().join(&commit), Path::new(""), &mut all);
    assert!(all.iter().all(|p| !p.to_string_lossy().contains('\u{fffd}') && !p.to_string_lossy().contains("caf")), "{all:?}");
}

/// Review round 3 (amended): a Windows reserved device name (any case, with or without an extension) is not an attack: the path
/// is skipped on every platform, never written, and listed in a `windows_reserved_skipped` limitation; the build goes on.
#[test]
fn windows_reserved_device_names_are_skipped_and_listed_everywhere() {
    let fx = Fx::new();
    let reserved = ["src/aux.c", "NUL.txt", "a/Com1/x", "lpt9.h", "con", "docs/PRN.md"];
    let fine = ["src/console.c", "src/com10.c", "src/com0", "src/auxiliary.py", "src/nul_check.py"];
    let all: Vec<&[u8]> = reserved.iter().chain(fine.iter()).map(|p| p.as_bytes()).collect();
    let commit = fx.hostile("dev", &all);
    let v = ok(fx.root(), &["build", "r", "--ref", "dev"]);
    let lim = v["evidence"]["limitations"].as_array().unwrap().iter().find(|l| l["code"] == "windows_reserved_skipped").unwrap_or_else(|| panic!("no windows_reserved_skipped: {}", limits(&v))).clone();
    let detail = lim["detail"].as_str().unwrap();
    assert!(detail.starts_with("6 "), "{detail}");
    for p in reserved {
        assert!(detail.contains(p), "{p} not listed: {detail}");
    }
    let tree = fx.refs_dir().join(&commit);
    let mut got = vec![];
    files_under(&tree, Path::new(""), &mut got);
    let names: Vec<String> = got.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    for p in reserved {
        assert!(!names.contains(&p.to_string()), "{p} was written: {names:?}");
    }
    for p in fine {
        assert!(names.contains(&p.to_string()), "{p} was not extracted: {names:?}");
    }
}
