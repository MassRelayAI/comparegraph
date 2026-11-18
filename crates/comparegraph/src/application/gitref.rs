//! CG-BR1: a git ref as a read-only source tree. The ref's tree is read with `git archive <commit> | tar -x` into a cache folder
//! under the store (`.comparegraph/refs/<repo>/<commit>/`); the clone is never checked out, its index and working tree never touched
//! (`git archive` and `rev-parse` only read). The folder is keyed by the commit, so it is immutable and shared by every name for it.
use super::{AppError, Result};
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

/// Where the tree of `commit` of repo `repo` is kept.
pub fn cache_dir(store_root: &Path, repo: &str, commit: &str) -> PathBuf {
    let safe: String = repo.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    store_root.join(".comparegraph").join("refs").join(safe).join(commit)
}

/// Extract the tree of `commit` into its cache folder (a no-op when a complete one exists) and return the folder.
pub fn materialise(clone: &Path, store_root: &Path, repo: &str, commit: &str) -> Result<PathBuf> {
    let dir = cache_dir(store_root, repo, commit);
    let marker = dir.with_file_name(format!("{commit}.complete")); // beside the tree, so the walker never sees it
    if marker.is_file() {
        return Ok(dir);
    }
    let io = |e: std::io::Error| AppError::Io(dir.clone(), e);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(io)?; // a half-written folder of an earlier failed run: ours, rebuilt whole
    }
    std::fs::create_dir_all(&dir).map_err(io)?;
    let fail = |msg: String| {
        let _ = std::fs::remove_dir_all(&dir);
        AppError::structured(
            "io_error",
            msg,
            Some("the clone must be a git repository that holds this commit".into()),
            None,
        )
    };
    let mut git = Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["archive", "--format=tar", commit])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| fail(format!("cannot run git archive: {e}")))?;
    let tar = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&dir)
        .stdin(Stdio::from(git.stdout.take().expect("piped")))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| fail(format!("cannot run tar: {e}")))?;
    let g = git.wait_with_output().map_err(|e| fail(format!("git archive: {e}")))?;
    if !g.status.success() {
        return Err(fail(format!("git archive {commit} failed: {}", String::from_utf8_lossy(&g.stderr).trim())));
    }
    if !tar.status.success() {
        return Err(fail(format!("tar could not extract {commit}: {}", String::from_utf8_lossy(&tar.stderr).trim())));
    }
    std::fs::write(&marker, commit).map_err(io)?;
    Ok(dir)
}
