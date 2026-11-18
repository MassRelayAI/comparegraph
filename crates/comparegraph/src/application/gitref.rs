//! CG-BR1: a git ref as a read-only source tree. The ref's tree is read as RAW blobs (`git ls-tree -r -z` + `git cat-file --batch`)
//! and written by this code into a cache folder under the store (`.comparegraph/refs/<repo>/<commit>/`). The clone is never checked
//! out and its index and working tree are never touched; nothing the clone configures runs (no smudge or lfs filter, no eol
//! conversion, no export-ignore: `git archive` applied all three). A symlink is written as a regular file holding its target (as
//! git does with `core.symlinks=false`), never followed; a submodule entry is skipped. A path that breaks git's `verify_path` rules
//! (absolute, `..`, `.git` in any NTFS/HFS spelling, a stream colon, a trailing dot or space: `unsafe_component`) refuses the tree
//! (`unsafe_tree`); paths that fold to one name on a case-insensitive file system, and paths that are not UTF-8, are left out with a
//! limitation. The folder is keyed by the commit, so it is immutable and shared by every
//! name for it; a lock file beside it makes two builds of one commit extract it once, and the tree is extracted into a temporary
//! sibling and renamed into place before it is marked complete.
use super::{AppError, Result};
use crate::domain::Limitation;
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
fn list_tree(clone: &Path, commit: &str) -> std::result::Result<(Vec<Entry>, Vec<Limitation>), TreeError> {
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
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = rec.iter().position(|b| *b == b'\t').ok_or_else(|| format!("unreadable ls-tree line: {}", String::from_utf8_lossy(rec)))?;
        let meta = String::from_utf8_lossy(&rec[..tab]).into_owned();
        let mut m = meta.split_whitespace();
        let (mode, kind, oid, size) = (m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""));
        if kind != "blob" {
            continue; // a submodule (commit) entry has no content here
        }
        let Ok(path) = std::str::from_utf8(&rec[tab + 1..]) else {
            non_utf8 += 1;
            non_utf8_first.get_or_insert_with(|| String::from_utf8_lossy(&rec[tab + 1..]).into_owned());
            continue;
        };
        if let Err(rule) = safe_rel_path(path) {
            return Err(TreeError::Unsafe { path: path.to_string(), rule });
        }
        v.push((mode.to_string(), oid.to_string(), size.parse().unwrap_or(0), path.to_string()));
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
    let mut kept = vec![];
    for (mode, oid, size, p) in v {
        let folded = p.to_lowercase();
        let hit = colliding.iter().any(|k| folded == **k || folded.starts_with(&format!("{k}/")));
        if hit {
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
    if non_utf8 > 0 {
        lims.push(Limitation::new(
            "non_utf8_path",
            format!("{non_utf8} path(s) are not UTF-8 and were left out, never renamed (first: {:?})", non_utf8_first.unwrap_or_default()),
        ));
    }
    Ok((kept, lims))
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
            let size: usize = size.parse().map_err(|_| format!("git cat-file: bad size in {}", header.trim_end()))?;
            let mut buf = vec![0u8; size + 1]; // the content and its trailing newline
            rd.read_exact(&mut buf).map_err(|e| format!("git cat-file: {e}"))?;
            buf.pop();
            let dest = into.join(rel);
            if !dest.starts_with(into) {
                return Err(format!("{} would land outside the cache folder", rel.display()));
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            // mode 120000 (a symlink): `buf` is its target, written as the content of a regular file
            std::fs::write(&dest, &buf).map_err(|e| format!("{}: {e}", dest.display()))?;
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

/// The lock beside a tree folder: held by one extractor at a time, removed on drop. A lock older than `STALE_LOCK_SECS` is a
/// crashed holder's and is taken over.
struct TreeLock(PathBuf);
const STALE_LOCK_SECS: u64 = 600;
const LOCK_WAIT_SECS: u64 = 1800;
impl Drop for TreeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
/// Take the lock, or None when the tree was completed by another holder while waiting.
fn lock_tree(lock: &Path, marker: &Path) -> Result<Option<TreeLock>> {
    let start = std::time::Instant::now();
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                return Ok(Some(TreeLock(lock.to_path_buf())));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if marker.is_file() {
                    return Ok(None);
                }
                let age = std::fs::metadata(lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
                if age.is_some_and(|a| a.as_secs() > STALE_LOCK_SECS) {
                    let _ = std::fs::remove_file(lock);
                    continue;
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

/// An extracted tree: its folder, and the limitations of what extraction left out (kept in the `.complete` marker, so a tree
/// extracted once reports them to every build that reads it).
#[derive(Debug)]
pub struct Tree {
    pub dir: PathBuf,
    pub limitations: Vec<Limitation>,
}

/// The tree behind a `.complete` marker: JSON `{commit, limitations}`, or (an older marker) the commit alone.
fn read_marker(marker: &Path, dir: PathBuf) -> Option<Tree> {
    let text = std::fs::read_to_string(marker).ok()?;
    let limitations = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| serde_json::from_value::<Vec<Limitation>>(v["limitations"].clone()).ok())
        .unwrap_or_default();
    Some(Tree { dir, limitations })
}

/// Extract the tree of `commit` into its cache folder (a no-op when a complete one exists) and return it.
pub fn materialise(clone: &Path, store_root: &Path, repo: &str, commit: &str) -> Result<Tree> {
    let dir = cache_dir(store_root, repo, commit);
    let marker = dir.with_file_name(format!("{commit}.complete")); // beside the tree, so the walker never sees it
    if let Some(t) = read_marker(&marker, dir.clone()) {
        return Ok(t);
    }
    let parent = dir.parent().expect("cache dir has a parent").to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| AppError::Io(parent.clone(), e))?;
    let done = |marker: &Path, dir: &Path| read_marker(marker, dir.to_path_buf()).ok_or_else(|| AppError::Io(marker.to_path_buf(), std::io::Error::other("unreadable tree marker")));
    let Some(_lock) = lock_tree(&dir.with_file_name(format!("{commit}.lock")), &marker)? else {
        return done(&marker, &dir);
    };
    if marker.is_file() {
        return done(&marker, &dir); // completed by the holder before us
    }
    let io = |p: &Path, e: std::io::Error| AppError::Io(p.to_path_buf(), e);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| io(&dir, e))?; // a half-written folder of a crashed run: ours, rebuilt whole
    }
    let tmp = dir.with_file_name(format!("{commit}.tmp-{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).map_err(|e| io(&tmp, e))?;
    }
    std::fs::create_dir_all(&tmp).map_err(|e| io(&tmp, e))?;
    let fail = |msg: String| {
        let _ = std::fs::remove_dir_all(&tmp);
        AppError::structured("io_error", msg, Some("the clone must be a git repository that holds this commit".into()), None)
    };
    let (entries, limitations) = match list_tree(clone, commit) {
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
    let body = serde_json::json!({ "commit": commit, "limitations": limitations }).to_string();
    std::fs::write(&marker, body).map_err(|e| io(&marker, e))?;
    Ok(Tree { dir, limitations })
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
        for ok in [".gitignore", ".github/workflows/ci.yml", "git~2/x", "a/.git-blame-ignore-revs", "x.y/z", "...a"] {
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
                    std::thread::spawn(move || materialise(&c, &s, "r", &k).map(|t| files_under(&t.dir)))
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
