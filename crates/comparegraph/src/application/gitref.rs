//! CG-BR1: a git ref as a read-only source tree. The ref's tree is read as RAW blobs (`git ls-tree -r -z` + `git cat-file --batch`)
//! and written by this code into a cache folder under the store (`.comparegraph/refs/<repo>/<commit>/`). The clone is never checked
//! out and its index and working tree are never touched; nothing the clone configures runs (no smudge or lfs filter, no eol
//! conversion, no export-ignore: `git archive` applied all three). A symlink is written as a regular file holding its target (as
//! git does with `core.symlinks=false`), never followed; a submodule entry is skipped. A path that breaks git's `verify_path` rules
//! (absolute, `..`, `.git` in any NTFS/HFS spelling, a stream colon, a trailing dot or space: `unsafe_component`) refuses the tree
//! (`unsafe_tree`); paths that fold to one name on a case-insensitive file system, paths holding a Windows reserved device name
//! (`aux.c`), and paths that are not UTF-8 are left out with a limitation. The folder is keyed by the commit, so it is immutable and shared by every
//! name for it; a lock file beside it makes two builds of one commit extract it once, and the tree is extracted into a temporary
//! sibling and renamed into place before it is marked complete. A reader (a build or a delta reading the tree) registers a
//! reader file under the lock, and neither `gc --refs` nor a re-extraction deletes a tree while a live reader holds it.
use super::{AppError, Result};
use crate::domain::Limitation;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn refused(code_msg: String, hint: &str, data: serde_json::Value) -> AppError {
    AppError::structured("not_found", code_msg, Some(hint.into()), Some(data))
}

/// The commit a ref names in this clone; an unknown ref (or one that is not a commit) is a structured refusal.
pub fn resolve(clone: &Path, git_ref: &str) -> Result<String> {
    let bad = |why: &str| {
        refused(
            format!("ref '{git_ref}' does not name a commit in {} ({why})", clone.display()),
            "list the refs with `git -C <clone> branch -a`, fetch the missing one, and pass it as --ref",
            serde_json::json!({ "ref": git_ref, "clone": clone.display().to_string() }),
        )
    };
    if git_ref.is_empty() || git_ref.starts_with('-') || git_ref.contains('\0') || git_ref.contains('\n') {
        return Err(bad("not a ref name"));
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{git_ref}^{{commit}}"))
        .output()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad("unknown ref"));
    }
    Ok(sha)
}

/// What a ref names now, as far as the clone can say.
#[derive(Debug, PartialEq)]
pub enum Probe {
    /// the ref names this commit
    Commit(String),
    /// the clone was read and the ref names nothing (deleted branch)
    Missing,
    /// the clone could not be read (folder missing or unmounted, not a git repository, git not runnable): nothing is known
    Unreadable(String),
}

/// Resolve a ref, telling "the ref is gone" (the clone answered) from "the clone could not be asked" (an I/O failure).
pub fn probe(clone: &Path, git_ref: &str) -> Probe {
    if !clone.is_dir() {
        return Probe::Unreadable(format!("{} is not a folder", clone.display()));
    }
    match Command::new("git").arg("-C").arg(clone).args(["rev-parse", "--git-dir"]).output() {
        Err(e) => return Probe::Unreadable(format!("cannot run git: {e}")),
        Ok(o) if !o.status.success() => return Probe::Unreadable(format!("{} is not a git repository", clone.display())),
        Ok(_) => {}
    }
    match resolve(clone, git_ref) {
        Ok(c) => Probe::Commit(c),
        Err(AppError::Io(_, e)) => Probe::Unreadable(format!("cannot run git: {e}")),
        Err(_) => Probe::Missing,
    }
}

/// Where the tree of `commit` of repo `repo` is kept.
pub fn cache_dir(store_root: &Path, repo: &str, commit: &str) -> PathBuf {
    let safe: String = repo.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    store_root.join(".comparegraph").join("refs").join(safe).join(commit)
}

/// HFS+ ignores these code points when it compares names, so `.g\u{200c}it` opens `.git` there (git's `is_hfs_dotgit` set).
fn hfs_ignorable(c: char) -> bool {
    matches!(c as u32, 0x200c..=0x200f | 0x202a..=0x202e | 0x206a..=0x206f | 0xfeff)
}

/// Why one tree path component is refused, by git's `verify_path` rules with core.protectNTFS and core.protectHFS on (applied on
/// every platform, so one tree gives one answer everywhere); None when it is safe.
/// - `dot_component`: empty, `.` or `..`
/// - `dotgit`: `.git` in any case
/// - `hfs_dotgit`: `.git` once HFS-ignorable code points are dropped
/// - `ntfs_dotgit`: `.git` followed by dots or spaces only (NTFS drops them: `.git.`, `.git `)
/// - `ntfs_8dot3_dotgit`: `git~1`, the 8.3 short name of `.git` (with trailing dots or spaces too)
/// - `ntfs_stream`: a `:` (an NTFS alternate data stream such as `.git::$INDEX_ALLOCATION`, or a drive letter)
/// - `trailing_dot_or_space`: Windows drops them, so the name would alias another path
pub fn unsafe_component(c: &str) -> Option<&'static str> {
    if c.is_empty() || c == "." || c == ".." {
        return Some("dot_component");
    }
    if c.eq_ignore_ascii_case(".git") {
        return Some("dotgit");
    }
    let hfs: String = c.chars().filter(|ch| !hfs_ignorable(*ch)).collect();
    if hfs.len() != c.len() && hfs.eq_ignore_ascii_case(".git") {
        return Some("hfs_dotgit");
    }
    if c.contains(':') {
        return Some("ntfs_stream");
    }
    let trimmed = c.trim_end_matches(['.', ' ']);
    if trimmed.eq_ignore_ascii_case(".git") {
        return Some("ntfs_dotgit");
    }
    if trimmed.eq_ignore_ascii_case("git~1") {
        return Some("ntfs_8dot3_dotgit");
    }
    if trimmed.len() != c.len() {
        return Some("trailing_dot_or_space");
    }
    None
}

/// A Windows reserved device name: `con`, `prn`, `aux`, `nul`, `com1`-`com9`, `lpt1`-`lpt9`, any case, with or without an
/// extension (`aux.c`, `NUL.txt`). Not an attack (unlike a `.git` alias): such a path is skipped on every platform, so one tree
/// extracts the same everywhere, and listed in `windows_reserved_skipped`.
pub fn reserved_component(c: &str) -> bool {
    let stem = c.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_lowercase();
    let numbered = stem.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && matches!(stem.as_bytes()[3], b'1'..=b'9');
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") || numbered
}

/// A tree path as a relative path inside the cache folder, or the rule it breaks: `empty`, `absolute`, `backslash`, `nul`, or a
/// component rule of `unsafe_component`.
pub fn safe_rel_path(p: &str) -> std::result::Result<PathBuf, &'static str> {
    if p.is_empty() {
        return Err("empty");
    }
    if p.starts_with('/') {
        return Err("absolute");
    }
    if p.contains('\\') {
        return Err("backslash");
    }
    if p.contains('\0') {
        return Err("nul");
    }
    let mut out = PathBuf::new();
    for c in p.split('/') {
        if let Some(rule) = unsafe_component(c) {
            return Err(rule);
        }
        out.push(c);
    }
    Ok(out)
}

/// One blob entry of the tree: (mode, object id, size in bytes, relative path).
type Entry = (String, String, u64, PathBuf);

/// Why a tree is not extracted.
enum TreeError {
    /// a path breaks a `verify_path` rule: the whole tree is refused (`unsafe_tree`)
    Unsafe { path: String, rule: &'static str },
    Other(String),
}
impl From<String> for TreeError {
    fn from(s: String) -> Self {
        TreeError::Other(s)
    }
}

/// The blob entries of `commit`'s tree, and the limitations of what is left out: paths that are not UTF-8 (never renamed
/// lossily), and every path of a group that folds to one name on a case-insensitive file system (both are skipped, on every
/// platform). A path that breaks a `verify_path` rule refuses the tree.
fn list_tree(clone: &Path, commit: &str, max_file_bytes: u64) -> std::result::Result<(Vec<Entry>, Vec<Limitation>), TreeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["ls-tree", "-r", "-z", "-l", "--full-tree", commit])
        .output()
        .map_err(|e| format!("cannot run git ls-tree: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-tree {commit} failed: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    let mut v: Vec<(String, String, u64, String)> = vec![];
    let (mut non_utf8, mut non_utf8_first) = (0usize, None::<String>);
    let (mut submodules, mut oversize) = (0usize, 0usize);
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = rec.iter().position(|b| *b == b'\t').ok_or_else(|| format!("unreadable ls-tree line: {}", String::from_utf8_lossy(rec)))?;
        let meta = String::from_utf8_lossy(&rec[..tab]).into_owned();
        let mut m = meta.split_whitespace();
        let (mode, kind, oid, size) = (m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""));
        if kind == "commit" {
            submodules += 1; // a submodule: another repository's commit, no content here
            continue;
        }
        if kind != "blob" {
            continue;
        }
        let Ok(path) = std::str::from_utf8(&rec[tab + 1..]) else {
            non_utf8 += 1;
            non_utf8_first.get_or_insert_with(|| String::from_utf8_lossy(&rec[tab + 1..]).into_owned());
            continue;
        };
        if let Err(rule) = safe_rel_path(path) {
            return Err(TreeError::Unsafe { path: path.to_string(), rule });
        }
        let size: u64 = size.parse().unwrap_or(0);
        if size > max_file_bytes {
            oversize += 1; // the walker would skip it: it is never read from git at all
            continue;
        }
        v.push((mode.to_string(), oid.to_string(), size, path.to_string()));
    }
    // case-folded collisions: a file path or any of its folders spelled two ways that fold to one name
    let mut spellings: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = Default::default();
    for (_, _, _, p) in &v {
        let mut prefix = String::new();
        for (i, c) in p.split('/').enumerate() {
            if i > 0 {
                prefix.push('/');
            }
            prefix.push_str(c);
            spellings.entry(prefix.to_lowercase()).or_default().insert(prefix.clone());
        }
    }
    let colliding: std::collections::BTreeSet<&String> = spellings.iter().filter(|(_, s)| s.len() > 1).map(|(k, _)| k).collect();
    let mut skipped: Vec<String> = vec![];
    let mut reserved: Vec<String> = vec![];
    let mut kept = vec![];
    for (mode, oid, size, p) in v {
        let folded = p.to_lowercase();
        let hit = colliding.iter().any(|k| folded == **k || folded.starts_with(&format!("{k}/")));
        if p.split('/').any(reserved_component) {
            reserved.push(p); // never written, on every platform
        } else if hit {
            skipped.push(p);
        } else {
            let rel = safe_rel_path(&p).expect("checked above");
            kept.push((mode, oid, size, rel));
        }
    }
    let mut lims = vec![];
    if !skipped.is_empty() {
        let shown: Vec<&String> = skipped.iter().take(10).collect();
        lims.push(Limitation::new(
            "case_collision_skipped",
            format!(
                "{} path(s) fold to the same name on a case-insensitive file system and were all left out (on every platform): {}{}",
                skipped.len(),
                shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                if skipped.len() > shown.len() { ", ..." } else { "" }
            ),
        ));
    }
    if oversize > 0 {
        lims.push(Limitation::new("skipped_oversize", format!("{oversize} file(s) over max_file_bytes ({max_file_bytes}) not extracted from the ref's tree")));
    }
    if submodules > 0 {
        lims.push(Limitation::new("submodule_skipped", format!("{submodules} submodule(s) skipped: a submodule is another repository's commit and is not read")));
    }
    if !reserved.is_empty() {
        let shown: Vec<&str> = reserved.iter().take(10).map(String::as_str).collect();
        lims.push(Limitation::new(
            "windows_reserved_skipped",
            format!(
                "{} path(s) hold a Windows reserved device name (con, prn, aux, nul, com1-9, lpt1-9) and were left out (on every platform): {}{}",
                reserved.len(),
                shown.join(", "),
                if reserved.len() > shown.len() { ", ..." } else { "" }
            ),
        ));
    }
    if non_utf8 > 0 {
        lims.push(Limitation::new(
            "non_utf8_path",
            format!("{non_utf8} path(s) are not UTF-8 and were left out, never renamed (first: {:?})", non_utf8_first.unwrap_or_default()),
        ));
    }
    Ok((kept, lims))
}

/// Every entry of a commit's tree as git lists it (`git ls-tree -r -z --full-tree`), nothing skipped: what a run of the commit
/// could have read. Paths that are not UTF-8 are kept lossily (they are compared, never written).
#[derive(Debug, Default, Clone)]
pub struct TreeIndex {
    /// every blob: path -> (mode, object id)
    pub blobs: std::collections::BTreeMap<String, (String, String)>,
    /// every gitlink (a submodule entry): path -> the commit it pins
    pub gitlinks: std::collections::BTreeMap<String, String>,
    /// entries whose key another entry already had (two paths one key): the listing is `partial`
    pub collisions: usize,
}

/// A tree path as a key: the path itself when it is UTF-8, else git's C-quoted form (`"caf\351.py"`), so two non-UTF-8 paths
/// never fold into one key (round 5, B-4).
pub fn path_key(raw: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_string();
    }
    let mut out = String::from("\"");
    for b in raw {
        match *b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(*b as char),
            _ => out.push_str(&format!("\\{b:03o}")),
        }
    }
    out.push('"');
    out
}

/// The `TreeIndex` of `commit` in `clone`.
pub fn tree_index(clone: &Path, commit: &str) -> Result<TreeIndex> {
    tree_index_with(clone, commit, &[])
}

/// The `TreeIndex` of a commit or tree, git run with `env` (an object directory of its own, for a tree `git merge-tree` wrote).
pub fn tree_index_with(clone: &Path, commit: &str, env: &[(String, String)]) -> Result<TreeIndex> {
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .args(["ls-tree", "-r", "-z", "--full-tree", commit])
        .output()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    if !out.status.success() {
        return Err(AppError::structured(
            "io_error",
            format!("git ls-tree {commit} failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
            Some("the clone must be a git repository that holds this commit".into()),
            None,
        ));
    }
    let mut t = TreeIndex::default();
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(tab) = rec.iter().position(|b| *b == b'\t') else { continue };
        let meta = String::from_utf8_lossy(&rec[..tab]).into_owned();
        let mut m = meta.split_whitespace();
        let (mode, kind, oid) = (m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""));
        let path = path_key(&rec[tab + 1..]);
        if t.blobs.contains_key(&path) || t.gitlinks.contains_key(&path) {
            t.collisions += 1;
            continue;
        }
        match kind {
            "blob" => {
                t.blobs.insert(path, (mode.to_string(), oid.to_string()));
            }
            "commit" => {
                t.gitlinks.insert(path, oid.to_string());
            }
            _ => {}
        }
    }
    Ok(t)
}

/// What git's own merge of `head` into `base` gives (`git merge-tree --write-tree`, git >= 2.38): the merged tree's id, its
/// entries, and the paths in conflict. Read-only for the clone: the objects the merge writes go to a temporary object directory
/// under the store (the clone's own objects are only an alternate), removed afterwards.
pub struct Merged {
    pub tree: String,
    pub index: TreeIndex,
    pub conflicts: std::collections::BTreeSet<String>,
}

/// `git merge-tree` of `base` and `head`; Err (with why) when git cannot give it (too old, an error, a missing object).
pub fn merge_tree(clone: &Path, store_root: &Path, base: &str, head: &str) -> std::result::Result<Merged, String> {
    merge_tree_from(clone, store_root, base, head, None)
}

/// `merge_tree` with an explicit merge base (`--merge-base`, git >= 2.40): what merging `head` into `base` gives when their
/// common history is taken to end at `from`.
pub fn merge_tree_from(clone: &Path, store_root: &Path, base: &str, head: &str, from: Option<&str>) -> std::result::Result<Merged, String> {
    // a test hook (tests/delta_round7.rs), compiled into debug builds only: a git without `--merge-base`
    #[cfg(debug_assertions)]
    if from.is_some() && std::env::var_os("COMPAREGRAPH_TEST_NO_MERGE_BASE").is_some() {
        return Err("--merge-base refused (COMPAREGRAPH_TEST_NO_MERGE_BASE)".into());
    }
    let objects = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["rev-parse", "--path-format=absolute", "--git-path", "objects"])
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !objects.status.success() {
        return Err("git rev-parse --git-path objects failed".into());
    }
    let objects = String::from_utf8_lossy(&objects.stdout).trim().to_string();
    let store = store_root.join(".comparegraph");
    sweep_stale_tmp_merges(&store);
    let tmp = TmpDir(store.join(format!("tmp-merge-{}", new_token().replace(' ', "-"))));
    std::fs::create_dir_all(&tmp.0).map_err(|e| format!("{}: {e}", tmp.0.display()))?;
    // the clone's own settings never shape the merge: no global or system config, no attributes (so no merge driver, no
    // textconv, no eol renormalisation), renames as `git merge` does them, a directory rename a conflict
    let env = vec![
        ("GIT_OBJECT_DIRECTORY".to_string(), tmp.0.display().to_string()),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES".to_string(), objects),
        ("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string()),
        ("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string()),
    ];
    let out = Command::new("git")
        .arg("-C")
        .arg(clone)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .args([
            &format!("--attr-source={EMPTY_TREE}"),
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "merge.renormalize=false",
            "-c",
            "merge.renames=true",
            "-c",
            "merge.directoryRenames=conflict",
            "merge-tree",
            "--write-tree",
            "--no-messages",
            "-z",
        ])
        .args(from.map(|f| format!("--merge-base={f}")))
        .args([base, head])
        .output()
        .map_err(|e| format!("cannot run git merge-tree: {e}"))?;
    // 0: clean, 1: conflicts (the tree is still written); anything else is an error (a git older than 2.42 knows no
    // --attr-source, one older than 2.38 no --write-tree: the backstop is then unavailable)
    if !matches!(out.status.code(), Some(0) | Some(1)) {
        return Err(format!("git merge-tree failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    // -z: `<tree>\0`, then one `<mode> <oid> <stage>\t<path>\0` per conflicted entry (paths raw, never quoted)
    let mut recs = out.stdout.split(|b| *b == 0);
    let tree = String::from_utf8_lossy(recs.next().unwrap_or(b"")).trim().to_string();
    if tree.len() < 40 || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("git merge-tree gave no tree id: {tree:?}"));
    }
    let mut conflicts = std::collections::BTreeSet::new();
    for r in recs {
        if r.is_empty() {
            break;
        }
        let Some(tab) = r.iter().position(|b| *b == b'\t') else { return Err(format!("unreadable conflict entry {:?}", String::from_utf8_lossy(r))) };
        conflicts.insert(path_key(&r[tab + 1..]));
    }
    if out.status.code() == Some(1) && conflicts.is_empty() {
        return Err("git merge-tree reported conflicts but listed none".into());
    }
    let index = tree_index_with(clone, &tree, &env).map_err(|e| e.to_string())?;
    Ok(Merged { tree, index, conflicts })
}

/// git's empty tree: an attribute source with no attributes in it.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A temporary folder removed when it goes out of scope (an early return or a panic included).
struct TmpDir(PathBuf);
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Remove the `tmp-merge-*` folders of runs that no longer live: the holder of the name's token is known dead (Linux: its
/// pid runs under another start time, or not at all), or the folder is older than an hour where that cannot be told.
fn sweep_stale_tmp_merges(store: &Path) {
    for e in std::fs::read_dir(store).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(token) = name.strip_prefix("tmp-merge-") else { continue };
        let mut f = token.splitn(2, '-');
        let pid: Option<u32> = f.next().and_then(|p| p.parse().ok());
        let start = f.next().and_then(|r| r.split_once('_')).map(|(s, _)| s.to_string());
        let dead = match (pid, cfg!(target_os = "linux")) {
            (Some(pid), true) => proc_start(pid).is_none_or(|now| start.as_deref().is_some_and(|s| !s.starts_with('n') && s != now)),
            _ => false,
        };
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > 3600);
        if dead || old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// The tree id of a commit.
pub fn tree_of(clone: &Path, commit: &str) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(clone).args(["rev-parse", &format!("{commit}^{{tree}}")]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Write every blob of `entries` under `into`, reading them raw through one `git cat-file --batch`.
fn write_blobs(clone: &Path, into: &Path, entries: &[Entry]) -> std::result::Result<(), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run git cat-file: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let oids: Vec<String> = entries.iter().map(|e| e.1.clone()).collect();
    // the requests are written from a thread so a full stdout pipe never blocks the writer
    let feeder = std::thread::spawn(move || -> std::io::Result<()> {
        for o in oids {
            writeln!(stdin, "{o}")?;
        }
        Ok(())
    });
    let mut rd = BufReader::new(child.stdout.take().expect("piped"));
    let res = (|| -> std::result::Result<(), String> {
        let mut header = String::new();
        for (mode, oid, _size, rel) in entries {
            header.clear();
            rd.read_line(&mut header).map_err(|e| format!("git cat-file: {e}"))?;
            let mut h = header.trim_end().split(' ');
            let (got, kind, size) = (h.next().unwrap_or(""), h.next().unwrap_or(""), h.next().unwrap_or(""));
            if got != oid || kind != "blob" {
                return Err(format!("git cat-file: expected blob {oid}, got {}", header.trim_end()));
            }
            let size: u64 = size.parse().map_err(|_| format!("git cat-file: bad size in {}", header.trim_end()))?;
            let dest = into.join(rel);
            if !dest.starts_with(into) {
                return Err(format!("{} would land outside the cache folder", rel.display()));
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            // streamed, never held whole in memory; mode 120000 (a symlink): the content is its target, written as a regular file
            let mut f = std::fs::File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            let n = std::io::copy(&mut (&mut rd).take(size), &mut f).map_err(|e| format!("{}: {e}", dest.display()))?;
            if n != size {
                return Err(format!("git cat-file: {oid} ended after {n} of {size} bytes"));
            }
            let mut nl = [0u8; 1]; // the trailing newline of each object
            rd.read_exact(&mut nl).map_err(|e| format!("git cat-file: {e}"))?;
            #[cfg(unix)]
            if mode == "100755" {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
            }
            #[cfg(not(unix))]
            let _ = mode;
        }
        Ok(())
    })();
    drop(rd);
    let fed = feeder.join().map_err(|_| "git cat-file feeder panicked".to_string())?;
    let status = child.wait().map_err(|e| format!("git cat-file: {e}"))?;
    res?;
    fed.map_err(|e| format!("git cat-file: {e}"))?;
    if !status.success() {
        return Err("git cat-file --batch failed".into());
    }
    Ok(())
}

/// A lock or reader file beside a tree folder, held while this value lives. It holds `<pid> <token>` (the token is unique per
/// process and per take, so two threads of one process never mistake each other's file), and a heartbeat thread refreshes its
/// mtime every `HEARTBEAT_SECS` while held, so a long extraction or read never looks stale. Such a file is taken over only when
/// its heartbeat stopped `STALE_LOCK_SECS` ago AND its holder is not known to be running (on Linux the pid is looked up in
/// `/proc`; elsewhere the heartbeat alone decides), by renaming it aside atomically and checking that the file moved is the stale
/// one (`take_over_stale`). A holder removes its file on drop only after reading its own token back from it.
pub struct TreeLock {
    path: PathBuf,
    token: String,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    beat: Option<std::thread::JoinHandle<()>>,
}
const HEARTBEAT_SECS: u64 = 2;
const STALE_LOCK_SECS: u64 = 120;
const LOCK_WAIT_SECS: u64 = 1800;
/// `<pid> <pid>-<n>`: what a lock or reader file holds, unique per process and per call
/// `<pid> <start>_<pid>-<n>`: `<start>` is the process's start time where the system tells it (Linux: field 22 of
/// `/proc/<pid>/stat`, in clock ticks since boot), else a nonce drawn once per process; a recycled pid then never reads as the
/// holder (M1). Windows gives no start time without a system call this crate does not make: there the heartbeat alone decides.
fn new_token() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static START: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let pid = std::process::id();
    let start = START.get_or_init(|| {
        proc_start(pid).unwrap_or_else(|| {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let mut h = Sha256::new();
            h.update(format!("{t} {pid} {:p}", &N).as_bytes());
            format!("n{}", &hex::encode(h.finalize())[..16])
        })
    });
    format!("{pid} {start}_{pid}-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}
/// A process's start time on Linux (field 22 of `/proc/<pid>/stat`, read after the `)` that ends its name), else None.
fn proc_start(pid: u32) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(19).map(String::from)
}
/// The token part of a lock file's text (`<pid> <token>`), or the whole text for an older file.
fn token_of(text: &str) -> &str {
    text.split_once(' ').map_or(text, |(_, t)| t).trim()
}
impl TreeLock {
    /// Create `path` (create-new) holding a fresh token: None when it exists.
    fn create(path: &Path) -> std::io::Result<Option<TreeLock>> {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                let token = new_token();
                let _ = write!(f, "{token}");
                Ok(Some(TreeLock::hold(path, token)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn hold(path: &Path, token: String) -> TreeLock {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (p, s, t) = (path.to_path_buf(), stop.clone(), token_of(&token).to_string());
        let beat = std::thread::spawn(move || {
            while !s.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::park_timeout(std::time::Duration::from_secs(HEARTBEAT_SECS));
                if s.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                // only our own file is kept fresh: a file someone else took over is theirs
                if std::fs::read_to_string(&p).is_ok_and(|x| token_of(&x) == t) {
                    if let Ok(f) = std::fs::File::options().write(true).open(&p) {
                        let _ = f.set_modified(std::time::SystemTime::now());
                    }
                }
            }
        });
        TreeLock { path: path.to_path_buf(), token, stop, beat: Some(beat) }
    }
    /// The file still holds this holder's token.
    fn still_ours(&self) -> bool {
        std::fs::read_to_string(&self.path).is_ok_and(|x| token_of(&x) == token_of(&self.token))
    }
}
impl Drop for TreeLock {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(b) = self.beat.take() {
            b.thread().unpark();
            let _ = b.join();
        }
        if self.still_ours() {
            let _ = std::fs::remove_file(&self.path); // just verified: ours
        }
    }
}
/// Whether the process that wrote `lock` still runs: Some on Linux (`/proc/<pid>`), None where that cannot be told.
fn holder_alive(lock: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(lock).ok()?;
    let mut f = text.split_whitespace();
    let pid: u32 = f.next()?.parse().ok()?;
    // the start time the holder recorded (`<start>_...`), when it is a start time and not a nonce
    let recorded = f.next().and_then(|t| t.split_once('_')).map(|(s, _)| s.to_string()).filter(|s| !s.starts_with('n'));
    if cfg!(target_os = "linux") {
        Some(match proc_start(pid) {
            None => false,
            Some(now) => recorded.is_none_or(|r| r == now), // a recycled pid has another start time
        })
    } else {
        None
    }
}
/// A lock file that a waiter must respect: it exists, and its heartbeat is fresh or its holder is known to run.
fn lock_is_live(lock: &Path) -> bool {
    let Ok(m) = std::fs::metadata(lock) else { return false };
    let stale = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|a| a.as_secs() > STALE_LOCK_SECS);
    !stale || holder_alive(lock) == Some(true)
}
/// Take over a stale lock or reader file: rename it aside to a name unique to this call (atomic: of two who try, one wins), then
/// check that the file moved is the one seen stale and is stale still; only then is it removed. A file that turns out live
/// (re-created between the check and the rename) is put back where it was. True when the stale file is gone.
fn take_over_stale(lock: &Path) -> bool {
    if lock_is_live(lock) {
        return false;
    }
    let seen = std::fs::read_to_string(lock).unwrap_or_default();
    let name = lock.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let aside = lock.with_file_name(format!("{name}.stale-{}", new_token().replace(' ', "-")));
    if std::fs::rename(lock, &aside).is_err() {
        return false; // gone already, or taken by another
    }
    let moved = std::fs::read_to_string(&aside).unwrap_or_default();
    if moved == seen && !lock_is_live(&aside) {
        let _ = std::fs::remove_file(&aside); // just verified: the stale file
        return true;
    }
    // a live file was moved: put it back (never over a file now at the path), and remove the aside name only once it is back
    if std::fs::hard_link(&aside, lock).is_ok() {
        let _ = std::fs::remove_file(&aside);
    }
    false
}
/// Take the lock, or None when the tree was completed by another holder while waiting.
fn lock_tree(lock: &Path, marker: &Path) -> Result<Option<TreeLock>> {
    let start = std::time::Instant::now();
    loop {
        match TreeLock::create(lock) {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {
                if marker.is_file() {
                    return Ok(None);
                }
                if take_over_stale(lock) {
                    continue; // a crashed holder's: no heartbeat, no running holder
                }
                if start.elapsed().as_secs() > LOCK_WAIT_SECS {
                    return Err(AppError::structured(
                        "store_busy",
                        format!("another build is extracting this tree ({} held)", lock.display()),
                        Some("retry when it has finished".into()),
                        None,
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(AppError::Io(lock.to_path_buf(), e)),
        }
    }
}
/// Take the lock now, or None when a live holder has it (extracting, or creating it this instant); a stale one is taken over.
fn try_lock(lock: &Path) -> Result<Option<TreeLock>> {
    loop {
        match TreeLock::create(lock) {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) if take_over_stale(lock) => continue,
            Ok(None) => return Ok(None),
            Err(e) => return Err(AppError::Io(lock.to_path_buf(), e)),
        }
    }
}

/// The reader files of `commit`'s tree (`<commit>.read-<token>`, beside the folder).
fn reader_files(dir: &Path, commit: &str) -> Vec<PathBuf> {
    let prefix = format!("{commit}.read-");
    let Some(parent) = dir.parent() else { return vec![] };
    let mut v: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix) && !e.file_name().to_string_lossy().contains(".stale-"))
        .map(|e| e.path())
        .collect();
    v.sort();
    v
}
/// Whether a live reader holds `commit`'s tree; stale reader files (a crashed reader's) are taken over and removed on the way.
fn readers_live(dir: &Path, commit: &str) -> bool {
    reader_files(dir, commit).iter().any(|r| !take_over_stale(r) && r.exists())
}

/// A reader of an extracted tree: while it lives, `gc --refs` and a re-extraction leave the tree in place.
pub struct TreeReader(#[allow(dead_code)] TreeLock);
impl std::fmt::Debug for TreeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TreeReader({})", self.0.path.display())
    }
}
/// Register a reader of `commit`'s tree; the caller holds the tree's lock (so no remover is between its check and its delete).
fn register_reader(dir: &Path, commit: &str) -> Result<TreeReader> {
    loop {
        let p = dir.with_file_name(format!("{commit}.read-{}", new_token().replace(' ', "-")));
        match TreeLock::create(&p) {
            Ok(Some(l)) => return Ok(TreeReader(l)),
            Ok(None) => continue,
            Err(e) => return Err(AppError::Io(p, e)),
        }
    }
}

/// An extracted tree: its folder, and the limitations of what extraction left out (kept in the `.complete` marker, so a tree
/// extracted once reports them to every build that reads it). While the value lives, its reader keeps the folder in place.
#[derive(Debug)]
pub struct Tree {
    pub dir: PathBuf,
    pub limitations: Vec<Limitation>,
    pub reader: Option<TreeReader>,
}

/// The tree behind a `.complete` marker, when it serves a read with `max_file_bytes`: the folder exists and the marker's own
/// limit is at least that (an older marker, the commit alone, extracted everything). The marker is JSON
/// `{commit, max_file_bytes, limitations}`.
fn read_marker(marker: &Path, dir: &Path, max_file_bytes: u64) -> Option<Tree> {
    let text = std::fs::read_to_string(marker).ok()?;
    if !dir.is_dir() {
        return None; // the folder went (by hand, or a gc cut short): extracted again, never read as an empty tree
    }
    let v = serde_json::from_str::<serde_json::Value>(&text).ok();
    let limit = v.as_ref().map_or(Some(u64::MAX), |v| v["max_file_bytes"].as_u64());
    if limit.is_none_or(|l| l < max_file_bytes) {
        return None; // extracted with a smaller size limit than this read needs
    }
    let limitations = v.and_then(|v| serde_json::from_value::<Vec<Limitation>>(v["limitations"].clone()).ok()).unwrap_or_default();
    Some(Tree { dir: dir.to_path_buf(), limitations, reader: None })
}

/// Extract the tree of `commit` into its cache folder (a no-op when a complete one serves `max_file_bytes`) and return it with a
/// reader registered. Every step runs under the tree's lock: a reader registers only on a complete tree, and a re-extraction
/// (a larger size limit) waits, holding the lock, until the readers of the old tree are gone. Blobs over `max_file_bytes` (the
/// walker's size limit) are never read from git; a later read with a larger limit extracts the tree again.
pub fn materialise(clone: &Path, store_root: &Path, repo: &str, commit: &str, max_file_bytes: u64) -> Result<Tree> {
    let dir = cache_dir(store_root, repo, commit);
    let marker = dir.with_file_name(format!("{commit}.complete")); // beside the tree, so the walker never sees it
    let parent = dir.parent().expect("cache dir has a parent").to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| AppError::Io(parent.clone(), e))?;
    let _lock = lock_tree(&dir.with_file_name(format!("{commit}.lock")), Path::new(""))?.expect("a wait without a marker ends with the lock");
    if let Some(mut t) = read_marker(&marker, &dir, max_file_bytes) {
        t.reader = Some(register_reader(&dir, commit)?);
        return Ok(t);
    }
    // the tree is (re-)extracted: wait for the readers of the one in place, the lock held so no new reader comes
    let start = std::time::Instant::now();
    while readers_live(&dir, commit) {
        if start.elapsed().as_secs() > LOCK_WAIT_SECS {
            return Err(AppError::structured(
                "store_busy",
                format!("{} is being read; it is extracted again once its readers are done", dir.display()),
                Some("retry when the other build or delta has finished".into()),
                None,
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let io = |p: &Path, e: std::io::Error| AppError::Io(p.to_path_buf(), e);
    // the marker goes first: a tree without it is never read as complete
    if marker.exists() {
        std::fs::remove_file(&marker).map_err(|e| io(&marker, e))?;
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| io(&dir, e))?; // a half-written folder of a crashed run: ours, rebuilt whole
    }
    // unique per process and per call: two threads of one process never share it
    let tmp = dir.with_file_name(format!("{commit}.tmp-{}", new_token().replace(' ', "-")));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|e| io(&tmp, e))?;
    }
    std::fs::create_dir_all(&tmp).map_err(|e| io(&tmp, e))?;
    let fail = |msg: String| {
        let _ = std::fs::remove_dir_all(&tmp);
        AppError::structured("io_error", msg, Some("the clone must be a git repository that holds this commit".into()), None)
    };
    let (entries, limitations) = match list_tree(clone, commit, max_file_bytes) {
        Ok(x) => x,
        Err(TreeError::Other(m)) => return Err(fail(m)),
        Err(TreeError::Unsafe { path, rule }) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(AppError::structured(
                "unsafe_tree",
                format!("the tree of {commit} holds a path git refuses to check out ({rule}): {path:?}; nothing of it was written"),
                Some("this commit cannot be read safely; build another ref, or remove the path in a new commit".into()),
                Some(serde_json::json!({ "commit": commit, "path": path, "rule": rule })),
            ));
        }
    };
    write_blobs(clone, &tmp, &entries).map_err(fail)?;
    std::fs::rename(&tmp, &dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&tmp);
        io(&dir, e)
    })?;
    let body = serde_json::json!({ "commit": commit, "max_file_bytes": max_file_bytes, "limitations": limitations }).to_string();
    std::fs::write(&marker, body).map_err(|e| io(&marker, e))?;
    let reader = Some(register_reader(&dir, commit)?);
    Ok(Tree { dir, limitations, reader })
}

/// Whether `commit`'s tree is busy: a live holder has its lock (an extraction is running) or a live reader holds it. `gc --refs`
/// then leaves the tree and its runs alone.
pub fn tree_locked(store_root: &Path, repo: &str, commit: &str) -> bool {
    let dir = cache_dir(store_root, repo, commit);
    lock_is_live(&dir.with_file_name(format!("{commit}.lock"))) || readers_live(&dir, commit)
}

/// Remove `commit`'s tree under its lock: the `.complete` marker first (so no reader takes a half-removed tree as complete),
/// then the folder. Ok(false) when a live holder has the lock (extracting, or creating it this instant) or a live reader holds
/// the tree (nothing is removed); Ok(true) when the folder was there and went. A stale lock is taken over by `take_over_stale`.
pub fn remove_tree(store_root: &Path, repo: &str, commit: &str) -> Result<bool> {
    let dir = cache_dir(store_root, repo, commit);
    let Some(held) = try_lock(&dir.with_file_name(format!("{commit}.lock")))? else { return Ok(false) };
    if readers_live(&dir, commit) {
        drop(held);
        return Ok(false);
    }
    let marker = dir.with_file_name(format!("{commit}.complete"));
    let io = |p: &Path, e: std::io::Error| AppError::Io(p.to_path_buf(), e);
    if marker.exists() {
        std::fs::remove_file(&marker).map_err(|e| io(&marker, e))?;
    }
    let had = dir.is_dir();
    if had {
        std::fs::remove_dir_all(&dir).map_err(|e| io(&dir, e))?;
    }
    drop(held);
    Ok(had)
}

#[cfg(test)]
mod tests {
    use super::safe_rel_path;

    #[test]
    fn only_relative_paths_inside_the_folder_are_accepted() {
        assert_eq!(safe_rel_path("src/a.py").unwrap(), std::path::PathBuf::from("src").join("a.py"));
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a/./b", "a//b", "a\\b", ".git/config", "x/.GIT/hooks/pre-commit", "a\0b", "C:/x", "a/b:stream"] {
            assert!(safe_rel_path(bad).is_err(), "{bad:?} accepted");
        }
    }

    /// git's verify_path rules (protectNTFS + protectHFS), one rule per spelling of `.git`, on every platform.
    #[test]
    fn every_spelling_of_dotgit_is_refused_with_its_rule() {
        for (p, rule) in [
            (".git./config", "ntfs_dotgit"),
            (".GIT/x", "dotgit"),
            ("git~1/x", "ntfs_8dot3_dotgit"),
            ("GIT~1. /x", "ntfs_8dot3_dotgit"),
            ("a/.git /x", "ntfs_dotgit"),
            ("a/.git::$INDEX_ALLOCATION/x", "ntfs_stream"),
            ("a/.g\u{200c}it/x", "hfs_dotgit"),
            ("a/\u{feff}.GIT/x", "hfs_dotgit"),
            ("a/name./x", "trailing_dot_or_space"),
            ("a/name /x", "trailing_dot_or_space"),
        ] {
            assert_eq!(safe_rel_path(p), Err(rule), "{p:?}");
        }
        // reserved device names are not refused: they are skipped (`reserved_component`, round 3 amended)
        for p in ["aux.c", "NUL.txt", "Com1", "LPT9.tar.gz", "con", "prn "] {
            assert!(super::reserved_component(p), "{p:?}");
        }
        for p in ["console.c", "com10.c", "com0", "auxiliary.py", "nul_check.py"] {
            assert!(!super::reserved_component(p), "{p:?}");
        }
        for ok in [".gitignore", ".github/workflows/ci.yml", "git~2/x", "a/.git-blame-ignore-revs", "x.y/z", "...a", "src/console.c", "com10.c", "com0", "auxiliary.py", "nul_check.py"] {
            assert!(safe_rel_path(ok).is_ok(), "{ok:?} refused");
        }
    }
}

#[cfg(test)]
mod race_tests {
    use super::materialise;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }
    fn files_under(d: &Path) -> usize {
        let mut n = 0;
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            n += if p.is_dir() { files_under(&p) } else { 1 };
        }
        n
    }

    /// MINOR: two builds of one commit at once must both get a whole tree; neither may leave a partial tree marked complete.
    #[test]
    fn parallel_extractions_of_one_commit_each_return_a_whole_tree() {
        let clone = tempfile::tempdir().unwrap();
        git(clone.path(), &["init", "-q", "-b", "main"]);
        for i in 0..400 {
            let f = clone.path().join(format!("src/d{}/m{i:03}.py", i % 7));
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, format!("def f{i}(x):\n    return x + {i}\n")).unwrap();
        }
        git(clone.path(), &["add", "-A"]);
        git(clone.path(), &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "c"]);
        let commit = git(clone.path(), &["rev-parse", "HEAD"]);
        for round in 0..3 {
            let store = tempfile::tempdir().unwrap();
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let (c, s, k) = (clone.path().to_path_buf(), store.path().to_path_buf(), commit.clone());
                    std::thread::spawn(move || materialise(&c, &s, "r", &k, u64::MAX).map(|t| files_under(&t.dir)))
                })
                .collect();
            for h in handles {
                match h.join().unwrap() {
                    Ok(n) => assert_eq!(n, 400, "round {round}: a caller got a tree of {n} files"),
                    Err(e) => panic!("round {round}: a parallel extraction failed: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod lock_tests {
    use super::lock_tree;

    /// MINOR: a lock whose mtime is old but whose holder still runs is never taken over (the holder may be in a long extraction).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_lock_whose_holder_lives_is_not_taken_over() {
        let d = tempfile::tempdir().unwrap();
        let (lock, marker) = (d.path().join("c.lock"), d.path().join("c.complete"));
        std::fs::write(&lock, std::process::id().to_string()).unwrap(); // a live pid: this test
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        let (l2, m2) = (lock.clone(), marker.clone());
        let h = std::thread::spawn(move || lock_tree(&l2, &m2).map(|l| l.is_some()).map_err(|e| e.to_string()));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let taken = h.is_finished();
        std::fs::write(&marker, "{}").unwrap(); // the holder finishes: the waiter returns without the lock
        let got = h.join().unwrap();
        assert!(!taken, "a lock whose holder lives was taken over: {got:?}");
        assert_eq!(got, Ok(false));
    }

    /// MINOR: a held lock is kept fresh by a heartbeat, so a long extraction never looks stale to a waiter.
    #[test]
    fn a_held_lock_is_kept_fresh_by_a_heartbeat() {
        let d = tempfile::tempdir().unwrap();
        let (lock, marker) = (d.path().join("c.lock"), d.path().join("c.complete"));
        let held = lock_tree(&lock, &marker).unwrap().expect("free lock taken");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        // the heartbeat beats every 2 s; a loaded machine (the whole suite at once) may delay a beat, so wait up to 20 s for one
        let age = |l: &std::path::Path| std::fs::metadata(l).unwrap().modified().unwrap().elapsed().unwrap_or_default();
        let start = std::time::Instant::now();
        while age(&lock).as_secs() > 60 && start.elapsed().as_secs() < 20 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(age(&lock).as_secs() < 60, "the held lock was not refreshed ({}s old)", age(&lock).as_secs());
        drop(held);
        assert!(!lock.exists());
    }
}

#[cfg(test)]
mod lock_round3_tests {
    use super::{lock_tree, materialise, remove_tree};
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }
    /// a clone with one commit (a small file and a 2,000-byte one); its sha
    fn clone_one(d: &Path) -> String {
        git(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.py"), "def f(x):\n    return x + 1\n").unwrap();
        std::fs::write(d.join("src/big.py"), "X = 1\n".repeat(400)).unwrap();
        git(d, &["add", "-A"]);
        git(d, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "c"]);
        git(d, &["rev-parse", "HEAD"])
    }

    /// M1 (round 4): a lock whose pid runs but under another start time (a recycled pid) is a dead holder's: taken over once
    /// its heartbeat is stale.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_recycled_pid_does_not_hold_a_lock() {
        let d = tempfile::tempdir().unwrap();
        let (lock, marker) = (d.path().join("c.lock"), d.path().join("c.complete"));
        std::fs::write(&lock, format!("{} 1_{}-0", std::process::id(), std::process::id())).unwrap(); // our pid, another start
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
        let (l2, m2) = (lock.clone(), marker.clone());
        let h = std::thread::spawn(move || lock_tree(&l2, &m2).map(|l| l.is_some()).map_err(|e| e.to_string()));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let taken = h.is_finished();
        std::fs::write(&marker, "{}").unwrap();
        assert!(taken, "a lock held by a recycled pid was never taken over");
        assert_eq!(h.join().unwrap(), Ok(true));
    }

    /// MINOR (round 3): a holder never removes a lock file that is no longer its own (taken over and re-created by another).
    #[test]
    fn a_lock_is_removed_only_by_its_own_holder() {
        let d = tempfile::tempdir().unwrap();
        let (lock, marker) = (d.path().join("c.lock"), d.path().join("c.complete"));
        let held = lock_tree(&lock, &marker).unwrap().expect("free lock taken");
        std::fs::write(&lock, "1 someone-else").unwrap(); // another holder's lock now sits at the path
        drop(held);
        assert!(lock.exists(), "a holder removed a lock file it did not hold");
    }

    /// MINOR (round 3): `remove_tree` (gc) never deletes a tree a reader still holds.
    #[test]
    fn remove_tree_refuses_while_a_reader_holds_the_tree() {
        let (clone, store) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let c = clone_one(clone.path());
        let tree = materialise(clone.path(), store.path(), "r", &c, u64::MAX).unwrap();
        assert!(!remove_tree(store.path(), "r", &c).unwrap(), "gc removed a tree a reader holds");
        assert!(tree.dir.join("src/a.py").is_file());
        drop(tree);
        assert!(remove_tree(store.path(), "r", &c).unwrap(), "the tree stays after its reader went");
    }

    /// MINOR (round 3): a re-extraction (a larger size limit) waits for the readers of the tree it replaces.
    #[test]
    fn a_re_extraction_waits_for_the_readers_of_the_old_tree() {
        let (clone, store) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let c = clone_one(clone.path());
        let small = materialise(clone.path(), store.path(), "r", &c, 1000).unwrap();
        assert!(!small.dir.join("src/big.py").exists());
        let (cl, st, cc) = (clone.path().to_path_buf(), store.path().to_path_buf(), c.clone());
        let h = std::thread::spawn(move || materialise(&cl, &st, "r", &cc, u64::MAX).map(|t| t.dir.join("src/big.py").is_file()));
        std::thread::sleep(std::time::Duration::from_millis(1500));
        // the old tree is still the one in place (the new one would hold big.py): nothing was deleted or swapped under the reader
        let (swapped, done) = (small.dir.join("src/big.py").exists(), h.is_finished());
        assert!(!swapped && !done, "the tree was replaced under its reader (swapped {swapped}, re-extraction done {done})");
        assert!(small.dir.join("src/a.py").is_file());
        drop(small);
        assert_eq!(h.join().unwrap().map_err(|e| e.to_string()), Ok(true));
    }
}
