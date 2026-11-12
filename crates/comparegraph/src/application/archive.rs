//! `archive` / `restore` (lane CG-STORE B3): a SOFT archive. The repo keeps every row (node, edge, claim, location,
//! decision and relatedness are never updated or deleted); it gains a manifest file and a `meta` flag
//! (`archived:<repo>` = the manifest's sha256), leaves the default views and is never read by `check`, so its clone
//! folder may be moved away. No manifest on disk, no flag: the manifest is written (temp, fsync, rename) and re-read
//! before the flag is set. Both verbs are CLI only, dry-run by default, and logged.
use super::governance::{self, Action, ARCHIVED_PREFIX};
use super::{AppError, Result};
use crate::domain::actionlog::sha256_hex;
use crate::store::{now_iso, Store};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MANIFEST_SCHEMA: &str = "archive-manifest/1";

/// The names of the archived repos of an open store.
pub fn archived_names(store: &Store) -> Result<BTreeSet<String>> {
    Ok(store.meta_with_prefix(ARCHIVED_PREFIX)?.into_iter().map(|(k, _)| k[ARCHIVED_PREFIX.len()..].to_string()).collect())
}

/// `repo_archived` for a verb that would write to or score an archived repo.
pub fn refuse_if_archived(store: &Store, repo: &str) -> Result<()> {
    if store.meta_get(&format!("{ARCHIVED_PREFIX}{repo}"))?.is_some() {
        return Err(AppError::structured(
            "repo_archived",
            format!("{repo} is archived; it is read only through history (`claims --repo`, `decisions --repo`)"),
            Some(format!("`comparegraph restore {repo} --by <who> --apply` brings it back")),
            Some(json!({ "repo": repo })),
        ));
    }
    Ok(())
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write `bytes` to `path` through a temp file, fsync, rename, then re-read and compare the sha256.
fn write_verified(path: &Path, bytes: &[u8]) -> Result<String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| AppError::Io(dir.to_path_buf(), e))?;
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    let mut f = std::fs::File::create(&tmp).map_err(|e| AppError::Io(tmp.clone(), e))?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| AppError::Io(tmp.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| AppError::Io(path.to_path_buf(), e))?;
    let back = std::fs::read(path).map_err(|e| AppError::Io(path.to_path_buf(), e))?;
    let want = sha256_hex(bytes);
    if sha256_hex(&back) != want {
        return Err(AppError::check_failed_hint(format!("{} did not read back as written", path.display()), "nothing was flagged; retry"));
    }
    Ok(want)
}

pub struct ArchiveOpts<'a> {
    pub why: &'a str,
    pub by: &'a str,
    pub policy: Option<&'a Path>,
    pub apply: bool,
    pub force_with_reason: Option<&'a str>,
}

/// `archive <repo>`: refusals first, then (with `--apply`) the manifest, then the flag and the log entry together.
pub fn archive(root: &Path, repo: &str, o: &ArchiveOpts) -> Result<Value> {
    if o.why.trim().is_empty() || o.by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--why and --by are required and must not be empty".into()));
    }
    if o.force_with_reason.is_some_and(|r| r.trim().is_empty()) {
        return Err(AppError::InvalidArguments("--force-with-reason needs the reason".into()));
    }
    let policy = super::relevance::load_policy(root, o.policy)?;
    let store = if o.apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let r = store.repo_by_name(repo)?.ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    if r.role == "root" {
        return Err(AppError::structured(
            "cannot_archive_root",
            format!("{repo} is the workspace root; the root is never archived, with or without --force-with-reason"),
            None,
            None,
        ));
    }
    if store.meta_get(&format!("{ARCHIVED_PREFIX}{repo}"))?.is_some() {
        return Err(AppError::structured("already_archived", format!("{repo} is already archived"), Some(format!("`comparegraph restore {repo}` first")), None));
    }
    let rating = super::relevance::rate(&store, root, policy.as_ref(), None, None)?;
    let rated = rating.items.iter().find(|i| i.repo == repo).cloned();
    // what only a forced reason may override
    let mut guards: Vec<String> = vec![];
    if let Some(p) = policy.as_ref().and_then(|p| p.policy.primary(repo).cloned()) {
        guards.push(format!("primary for {} ({})", p.domain, p.why));
    }
    if rated.as_ref().is_some_and(|x| x.pinned) {
        guards.push(format!("pinned: {}", rated.as_ref().unwrap().rule));
    }
    if store.adopted_repos()?.iter().any(|n| n == repo) {
        guards.push("a decision adopts it (or one of its features)".into());
    }
    if let Some(n) = rated.as_ref().map(|x| x.factors.pull_rows).filter(|n| *n > 0) {
        guards.push(format!("{n} pull row(s) of the policy's port list name it as a source"));
    }
    if !guards.is_empty() && o.force_with_reason.is_none() {
        return Err(AppError::structured(
            "archive_refused",
            format!("{repo} is protected: {}", guards.join("; ")),
            Some("archive it anyway only with --force-with-reason \"<why>\" (recorded in the manifest and the log)".into()),
            Some(json!({ "repo": repo, "guards": guards })),
        ));
    }
    let id = r.id.unwrap();
    let claims = super::claim::claims_in(&store, Some(repo), None, None, true)?;
    let mut csv = String::from("id,family,feature,verdict,confidence,author,created_at,supersedes_id,note,locations\n");
    for c in &claims {
        let locs: Vec<String> = c.locations.iter().map(|l| format!("{}:{}-{}", l.path, l.line_start, l.line_end)).collect();
        csv.push_str(&[
            c.id.to_string(), csv_cell(&c.family), csv_cell(&c.feature), csv_cell(&c.verdict), csv_cell(&c.confidence),
            csv_cell(&c.author), csv_cell(&c.created_at), c.supersedes_id.map(|x| x.to_string()).unwrap_or_default(),
            csv_cell(&c.note), csv_cell(&locs.join("; ")),
        ].join(","));
        csv.push('\n');
    }
    let live = {
        let mut seen = BTreeSet::new();
        claims.iter().rev().filter(|c| seen.insert((c.family.clone(), c.feature.clone()))).count()
    };
    let runs = store.runs_of_repo(id)?;
    let graph_nodes: i64 = runs.iter().map(|r| r["nodes"].as_i64().unwrap_or(0)).sum();
    let mpath = governance::manifest_path(root, repo);
    let cpath = governance::archive_dir(root).join(format!("{repo}.claims.csv"));
    let plan = json!({
        "repo": repo, "manifest": mpath, "claims_file": cpath, "clone_path": r.path, "graph_nodes": graph_nodes,
        "claims": claims.len(), "guards": guards, "forced": o.force_with_reason,
        "rating": rated.as_ref().map(|x| json!({"class": x.class, "score": x.score})),
    });
    if !o.apply {
        return Ok(json!({ "dry_run": true, "plan": plan }));
    }
    let head = super::git(Path::new(&r.path), &["rev-parse", "HEAD"]).or(r.head.clone());
    let remote = super::git(Path::new(&r.path), &["remote", "get-url", "origin"]).or(r.remote.clone());
    let bytes = store.file_bytes();
    // everything under the store's write lock: a concurrent archive of the same repo waits, then finds the flag and
    // writes no manifest over this one. The manifest is on disk and re-read before the flag (manifest first).
    let (sha, entry) = store.with_write_lock(|s| -> Result<(String, Value)> {
        if s.meta_get(&format!("{ARCHIVED_PREFIX}{repo}"))?.is_some() {
            return Err(AppError::structured("already_archived", format!("{repo} is already archived"), None, None));
        }
        let csv_sha = write_verified(&cpath, csv.as_bytes())?;
        let manifest = json!({
            "schema": MANIFEST_SCHEMA, "repo": repo, "remote": remote, "head": head, "path_at_archive": r.path,
            "licence": r.licence, "lane": r.lane, "added_at": r.added_at, "archived_at": now_iso(), "by": o.by, "why": o.why,
            "forced": o.force_with_reason,
            "rating": rated.as_ref().map(|x| json!({"class": x.class, "score": x.score, "factors": x.factors, "policy_sha256": rating.inputs["policy_sha256"], "store_digest": rating.inputs["store_digest"]})),
            "runs": runs,
            "bands": rated.as_ref().and_then(|x| x.bands.clone()).unwrap_or_default(),
            "claims": {"count": claims.len(), "live": live, "file": cpath.file_name().map(|n| n.to_string_lossy().into_owned()), "sha256": csv_sha},
            "engine": governance::engine(),
        });
        let text = format!("{}
", serde_json::to_string_pretty(&manifest).unwrap());
        let sha = write_verified(&mpath, text.as_bytes())?;
        s.meta_set(&format!("{ARCHIVED_PREFIX}{repo}"), &sha)?;
        let entry = governance::append(s, root, &Action {
            verb: "archive".into(),
            repo: Some(repo.into()),
            by: o.by.into(),
            why: Some(o.why.into()),
            forced: o.force_with_reason.map(String::from),
            inputs: json!({"manifest_sha256": sha, "policy_sha256": rating.inputs["policy_sha256"], "store_digest": rating.inputs["store_digest"]}),
            bytes_before: Some(bytes),
            bytes_after: Some(bytes),
            ..Default::default()
        })?;
        Ok((sha, entry))
    })?;
    Ok(json!({
        "dry_run": false, "plan": plan, "manifest_sha256": sha, "action_seq": entry["seq"],
        "may_move": r.path, "note": "the clone folder may now be moved (check no longer reads it); the graph stays in the store",
    }))
}

pub struct RestoreOpts<'a> {
    pub by: &'a str,
    pub path: Option<&'a Path>,
    pub rebuild: bool,
    pub apply: bool,
}

/// `restore <manifest.json | repo>`: verify the manifest against the flag, find the clone at its head, lift the flag.
pub fn restore(root: &Path, target: &str, o: &RestoreOpts) -> Result<Value> {
    if o.by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required and must not be empty".into()));
    }
    let mpath = if target.ends_with(".json") { PathBuf::from(target) } else { governance::manifest_path(root, target) };
    let bytes = std::fs::read(&mpath).map_err(|e| AppError::Io(mpath.clone(), e))?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|e| AppError::InvalidArguments(format!("{}: {e}", mpath.display())))?;
    if manifest["schema"] != MANIFEST_SCHEMA {
        return Err(AppError::InvalidArguments(format!("{} is not an {MANIFEST_SCHEMA}", mpath.display())));
    }
    let repo = manifest["repo"].as_str().unwrap_or_default().to_string();
    let store = if o.apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let flag = store.meta_get(&format!("{ARCHIVED_PREFIX}{repo}"))?;
    let Some(flag) = flag else {
        return Err(AppError::structured("not_archived", format!("{repo} is not archived"), None, None));
    };
    let sha = sha256_hex(&bytes);
    if sha != flag {
        return Err(AppError::structured(
            "manifest_mismatch",
            format!("{} hashes to {sha}, the archive flag says {flag}: the manifest changed since the archive", mpath.display()),
            Some("restore from the manifest the archive wrote".into()),
            None,
        ));
    }
    let r = store.repo_by_name(&repo)?.ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let path = o.path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| manifest["path_at_archive"].as_str().unwrap_or(&r.path).to_string());
    let want = manifest["head"].as_str().map(String::from);
    if !Path::new(&path).is_dir() {
        let remote = manifest["remote"].as_str().unwrap_or("<remote>");
        return Err(AppError::structured(
            "clone_missing",
            format!("{path} does not exist; the engine makes no network call"),
            Some(format!("git clone {remote} {path} && git -C {path} checkout {}", want.as_deref().unwrap_or("<head>"))),
            Some(json!({"path": path, "remote": manifest["remote"], "head": want})),
        ));
    }
    let found = super::git(Path::new(&path), &["rev-parse", "HEAD"]);
    let same = match (&want, &found) {
        (Some(w), Some(f)) => w == f,
        (None, _) => true,
        _ => false,
    };
    if !same && !o.rebuild {
        return Err(AppError::structured(
            "head_mismatch",
            format!("{path} is at {found:?}, the archive recorded {want:?}"),
            Some("check out the recorded head, or pass --rebuild to build the clone as it is now (old claims then go stale honestly)".into()),
            Some(json!({"expected": want, "found": found})),
        ));
    }
    let plan = json!({"repo": repo, "manifest": mpath, "path": path, "head_matches": same, "rebuild": o.rebuild && !same});
    if !o.apply {
        return Ok(json!({"dry_run": true, "plan": plan}));
    }
    let bytes_now = store.file_bytes();
    let entry = store.with_write_lock(|s| -> Result<Value> {
        if path != r.path {
            s.set_repo_path_only(r.id.unwrap(), &path)?;
        }
        s.meta_delete(&format!("{ARCHIVED_PREFIX}{repo}"))?;
        governance::append(s, root, &Action {
            verb: "restore".into(),
            repo: Some(repo.clone()),
            by: o.by.into(),
            inputs: json!({"manifest_sha256": sha, "path": path, "rebuild": o.rebuild && !same}),
            bytes_before: Some(bytes_now),
            ..Default::default()
        })
    })?;
    drop(store);
    let mut built = Value::Null;
    if o.rebuild && !same {
        built = serde_json::to_value(super::build::build_repo(root, &repo, "reference", &Default::default())?).unwrap();
    }
    Ok(json!({"dry_run": false, "plan": plan, "action_seq": entry["seq"], "build": built}))
}
