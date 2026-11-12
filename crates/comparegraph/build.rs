//! Hashes the vendored parser sources so the parse-cache key changes whenever the parser does, not only on a version bump.
use sha2::{Digest, Sha256};
use std::path::Path;

fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let root = Path::new("src/providers/reference/vendor");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    let mut h = Sha256::new();
    for f in &files {
        h.update(f.to_string_lossy().as_bytes());
        h.update(std::fs::read(f).unwrap_or_default());
    }
    let hex: String = h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=COMPAREGRAPH_PARSER_HASH={hex}");
    git_sha();
}

/// Embeds the short commit as COMPAREGRAPH_GIT_SHA so `--version` tells two builds of one crate version apart; "unknown" without git (a source archive, a registry build).
fn git_sha() {
    let git = |args: &[&str]| std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty());
    for what in ["HEAD", "logs/HEAD"] {
        if let Some(p) = git(&["rev-parse", "--git-path", what]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let sha = git(&["rev-parse", "--short=10", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=COMPAREGRAPH_GIT_SHA={sha}");
}
