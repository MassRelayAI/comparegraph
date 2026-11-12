//! Shared fixture for the store-safety tests (lane CG-STORE): a small workspace built from scratch in a temp dir, so no
//! test ever opens a real store. `Ws::new()` = a root clone, two comparators (`a`, `b`), one family, two built runs,
//! claims with citations.
#![allow(dead_code)]
use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success();
    assert!(ok, "git {args:?}");
}
pub fn clone_with(dir: &Path, files: &[(&str, &str)], licence: &str) {
    for (p, c) in files {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(dir.join("LICENSE"), licence).unwrap();
    git(dir, &["init", "-q"]);
    git(dir, &["add", "."]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]);
}
pub fn commit_all(dir: &Path) {
    git(dir, &["add", "."]);
    git(dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "y"]);
}
pub fn cc(root: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root).env_remove("COMPAREGRAPH_WAIT").env_remove("COMPAREGRAPH_ROOT_MAP");
    c
}
pub fn out(a: assert_cmd::assert::Assert) -> Value {
    let o = a.get_output();
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}
/// Run a verb and parse the envelope, whatever the exit code.
pub fn run(root: &Path, args: &[&str]) -> Value {
    let o = cc(root).args(args).output().unwrap();
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{args:?}: {e}: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}
pub fn ok(root: &Path, args: &[&str]) -> Value {
    let v = run(root, args);
    assert_eq!(v["ok"], true, "{args:?}: {v}");
    v
}
pub fn err(root: &Path, args: &[&str], code: &str) -> Value {
    let v = run(root, args);
    assert_eq!(v["ok"], false, "{args:?} should fail: {v}");
    assert_eq!(v["error"]["code"], code, "{args:?}: {v}");
    v
}

pub const PACK: &str = "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n\
| routing.retry | routing | Retry with backoff | Failed calls are retried | a retry loop | basics | 2 |\n\
| memory.recall | memory | Recall | Past turns | a store read | extra | 1 |\n";
const MIT: &str = "MIT License\n\nCopyright (c) 2026 S\n";
const AGPL: &str = "GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3\n";

pub struct Ws {
    pub ws: tempfile::TempDir,
    pub base: tempfile::TempDir,
}
impl Ws {
    pub fn root(&self) -> &Path {
        self.ws.path()
    }
    pub fn clone_dir(&self, name: &str) -> PathBuf {
        self.base.path().join(name)
    }
    pub fn db(&self) -> PathBuf {
        self.ws.path().join(".comparegraph/comparegraph.db")
    }
    /// The bytes of the store file and its WAL (what "nothing was written" compares).
    pub fn store_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        let wal = PathBuf::from(format!("{}-wal", self.db().display()));
        (std::fs::read(self.db()).unwrap(), std::fs::read(wal).unwrap_or_default())
    }
    /// A root, comparators `a` (MIT, two claims) and `b` (AGPL, one claim), family `fam`, all built.
    pub fn new() -> Ws {
        let ws = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        let pack = base.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("rows.md"), PACK).unwrap();
        ok(ws.path(), &["family", "add", pack.to_str().unwrap()]);
        let mk = |n: &str, src: &str, lic: &str| {
            let d = base.path().join(n);
            std::fs::create_dir_all(&d).unwrap();
            clone_with(&d, &[("src/net/a.py", src)], lic);
            d
        };
        let root = mk("rootrepo", "def retry_loop():\n    return 0\n\ndef other():\n    return 1\n", MIT);
        let a = mk("a", "def retry_with_backoff():\n    return 1\n\ndef recall_turns():\n    return 2\n", MIT);
        let b = mk("b", "def retry_later():\n    return 3\n\ndef unrelated():\n    return 4\n", AGPL);
        ok(ws.path(), &["add", root.to_str().unwrap(), "--name", "rootrepo", "--as-root"]);
        ok(ws.path(), &["add", a.to_str().unwrap(), "--name", "a"]);
        ok(ws.path(), &["add", b.to_str().unwrap(), "--name", "b"]);
        for n in ["rootrepo", "a", "b"] {
            ok(ws.path(), &["build", n]);
        }
        ok(ws.path(), &["claim", "a", "routing.retry", "has", "--author", "t", "--at", "src/net/a.py:1-2", "--no-reach"]);
        ok(ws.path(), &["claim", "a", "memory.recall", "partial", "--author", "t", "--at", "src/net/a.py:4-5", "--no-reach"]);
        ok(ws.path(), &["claim", "b", "routing.retry", "has", "--author", "t", "--at", "src/net/a.py:1-2", "--no-reach"]);
        ok(ws.path(), &["claim", "rootrepo", "routing.retry", "has", "--author", "t", "--at", "src/net/a.py:1-2", "--no-reach"]);
        Ws { ws, base }
    }
    /// Raw connection to the fixture store (for tests that poison or inspect rows).
    pub fn conn(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).unwrap()
    }
    pub fn count(&self, sql: &str) -> i64 {
        self.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}
