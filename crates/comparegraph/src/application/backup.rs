//! `backup` and `restore-store` (lane CG-STORE A1, A2). The claims are the one part of the store that cannot be
//! rebuilt from the clones; these verbs copy it out and put it back, and never migrate the store they read.
//!
//! - full: `VACUUM INTO` a new file (one consistent snapshot, readers and writers keep going under WAL), then
//!   `PRAGMA quick_check` and row counts of the copy against the source; `backup-manifest/1` beside it.
//! - `--claims-only`: one JSONL of the non-rebuildable tables (`store::NON_REBUILDABLE`) and the action log, read in one
//!   snapshot; a few MB instead of GB.
//! - `restore-store`: verify the backup's sha256 and integrity, refuse while the store is open elsewhere, take a
//!   pre-restore backup of the current store, move the current files aside (never deleted), then put the copy in.
use super::governance::{self, Action};
use super::{AppError, Result};
use crate::domain::RunStatus;
use crate::store::{self, now_iso, Store};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

pub const MANIFEST_SCHEMA: &str = "backup-manifest/1";
pub const CLAIMS_SCHEMA: &str = "claims-backup/1";

pub fn default_out(root: &Path) -> PathBuf {
    root.join(".comparegraph").join("backups")
}
/// `2026-11-12T10:15:00Z` -> `20261112T101500Z` (file-name safe, sorts by time).
pub fn stamp(iso: &str) -> String {
    iso.chars().filter(|c| *c != '-' && *c != ':').collect()
}
pub fn manifest_of(backup: &Path) -> PathBuf {
    PathBuf::from(format!("{}.manifest.json", backup.display()))
}
/// The sha256 of a file, streamed (a full backup is GB).
pub fn file_sha256(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p).map_err(|e| AppError::Io(p.to_path_buf(), e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| AppError::Io(p.to_path_buf(), e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
fn write_synced(p: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::File::create(p).map_err(|e| AppError::Io(p.to_path_buf(), e))?;
    f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| AppError::Io(p.to_path_buf(), e))
}
fn sync_file(p: &Path) -> Result<()> {
    std::fs::File::open(p).and_then(|f| f.sync_all()).map_err(|e| AppError::Io(p.to_path_buf(), e))
}
fn counts_json(c: &[(String, i64)]) -> Value {
    Value::Object(c.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
}
/// sha256 over the sorted `(repo, latest done run store_hash)` pairs and the max claim id: what a backup or a rating
/// was taken from.
pub fn store_digest(store: &Store) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    for r in store.repos()? {
        let h = match r.id {
            Some(id) => store.latest_done_run(id)?.and_then(|x| x.store_hash).unwrap_or_default(),
            None => String::new(),
        };
        lines.push(format!("{}\t{h}", r.name));
    }
    lines.sort();
    lines.push(format!("max_claim\t{}", store.max_claim_id()?));
    Ok(crate::domain::actionlog::sha256_hex(lines.join("\n").as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Full,
    ClaimsOnly,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Full => "full",
            Kind::ClaimsOnly => "claims-only",
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Kind::Full => "comparegraph-",
            Kind::ClaimsOnly => "comparegraph-claims-",
        }
    }
    fn ext(self) -> &'static str {
        match self {
            Kind::Full => ".db",
            Kind::ClaimsOnly => ".jsonl",
        }
    }
    /// A file of this kind in a backup folder (`comparegraph-<stamp>.db`, `comparegraph-claims-<stamp>.jsonl`).
    fn owns(self, name: &str) -> bool {
        let rest = match name.strip_prefix(self.prefix()) {
            Some(r) => r,
            None => return false,
        };
        // `comparegraph-claims-...` is not a full backup
        rest.ends_with(self.ext()) && !(self == Kind::Full && rest.starts_with("claims-"))
    }
}

/// Take one backup of the store at `root` into `out`; `keep` then removes the oldest of the same kind in `out` beyond
/// `keep` (with their manifests). Returns the envelope data.
pub fn backup(root: &Path, out: Option<&Path>, kind: Kind, keep: Option<usize>, by: &str) -> Result<Value> {
    if by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required and must not be empty".into()));
    }
    if keep == Some(0) {
        return Err(AppError::InvalidArguments("--keep must be at least 1".into()));
    }
    let out = out.map(Path::to_path_buf).unwrap_or_else(|| default_out(root));
    std::fs::create_dir_all(&out).map_err(|e| AppError::Io(out.clone(), e))?;
    let src = Store::open_read_only(root)?;
    let now = now_iso();
    let file = out.join(format!("{}{}{}", kind.prefix(), stamp(&now), kind.ext()));
    if file.exists() || manifest_of(&file).exists() {
        return Err(AppError::InvalidArguments(format!("{} already exists; a backup is never overwritten", file.display())));
    }
    let partial = PathBuf::from(format!("{}.partial", file.display()));
    let _ = std::fs::remove_file(&partial);
    let mut limitations: Vec<Value> = Vec::new();
    let (counts, source_before, source_after, consistent) = match kind {
        Kind::Full => {
            let before = src.row_counts(store::COUNTED)?;
            src.vacuum_into(&partial)?;
            let after = src.row_counts(store::COUNTED)?;
            sync_file(&partial)?;
            let qc = store::quick_check(&partial)?;
            if qc != "ok" {
                let _ = std::fs::remove_file(&partial);
                return Err(AppError::check_failed_hint(format!("the copy failed quick_check: {qc}"), "nothing was kept; retry"));
            }
            let got = store::file_row_counts(&partial, store::COUNTED)?;
            let consistent = counts_between(&got, &before, &after);
            if !consistent {
                let _ = std::fs::remove_file(&partial);
                return Err(AppError::check_failed_hint(
                    format!("the copy's row counts {} match neither the source before {} nor after {}", counts_json(&got), counts_json(&before), counts_json(&after)),
                    "nothing was kept; retry",
                ));
            }
            if before != after {
                limitations.push(json!({"code": "source_changed_during_backup", "detail": "the store was written while the copy ran; the copy is one consistent snapshot between the two counts"}));
            }
            (got, before, after, consistent)
        }
        Kind::ClaimsOnly => {
            let (text, counts) = src.read_snapshot(|s| claims_dump(s, root, &now))?;
            write_synced(&partial, text.as_bytes())?;
            // re-read what landed on disk and count its rows: the file must hold what the snapshot held
            let reread = read_claims_file(&partial)?;
            let got: Vec<(String, i64)> = store::NON_REBUILDABLE.iter().map(|t| (t.to_string(), reread.rows.get(*t).map_or(0, |v| v.len() as i64))).collect();
            let want: Vec<(String, i64)> = store::NON_REBUILDABLE.iter().map(|t| (t.to_string(), counts.iter().find(|c| c.0 == *t).map_or(0, |c| c.1))).collect();
            if got != want {
                let _ = std::fs::remove_file(&partial);
                return Err(AppError::check_failed_hint(format!("the file's rows {} differ from the snapshot's {}", counts_json(&got), counts_json(&want)), "nothing was kept; retry"));
            }
            (got, counts.clone(), counts, true)
        }
    };
    std::fs::rename(&partial, &file).map_err(|e| AppError::Io(file.clone(), e))?;
    let sha = file_sha256(&file)?;
    let bytes = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let manifest = json!({
        "schema": MANIFEST_SCHEMA, "kind": kind.name(), "file": file.file_name().map(|n| n.to_string_lossy().into_owned()),
        "sha256": sha, "bytes": bytes, "created_at": now, "by": by, "engine": governance::engine(),
        "sqlite": Store::sqlite_version(), "source": src.path(), "source_bytes": src.file_bytes(),
        "store_digest": store_digest(&src)?, "store_predates_binary": src.predates_this_binary()?,
        "counts": counts_json(&counts), "source_counts_before": counts_json(&source_before),
        "source_counts_after": counts_json(&source_after), "consistent": consistent,
    });
    write_synced(&manifest_of(&file), format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()).as_bytes())?;
    let removed = match keep {
        Some(n) => prune_backups(&out, kind, n)?,
        None => vec![],
    };
    let store_bytes = src.file_bytes();
    drop(src);
    let logged = Store::open_guarded(root).map_err(AppError::from).and_then(|w| {
        governance::append_locked(&w, root, &Action {
            verb: "backup".into(),
            by: by.into(),
            inputs: json!({"kind": kind.name(), "file": file, "sha256": sha, "store_digest": manifest["store_digest"]}),
            bytes_before: Some(store_bytes),
            bytes_after: Some(store_bytes),
            detail: json!({"backup_bytes": bytes, "removed": removed}),
            ..Default::default()
        })
    });
    let logged = match logged {
        Ok(e) => e["seq"].clone(),
        Err(e) => {
            // the copy is good either way; a backup must still be possible when the log is broken
            limitations.push(json!({"code": "action_log_not_written", "detail": e.to_string()}));
            Value::Null
        }
    };
    Ok(json!({
        "kind": kind.name(), "file": file, "manifest": manifest_of(&file), "sha256": sha, "bytes": bytes,
        "counts": counts_json(&counts), "verified": true, "removed": removed, "action_seq": logged,
        "limitation_items": limitations,
    }))
}

/// Every count of `got` lies between `before` and `after` (equal to both when nothing was written meanwhile).
fn counts_between(got: &[(String, i64)], before: &[(String, i64)], after: &[(String, i64)]) -> bool {
    got.len() == before.len()
        && got.iter().zip(before).zip(after).all(|(((_, g), (_, b)), (_, a))| {
            let (lo, hi) = if b <= a { (b, a) } else { (a, b) };
            g >= lo && g <= hi
        })
}

/// Remove the oldest backups of `kind` in `dir` beyond `keep` (by file name, which sorts by time), with manifests.
fn prune_backups(dir: &Path, kind: Kind, keep: usize) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| AppError::Io(dir.to_path_buf(), e))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| kind.owns(n))
        .collect();
    names.sort();
    let drop_n = names.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for n in names.into_iter().take(drop_n) {
        let p = dir.join(&n);
        std::fs::remove_file(&p).map_err(|e| AppError::Io(p.clone(), e))?;
        let _ = std::fs::remove_file(manifest_of(&p));
        removed.push(n);
    }
    Ok(removed)
}

/// The claims-only JSONL: a header, then one `{"t": table, "r": [values]}` per row, then one `{"t": "_actions", "line":
/// ...}` per action-log line. Read inside one snapshot.
fn claims_dump(s: &Store, root: &Path, now: &str) -> Result<(String, Vec<(String, i64)>)> {
    let mut body = String::new();
    let mut tables = Vec::new();
    let mut counts = Vec::new();
    for t in store::NON_REBUILDABLE {
        let (cols, rows) = s.dump_table(t)?;
        counts.push((t.to_string(), rows.len() as i64));
        tables.push(json!({"name": t, "columns": cols, "rows": rows.len()}));
        for r in rows {
            body.push_str(&json!({"t": t, "r": r}).to_string());
            body.push('\n');
        }
    }
    let log = std::fs::read_to_string(governance::log_path(root)).unwrap_or_default();
    let mut n_actions = 0;
    for l in log.lines().filter(|l| !l.is_empty()) {
        body.push_str(&json!({"t": "_actions", "line": l}).to_string());
        body.push('\n');
        n_actions += 1;
    }
    let head = json!({"schema": CLAIMS_SCHEMA, "created_at": now, "engine": governance::engine(), "tables": tables, "actions": n_actions});
    Ok((format!("{head}\n{body}"), counts))
}

/// A parsed claims-only backup.
pub struct ClaimsFile {
    pub header: Value,
    pub columns: std::collections::HashMap<String, Vec<String>>,
    pub rows: std::collections::HashMap<String, Vec<Vec<Value>>>,
    pub actions: Vec<String>,
}
pub fn read_claims_file(p: &Path) -> Result<ClaimsFile> {
    let f = std::fs::File::open(p).map_err(|e| AppError::Io(p.to_path_buf(), e))?;
    let mut lines = std::io::BufReader::new(f).lines();
    let bad = |m: String| AppError::structured("backup_mismatch", format!("{}: {m}", p.display()), None, None);
    let header: Value = match lines.next() {
        Some(Ok(l)) => serde_json::from_str(&l).map_err(|e| bad(format!("header: {e}")))?,
        _ => return Err(bad("empty file".into())),
    };
    if header["schema"] != CLAIMS_SCHEMA {
        return Err(bad(format!("schema is not {CLAIMS_SCHEMA}")));
    }
    let mut columns = std::collections::HashMap::new();
    for t in header["tables"].as_array().into_iter().flatten() {
        let cols: Vec<String> = t["columns"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
        columns.insert(t["name"].as_str().unwrap_or("").to_string(), cols);
    }
    let mut rows: std::collections::HashMap<String, Vec<Vec<Value>>> = Default::default();
    let mut actions = Vec::new();
    for (i, l) in lines.enumerate() {
        let l = l.map_err(|e| AppError::Io(p.to_path_buf(), e))?;
        let v: Value = serde_json::from_str(&l).map_err(|e| bad(format!("line {}: {e}", i + 2)))?;
        match v["t"].as_str() {
            Some("_actions") => actions.push(v["line"].as_str().unwrap_or("").to_string()),
            Some(t) if columns.contains_key(t) => rows.entry(t.to_string()).or_default().push(v["r"].as_array().cloned().unwrap_or_default()),
            _ => return Err(bad(format!("line {}: unknown table", i + 2))),
        }
    }
    Ok(ClaimsFile { header, columns, rows, actions })
}

/// `restore-store <backup>`: dry-run by default (what would be replaced); `--apply` replaces the store.
pub fn restore_store(root: &Path, backup: &Path, apply: bool, by: &str) -> Result<Value> {
    let mpath = manifest_of(backup);
    let manifest: Value = std::fs::read_to_string(&mpath)
        .map_err(|e| AppError::Io(mpath.clone(), e))
        .and_then(|t| serde_json::from_str(&t).map_err(|e| AppError::InvalidArguments(format!("{}: {e}", mpath.display()))))?;
    if manifest["schema"] != MANIFEST_SCHEMA {
        return Err(AppError::InvalidArguments(format!("{} is not a {MANIFEST_SCHEMA}", mpath.display())));
    }
    let sha = file_sha256(backup)?;
    if Some(sha.as_str()) != manifest["sha256"].as_str() {
        return Err(AppError::structured(
            "backup_mismatch",
            format!("{} hashes to {sha}, its manifest says {}; the store was not touched", backup.display(), manifest["sha256"]),
            Some("use another backup".into()),
            None,
        ));
    }
    let kind = if manifest["kind"] == "claims-only" { Kind::ClaimsOnly } else { Kind::Full };
    if kind == Kind::Full {
        let qc = store::quick_check(backup)?;
        if qc != "ok" {
            return Err(AppError::structured("backup_mismatch", format!("{} fails quick_check: {qc}", backup.display()), None, None));
        }
    } else {
        read_claims_file(backup)?;
    }
    let dir = root.join(".comparegraph");
    let db = dir.join("comparegraph.db");
    let exists = db.is_file();
    let current_bytes = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    if exists {
        // in use: another comparegraph process has it open, or another connection holds the write lock
        let s = Store::open_guarded(root)?;
        let others = s.other_holders();
        let lock_free = s.write_lock_free()?;
        drop(s);
        if !others.is_empty() || !lock_free {
            return Err(AppError::structured(
                "store_busy",
                format!("the store is in use ({}); restore-store refuses to replace it", if others.is_empty() { "another connection holds the write lock".to_string() } else { others.join(", ") }),
                Some("stop the other process (on onyx: no chain running) and retry".into()),
                Some(json!({"holders": others})),
            ));
        }
    }
    let now = now_iso();
    let replaced = dir.join(format!("comparegraph.db.replaced-{}", stamp(&now)));
    let plan = json!({
        "backup": backup, "kind": kind.name(), "sha256": sha, "backup_counts": manifest["counts"],
        "store": db, "store_exists": exists, "store_bytes": current_bytes,
        "pre_restore_backup_dir": default_out(root), "replaced_to": if exists { json!(replaced) } else { Value::Null },
    });
    if !apply {
        return Ok(json!({"dry_run": true, "apply": false, "plan": plan}));
    }
    if by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required with --apply".into()));
    }
    let mut pre = Value::Null;
    if exists {
        // a pre-restore backup first: the replaced store is also moved aside, never deleted
        pre = self::backup(root, Some(&default_out(root)), Kind::Full, None, by)?;
        for ext in ["", "-wal", "-shm"] {
            let from = PathBuf::from(format!("{}{ext}", db.display()));
            if from.exists() {
                let to = PathBuf::from(format!("{}{ext}", replaced.display()));
                std::fs::rename(&from, &to).map_err(|e| AppError::Io(from.clone(), e))?;
            }
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Io(dir.clone(), e))?;
    let mut limitations = Vec::new();
    match kind {
        Kind::Full => {
            let partial = dir.join("comparegraph.db.restoring");
            std::fs::copy(backup, &partial).map_err(|e| AppError::Io(partial.clone(), e))?;
            sync_file(&partial)?;
            std::fs::rename(&partial, &db).map_err(|e| AppError::Io(db.clone(), e))?;
            let got = store::file_row_counts(&db, store::COUNTED)?;
            if counts_json(&got) != manifest["counts"] {
                return Err(AppError::check_failed_hint(format!("restored counts {} differ from the manifest", counts_json(&got)), "the replaced store is beside it; move it back"));
            }
        }
        Kind::ClaimsOnly => {
            let cf = read_claims_file(backup)?;
            let s = Store::open(root)?;
            if !s.is_empty_store()? {
                return Err(AppError::InvalidArguments("the new store is not empty".into()));
            }
            s.set_foreign_keys(false)?;
            s.with_write_lock(|s| -> Result<()> {
                for t in store::NON_REBUILDABLE {
                    let cols = cf.columns.get(*t).cloned().unwrap_or_default();
                    let rows = cf.rows.get(*t).cloned().unwrap_or_default();
                    s.insert_dumped(t, &cols, &rows)?;
                }
                s.mark_all_runs(RunStatus::Restored)?;
                Ok(())
            })?;
            s.set_foreign_keys(true)?;
            if !governance::log_path(root).exists() && !cf.actions.is_empty() {
                let body: String = cf.actions.iter().map(|l| format!("{l}\n")).collect();
                write_synced(&governance::log_path(root), body.as_bytes())?;
                limitations.push(json!({"code": "action_log_restored", "detail": "the action log file was missing; it was written from the backup"}));
            }
            limitations.push(json!({"code": "graph_not_restored", "detail": "runs come back with status `restored` and no graph; `build` each repo, then `check`"}));
        }
    }
    let w = Store::open_guarded(root)?;
    let entry = w.with_write_lock(|w| -> Result<Value> {
        let meta_head_in_backup = w.meta_get(governance::HEAD_KEY)?;
        governance::adopt_file_head(w, root)?;
        governance::append(w, root, &Action {
            verb: "restore-store".into(),
            by: by.into(),
            inputs: json!({"backup": backup, "sha256": sha, "kind": kind.name()}),
            bytes_before: Some(current_bytes),
            bytes_after: Some(w.file_bytes()),
            detail: json!({"replaced_to": if exists { json!(replaced) } else { Value::Null }, "meta_head_in_backup": meta_head_in_backup}),
            ..Default::default()
        })
    })?;
    Ok(json!({"dry_run": false, "apply": true, "plan": plan, "pre_restore_backup": pre, "action_seq": entry["seq"], "limitation_items": limitations}))
}
