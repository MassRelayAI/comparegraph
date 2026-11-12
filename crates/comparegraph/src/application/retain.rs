//! `gc` (lane CG-STORE A4, A5): store retention. Dry-run by default; `--apply` is CLI only, refused while another
//! comparegraph process has the store open, and logged in the action log. v1 touches only what the schema leaves
//! writable: `parse_cache` rows ("prunable, so no append-only triggers") and a stuck run's `status`. It never touches
//! node, edge, community, community_member, claim, location, decision or relatedness.
use super::governance::{self, Action};
use super::{AppError, Result};
use crate::domain::relevance::parse_ts_secs;
use crate::domain::retention::{cache_versions_to_keep, run_is_abandoned};
use crate::domain::RunStatus;
use crate::providers::GraphProvider;
use crate::store::{now_iso, Store};
use serde_json::{json, Value};
use std::path::Path;

/// Free bytes on the volume holding `p` (None where it cannot be asked).
#[cfg(unix)]
pub fn free_bytes(p: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path and `s` a zeroed statvfs the call fills; nothing else is touched
    let r = unsafe { libc::statvfs(c.as_ptr(), &mut s) };
    (r == 0).then(|| s.f_bavail as u64 * s.f_frsize as u64)
}
#[cfg(not(unix))]
pub fn free_bytes(_p: &Path) -> Option<u64> {
    None
}

fn refuse_busy(store: &Store) -> Result<()> {
    let others = store.other_holders();
    if !others.is_empty() {
        return Err(AppError::structured(
            "store_busy",
            format!("the store is open in {}; gc --apply refuses while another process holds it", others.join(", ")),
            Some("retry when they have finished (on onyx: never during a chain)".into()),
            Some(json!({ "holders": others })),
        ));
    }
    if !store.write_lock_free()? {
        return Err(AppError::structured(
            "store_busy",
            "another connection holds the store's write lock".into(),
            Some("retry when it has finished".into()),
            None,
        ));
    }
    Ok(())
}

pub struct CacheOpts<'a> {
    pub keep_versions: Option<usize>,
    pub all: bool,
    pub apply: bool,
    pub by: &'a str,
    /// a policy's `retention.parse_cache_versions`, used when `keep_versions` is not given
    pub policy_keep: Option<usize>,
}

/// `gc --parse-cache`: keep the running binary's provider version (and the `n - 1` newest others), or nothing with
/// `--all-cache`; then VACUUM. Dry-run reports an estimate from the payload sums, labelled `estimated`.
pub fn gc_parse_cache(root: &Path, o: &CacheOpts) -> Result<Value> {
    if o.keep_versions == Some(0) {
        return Err(AppError::InvalidArguments("--keep-versions must be at least 1 (use --all-cache to drop every row)".into()));
    }
    let current = crate::providers::reference::ReferenceProvider.version();
    let keep_n = o.keep_versions.or(o.policy_keep).unwrap_or(1);
    let store = if o.apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let versions = store.parse_cache_versions()?;
    let present: Vec<String> = versions.iter().map(|v| v.provider_version.clone()).collect();
    let keep: Vec<String> = if o.all { vec![] } else { cache_versions_to_keep(&present, &current, keep_n) };
    let dropped: Vec<&crate::store::CacheVersion> = versions.iter().filter(|v| !keep.contains(&v.provider_version)).collect();
    let drop_rows: i64 = dropped.iter().map(|v| v.rows).sum();
    let drop_bytes: i64 = dropped.iter().map(|v| v.payload_bytes).sum();
    let bytes_before = store.file_bytes();
    let mut limitations = vec![];
    if o.all {
        limitations.push(json!({"code": "cache_dropped", "detail": "--all-cache: the next build of every clone parses every file again"}));
    }
    let base = json!({
        "current_version": current, "versions": versions, "keep": keep,
        "drop": dropped.iter().map(|v| &v.provider_version).collect::<Vec<_>>(),
        "drop_rows": drop_rows, "drop_payload_bytes": drop_bytes, "bytes_before": bytes_before,
    });
    if !o.apply {
        let mut v = base;
        v["dry_run"] = json!(true);
        v["bytes_after"] = json!(bytes_before.saturating_sub(drop_bytes.max(0) as u64));
        v["bytes_after_is"] = json!("estimated");
        limitations.push(json!({"code": "estimated", "detail": "bytes_after is the file size minus the dropped payload; page overhead and the VACUUM decide the real figure"}));
        v["limitation_items"] = json!(limitations);
        return Ok(v);
    }
    if o.by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required with --apply".into()));
    }
    refuse_busy(&store)?;
    // VACUUM writes a temporary copy on the same volume
    if let Some(free) = free_bytes(&store.dir()) {
        if (free as f64) < 1.2 * bytes_before as f64 {
            return Err(AppError::structured(
                "insufficient_space",
                format!("VACUUM needs about 1.2 x the store ({bytes_before} bytes) free on its volume; {free} bytes are free"),
                Some("free space first; nothing was changed".into()),
                Some(json!({"free": free, "store_bytes": bytes_before})),
            ));
        }
    } else {
        limitations.push(json!({"code": "free_space_unknown", "detail": "free space could not be read on this platform"}));
    }
    let rows = store.prune_parse_cache_except(&keep)?;
    store.vacuum()?;
    let bytes_after = store.file_bytes();
    let entry = governance::append_locked(&store, root, &Action {
        verb: "gc-parse-cache".into(),
        by: o.by.into(),
        inputs: json!({"keep": keep, "all_cache": o.all}),
        bytes_before: Some(bytes_before),
        bytes_after: Some(bytes_after),
        detail: json!({"rows_deleted": rows}),
        ..Default::default()
    })?;
    let mut v = base;
    v["dry_run"] = json!(false);
    v["rows_deleted"] = json!(rows);
    v["bytes_after"] = json!(bytes_after);
    v["bytes_after_is"] = json!("measured");
    v["action_seq"] = entry["seq"].clone();
    v["limitation_items"] = json!(limitations);
    Ok(v)
}

/// `gc --stale-runs`: runs still `running` that started more than `older_than_hours` ago become `abandoned`. Nothing is
/// marked while a live `build` process has the store open (its run is the one that looks stuck).
pub fn gc_stale_runs(root: &Path, older_than_hours: u64, apply: bool, by: &str) -> Result<Value> {
    if older_than_hours == 0 {
        return Err(AppError::InvalidArguments("--older-than-hours must be at least 1".into()));
    }
    let store = if apply { Store::open_guarded(root)? } else { Store::open_read_only(root)? };
    let now = parse_ts_secs(&now_iso()).unwrap_or(0);
    let max_age = older_than_hours as i64 * 3600;
    let running = store.runs_with_status(RunStatus::Running)?;
    let repos = store.repos()?;
    let name_of = |id: crate::domain::RepoId| repos.iter().find(|r| r.id == Some(id)).map(|r| r.name.clone()).unwrap_or_default();
    let mut stale = vec![];
    let mut young = vec![];
    for r in &running {
        let row = json!({"run": r.id.map(|x| x.0), "repo": name_of(r.repo), "started_at": r.started_at});
        if run_is_abandoned(parse_ts_secs(&r.started_at), now, max_age) {
            stale.push((r.id.unwrap(), row));
        } else {
            young.push(row);
        }
    }
    let builds: Vec<String> = store.other_holders().into_iter().filter(|h| h.starts_with("build")).collect();
    let mut limitations = vec![];
    if !builds.is_empty() {
        limitations.push(json!({"code": "live_build", "detail": format!("a build is running ({}); no run is marked while one is", builds.join(", "))}));
    }
    let base = json!({
        "older_than_hours": older_than_hours,
        "stale": stale.iter().map(|s| &s.1).collect::<Vec<_>>(),
        "running_but_young": young,
    });
    if !apply {
        let mut v = base;
        v["dry_run"] = json!(true);
        v["limitation_items"] = json!(limitations);
        return Ok(v);
    }
    if by.trim().is_empty() {
        return Err(AppError::InvalidArguments("--by is required with --apply".into()));
    }
    if !builds.is_empty() {
        return Err(AppError::structured(
            "store_busy",
            format!("a build is running ({}); gc --stale-runs marks nothing while one is", builds.join(", ")),
            Some("retry when it has finished".into()),
            None,
        ));
    }
    let ids: Vec<i64> = stale.iter().map(|s| s.0 .0).collect();
    let bytes = store.file_bytes();
    let entry = store.with_write_lock(|s| -> Result<Value> {
        for (id, _) in &stale {
            s.set_run_status(*id, RunStatus::Abandoned)?;
        }
        governance::append(s, root, &Action {
            verb: "gc-stale-runs".into(),
            by: by.into(),
            inputs: json!({"older_than_hours": older_than_hours, "runs": ids}),
            bytes_before: Some(bytes),
            ..Default::default()
        })
    })?;
    let mut v = base;
    v["dry_run"] = json!(false);
    v["marked_abandoned"] = json!(ids);
    v["action_seq"] = entry["seq"].clone();
    v["limitation_items"] = json!(limitations);
    Ok(v)
}
