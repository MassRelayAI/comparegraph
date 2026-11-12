//! `claim` and `claims`: the engine's evidence record. A claim is written only with an explicit author and, for
//! has/partial, with locations that resolve now. Append-only: a new claim for the same (repo, feature) supersedes.
use super::{AppError, Result};
use crate::domain::Limitation;
use crate::store::{now_iso, Store};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::path::{Component, Path};

#[derive(Debug, Serialize, Clone)]
pub struct LocationOut {
    pub id: i64,
    pub node_id: Option<i64>,
    pub path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub content_hash: String,
    pub stale: bool,
}
#[derive(Debug, Serialize, Clone)]
pub struct ClaimOut {
    pub id: i64,
    pub repo: String,
    pub family: String,
    pub feature: String,
    pub run_id: i64,
    pub verdict: String,
    pub confidence: String,
    pub author: String,
    pub created_at: String,
    pub supersedes_id: Option<i64>,
    pub note: String,
    pub locations: Vec<LocationOut>,
    /// The upstream commit the citations were read at (`--at-commit`), when the caller gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_commit: Option<String>,
    /// Qualifier: the evidence is upstream at this path (`path[:lines]`), outside the exported tree (`--out-of-subset`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_subset: Option<String>,
    /// has/partial only: whether an entry point reaches the cited symbol, computed at write time (`reach.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reach: Option<super::reach::Reach>,
    /// True when a batch row equalled the live claim and nothing was written; `id` is the existing claim.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
    /// carried to the envelope's evidence.limitations by the CLI (`read_capped`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limitation_items: Vec<Limitation>,
}

pub struct ClaimInput<'a> {
    pub repo: &'a str,
    pub feature: &'a str,
    pub family: Option<&'a str>,
    pub verdict: &'a str,
    pub author: &'a str,
    pub at: &'a [String],
    pub searched: Option<&'a str>,
    /// Qualified names of the search hits a reader opened and rejected (`none --searched` only); see `claim_on`.
    pub read_hits: &'a [String],
    pub note: Option<&'a str>,
    /// Write nothing when the live claim for the feature already equals this one (a batch resumed after an interruption).
    pub skip_equal: bool,
    /// What the citations must be, beyond resolving: see `Evidence`.
    pub evidence: Evidence<'a>,
}

/// The write-time checks and qualifiers of a citation (field-run asks 7, 8, 11, 14, 15); the default checks the line
/// content only.
#[derive(Default, Clone, Copy)]
pub struct Evidence<'a> {
    /// `--at-commit`: the upstream commit the citations were read at; must agree with the head the build read.
    pub at_commit: Option<&'a str>,
    /// `--anchor 'path::regex'`: each resolves to the one code line the regex matches, stored as `path:N`.
    pub anchors: &'a [String],
    /// `--expect`: one pattern for every `--at`, or one per `--at` in order; checked against the cited lines.
    pub expect: &'a [String],
    /// `--allow-comment`: accept a citation that is only blank, comment or docstring lines.
    pub allow_comment: bool,
    /// `--out-of-subset <upstream path[:lines]>`: has/partial evidence that the exported tree does not contain.
    pub out_of_subset: Option<&'a str>,
    /// `--no-reach`: skip the reach walk; the claim records no reach (as a none or unknown does).
    pub no_reach: bool,
}

/// `a, b, …` up to `n` items, then `+k more` (`…` for a word list).
fn capped<S: AsRef<str>>(xs: &[S], n: usize) -> String {
    let shown: Vec<&str> = xs.iter().take(n).map(AsRef::as_ref).collect();
    let mut out = shown.join(", ");
    if xs.len() > n {
        out.push_str(&format!(", +{} more", xs.len() - n));
    }
    out
}

fn invalid<T>(m: impl Into<String>) -> Result<T> {
    Err(AppError::InvalidArguments(m.into()))
}

/// `path:start-end` or `path:line`; split on the LAST ':'.
pub fn parse_at(at: &str) -> Result<(String, usize, usize)> {
    let Some((p, range)) = at.rsplit_once(':') else {
        return invalid(format!(
            "--at {at}: expected <path>:<start>-<end> or <path>:<line>"
        ));
    };
    let num = |s: &str| {
        s.parse::<usize>().map_err(|_| {
            AppError::InvalidArguments(format!("--at {at}: {s:?} is not a line number"))
        })
    };
    let (s, e) = match range.split_once('-') {
        Some((a, b)) => (num(a)?, num(b)?),
        None => {
            let n = num(range)?;
            (n, n)
        }
    };
    if p.is_empty() {
        return invalid(format!("--at {at}: empty path"));
    }
    if s == 0 {
        return invalid(format!("--at {at}: lines start at 1"));
    }
    if s > e {
        return invalid(format!("--at {at}: a reversed range ({s} > {e}); give <start>-<end> with start <= end"));
    }
    Ok((p.to_string(), s, e))
}

/// Resolve a clone-relative path: no absolute path, no `..`, and the canonical file must stay inside the canonical root.
/// Returns (normalised relative path with '/', canonical file path).
fn safe_path(clone: &Path, rel: &str) -> Result<(String, std::path::PathBuf)> {
    let mut parts: Vec<String> = vec![];
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return invalid(format!(
                    "path {rel:?} must be relative to the clone and contain no '..'"
                ))
            }
        }
    }
    if parts.is_empty() {
        return invalid(format!("path {rel:?} is empty"));
    }
    let norm = parts.join("/");
    let root = clone
        .canonicalize()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    let full = clone.join(&norm).canonicalize().map_err(|_| {
        AppError::InvalidArguments(format!("{norm}: file does not exist in the clone"))
    })?;
    if !full.starts_with(&root) {
        return invalid(format!("{norm}: resolves outside the clone"));
    }
    if !full.is_file() {
        return invalid(format!("{norm}: not a file"));
    }
    Ok((norm, full))
}

pub(crate) fn lookup_feature(
    store: &Store,
    id: &str,
    family: Option<&str>,
) -> Result<(i64, String)> {
    let mut st = store.conn().prepare(
        "SELECT f.id, fa.name FROM feature f JOIN family fa ON fa.id = f.family_id \
         WHERE f.slug = ?1 AND (?2 IS NULL OR fa.name = ?2) ORDER BY fa.name",
    )?;
    let rows: Vec<(i64, String)> = st
        .query_map(params![id, family], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    match rows.len() {
        0 => Err(AppError::NotFound(format!(
            "no feature {id}{}",
            family.map(|f| format!(" in {f}")).unwrap_or_default()
        ))),
        1 => Ok(rows.into_iter().next().unwrap()),
        _ => invalid(format!(
            "feature {id} exists in several families ({}); pass --family",
            rows.iter()
                .map(|r| r.1.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn claim(root: &Path, i: &ClaimInput) -> Result<ClaimOut> {
    check_input(i)?;
    let store = Store::open(root)?;
    claim_on(&store, i)
}

fn check_input(i: &ClaimInput) -> Result<()> {
    if !["has", "partial", "none", "unknown"].contains(&i.verdict) {
        return invalid(format!(
            "unknown verdict {} (has|partial|none|unknown)",
            i.verdict
        ));
    }
    if i.author.trim().is_empty() {
        return invalid("--author is required");
    }
    if !i.read_hits.is_empty() && !(i.verdict == "none" && i.searched.is_some()) {
        return invalid("--read-hits needs --searched and a none verdict: it names the hits of that search that were read");
    }
    let ev = &i.evidence;
    if matches!(i.verdict, "has" | "partial") && i.at.is_empty() && ev.anchors.is_empty() && ev.out_of_subset.is_none()
    {
        return invalid(format!(
            "a {} claim needs at least one --at <path>:<start>-<end> (or --anchor 'path::regex', or --out-of-subset <upstream path>)",
            i.verdict
        ));
    }
    if ev.out_of_subset.is_some() && !matches!(i.verdict, "has" | "partial") {
        return invalid("--out-of-subset qualifies a has or partial claim: it names where upstream the evidence is");
    }
    if !ev.expect.is_empty() && i.at.is_empty() {
        return invalid("--expect checks an --at citation; an --anchor carries its own regex");
    }
    if ev.expect.len() > 1 && ev.expect.len() != i.at.len() {
        return invalid(format!(
            "{} --expect for {} --at: give one --expect for every --at, or one per --at in the same order",
            ev.expect.len(),
            i.at.len()
        ));
    }
    Ok(())
}

/// The body of `claim` on an already open store; inside a batch transaction the write is a savepoint.
pub fn claim_on(store: &Store, i: &ClaimInput) -> Result<ClaimOut> {
    claim_on_with(store, i, &mut super::reach::ReachCache::default())
}

/// `claim_on` with a reach cache shared across calls: a batch passes one for all its rows, so the run's reach state is
/// built once (`graph::ReachCache`).
pub fn claim_on_with(store: &Store, i: &ClaimInput, reach_cache: &mut super::reach::ReachCache) -> Result<ClaimOut> {
    check_input(i)?;
    let repo = store
        .repo_by_name(i.repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {}", i.repo)))?;
    super::archive::refuse_if_archived(store, i.repo)?;
    let rid = repo.id.unwrap();
    let run = store
        .latest_done_run(rid)?
        .ok_or_else(|| {
            AppError::NotFound(format!("{} has no finished run; build it first", i.repo))
        })?
        .id
        .unwrap();
    let (fid, fam) = lookup_feature(store, i.feature, i.family)?;
    let tier: String = store.conn().query_row(
        "SELECT tier FROM feature WHERE id = ?1",
        params![fid],
        |r| r.get(0),
    )?;
    if tier == "retired" {
        return invalid(format!("feature {} is retired", i.feature));
    }

    let mut note = String::new();
    let mut graph_absent = false;
    let mut read_confirmed = false;
    let mut limitation_items: Vec<Limitation> = vec![];
    if i.verdict == "none" {
        if i.searched.is_some_and(|s| s.trim().is_empty()) {
            return invalid("--searched has no searchable term");
        }
        if let Some(terms_text) = i.searched {
            let f = store
                .features_of(&fam)?
                .into_iter()
                .find(|f| f.id == fid)
                .ok_or_else(|| AppError::NotFound(format!("no feature {}", i.feature)))?;
            super::absent::guard_searched(store, &f, terms_text)?;
            let pin = super::absent::pinned_run(store, i.repo)?;
            // refusals are `none_refused` with the required `hits_read` list as data (see `judge_searched_none`)
            let j = super::absent::judge_searched_none(store, i.repo, pin.0, &f, terms_text, i.read_hits)?;
            let non_code = if j.non_code > 0 {
                format!("; {} non-code (css/scss/html) candidates set aside", j.non_code)
            } else {
                String::new()
            };
            if j.total == 0 {
                note = format!("searched (author-supplied): {terms_text} → 0 hits{non_code}");
                if j.read_given {
                    note.push_str("; no hits were found, so nothing was read");
                }
            } else {
                let top = &j.top;
                note = format!(
                    "searched: {terms_text} → {} candidates, top {} read and rejected: {}",
                    j.total,
                    top.len(),
                    capped(top, 20)
                );
                if let Some(t) = &j.judged_on {
                    note.push_str(&format!("; judged on the read set of the terms \"{t}\" (searched is a superset)"));
                }
                if j.total > j.cap {
                    note.push_str(&format!("; {} lower-ranked candidates not demanded", j.total - top.len()));
                    limitation_items.push(Limitation::new(
                        "read_capped",
                        format!("{} candidates, the top {} were read; the rest are not demanded", j.total, top.len()),
                    ));
                }
                note.push_str(&non_code);
                read_confirmed = true;
            }
        } else {
            // no manual terms: the graph decides (four channels); a hit anywhere refuses, zero everywhere is `derived`
            let f = store
                .features_of(&fam)?
                .into_iter()
                .find(|f| f.id == fid)
                .ok_or_else(|| AppError::NotFound(format!("no feature {}", i.feature)))?;
            let pin = super::absent::pinned_run(store, i.repo)?;
            let chk = super::absent::check_feature(store, i.repo, pin.0, &pin.1, &f, None, false)?;
            if !chk.absent {
                return Err(chk.refusal_error(store, &f)?);
            }
            note = chk.note();
            graph_absent = true;
        }
    }
    if let Some(n) = i.note.filter(|n| !n.is_empty()) {
        note = if note.is_empty() {
            n.to_string()
        } else {
            format!("{note} — {n}")
        };
    }

    // resolve every location before writing anything
    let ev = &i.evidence;
    let at_commit = check_at_commit(store, &repo, run, ev.at_commit, &mut limitation_items)?;
    let out_of_subset = match ev.out_of_subset {
        Some(p) => {
            let p = check_out_of_subset(Path::new(&repo.path), p)?;
            limitation_items.push(Limitation::new(
                "out_of_subset",
                format!("the evidence is upstream at {p}, outside the exported tree; the engine did not read it"),
            ));
            Some(p)
        }
        None => None,
    };
    struct Resolved {
        path: String,
        s: usize,
        e: usize,
        hash: String,
        node: Option<i64>,
    }
    fn res_keys(r: &[Resolved]) -> Vec<(String, i64, i64, String)> {
        r.iter().map(|x| (x.path.clone(), x.s as i64, x.e as i64, x.hash.clone())).collect()
    }
    let read_text = |rel: &str| -> Result<(String, String)> {
        let (norm, full) = safe_path(Path::new(&repo.path), rel)?;
        let text = match std::fs::read(&full) {
            Ok(b) => String::from_utf8(b)
                .map_err(|_| AppError::InvalidArguments(format!("{norm} is not UTF-8 text")))?,
            Err(er) => return Err(AppError::Io(full.clone(), er)),
        };
        Ok((norm, text))
    };
    let expects: Vec<super::citation::Pattern> = ev
        .expect
        .iter()
        .map(|x| {
            super::citation::Pattern::parse(x)
                .map_err(|m| AppError::InvalidArguments(format!("--expect {x:?}: the regex has {m}")))
        })
        .collect::<Result<_>>()?;
    // (path, start, end, file text, index of its --expect): every --at, then every --anchor resolved to its one line
    let mut spans: Vec<(String, usize, usize, String, Option<usize>)> = vec![];
    for (k, a) in i.at.iter().enumerate() {
        let (p, s, e) = parse_at(a)?;
        let (norm, text) = read_text(&p)?;
        let ex = (!expects.is_empty()).then_some(if expects.len() == 1 { 0 } else { k });
        spans.push((norm, s, e, text, ex));
    }
    for a in ev.anchors {
        let (p, pat) = super::citation::split_anchor(a).map_err(AppError::InvalidArguments)?;
        let (norm, text) = read_text(p)?;
        let n = super::citation::resolve_anchor(&norm, &text, &pat, ev.allow_comment)
            .map_err(AppError::InvalidArguments)?;
        spans.push((norm, n, n, text, None));
    }
    let mut imports_only: Vec<String> = vec![];
    let mut res: Vec<Resolved> = vec![];
    for (norm, s, e, text, ex) in spans {
        let Some(hash) = super::lines::span_hash(&text, s, e) else {
            return invalid(format!(
                "{norm}:{s}-{e}: lines out of range (need 1 ≤ start ≤ end ≤ file lines)"
            ));
        };
        let chk = super::citation::check_span(&norm, &text, s, e, ex.map(|k| &expects[k]), ev.allow_comment)
            .map_err(AppError::InvalidArguments)?;
        if chk.imports_only {
            imports_only.push(super::citation::span_label(&norm, s, e));
        }
        let node: Option<i64> = store
            .conn()
            .query_row(
                "SELECT id FROM node WHERE run_id = ?1 AND path = ?2 AND line_start <= ?3 AND line_end >= ?4 \
                 ORDER BY (line_end - line_start), id LIMIT 1",
                params![run.0, norm, s as i64, e as i64],
                |r| r.get(0),
            )
            .optional()?;
        res.push(Resolved {
            path: norm,
            s,
            e,
            hash,
            node,
        });
    }
    if !imports_only.is_empty() {
        limitation_items.push(Limitation::new(
            "citation_imports_only",
            format!(
                "{} cite(s) only import, use or package lines: the module is present, the feature's code is not shown",
                imports_only.join(", ")
            ),
        ));
    }
    // field-run ask 5: whether an entry point reaches the cited code, after every citation check has passed
    let reach = if matches!(i.verdict, "has" | "partial") && !ev.no_reach {
        let nodes: Vec<Option<i64>> = res.iter().map(|r| r.node).collect();
        Some(super::reach::compute(store, i.repo, &nodes, &mut limitation_items, reach_cache)?)
    } else {
        None
    };

    let confidence = if i.verdict == "unknown" {
        "unknown"
    } else if read_confirmed {
        "observed"
    } else if graph_absent || (out_of_subset.is_some() && res.is_empty()) {
        "inferred"
    } else {
        "derived"
    };
    if i.skip_equal {
        let quals = (at_commit.as_deref(), out_of_subset.as_deref(), reach.as_ref());
        if let Some(mut same) =
            equal_live_claim(store, i, rid.0, fid, run.0, confidence, &note, &fam, &res_keys(&res), quals)?
        {
            // the qualifiers this row computed are the stored claim's too (it equals it): the envelope carries them
            same.limitation_items = limitation_items;
            return Ok(same);
        }
    }
    let now = now_iso();
    let tx = store.conn();
    // a single claim takes the write lock before its first read (a deferred read-then-write upgrade skips SQLite's busy
    // handler and fails at once under a concurrent writer); inside a batch's transaction the write is a savepoint
    let own = tx.is_autocommit();
    tx.execute_batch(if own { "BEGIN IMMEDIATE" } else { "SAVEPOINT cg_claim" })?;
    let written: Result<(Option<i64>, i64, Vec<LocationOut>)> = (|| {
        let supersedes: Option<i64> = tx.query_row(
            "SELECT MAX(id) FROM claim WHERE repo_id = ?1 AND feature_id = ?2",
            params![rid.0, fid],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, note, created_at, supersedes_id, at_commit, out_of_subset, \
             reach, reach_path, reach_note) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                rid.0, fid, run.0, i.verdict, confidence, i.author, note, now, supersedes, at_commit, out_of_subset,
                reach.as_ref().map(|r| r.status.clone()),
                reach.as_ref().map(|r| r.path_json()),
                reach.as_ref().and_then(|r| r.note.clone())
            ],
        )?;
        let cid = tx.last_insert_rowid();
        let mut locations = vec![];
        for r in &res {
            tx.execute(
                "INSERT INTO location(claim_id, node_id, path, line_start, line_end, content_hash, resolved_at, stale) VALUES (?1,?2,?3,?4,?5,?6,?7,0)",
                params![cid, r.node, r.path, r.s as i64, r.e as i64, r.hash, now],
            )?;
            locations.push(LocationOut {
                id: tx.last_insert_rowid(),
                node_id: r.node,
                path: r.path.clone(),
                line_start: r.s as i64,
                line_end: r.e as i64,
                content_hash: r.hash.clone(),
                stale: false,
            });
        }
        Ok((supersedes, cid, locations))
    })();
    let (supersedes, cid, locations) = match written {
        Ok(x) => {
            tx.execute_batch(if own { "COMMIT" } else { "RELEASE cg_claim" })?;
            x
        }
        Err(e) => {
            let _ = tx.execute_batch(if own { "ROLLBACK" } else { "ROLLBACK TO cg_claim; RELEASE cg_claim" });
            return Err(e);
        }
    };
    Ok(ClaimOut {
        id: cid,
        repo: i.repo.into(),
        family: fam,
        feature: i.feature.into(),
        run_id: run.0,
        verdict: i.verdict.into(),
        confidence: confidence.into(),
        author: i.author.into(),
        created_at: now,
        supersedes_id: supersedes,
        note,
        locations,
        at_commit,
        out_of_subset,
        reach,
        skipped: false,
        limitation_items,
    })
}

/// `--at-commit`: 7 to 40 hex digits that agree (one a prefix of the other) with the commit the build read: the repo's
/// `source_commit` when it is set (an export's upstream commit, which its local head is not), else the git head the
/// latest done run read, else the head `add` saw (a run from before the run's head was recorded). With none known it is
/// recorded as given, under an `at_commit_unverified` limitation.
fn check_at_commit(
    store: &Store,
    repo: &crate::store::RepoRow,
    run: crate::domain::RunId,
    at: Option<&str>,
    lims: &mut Vec<Limitation>,
) -> Result<Option<String>> {
    let Some(at) = at else { return Ok(None) };
    let sha = at.trim().to_ascii_lowercase();
    if !(7..=40).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return invalid(format!("--at-commit {at}: expected a commit id of 7 to 40 hex digits"));
    }
    let nonempty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    let known = nonempty(repo.source_commit.clone())
        .or(nonempty(store.run_head(run)?))
        .or_else(|| nonempty(repo.head.clone()));
    match known {
        Some(k) => {
            let k = k.trim().to_ascii_lowercase();
            if !(k.starts_with(&sha) || sha.starts_with(&k)) {
                return invalid(format!(
                    "--at-commit {at} disagrees with the commit the build of {} read ({k}); cite the lines at that commit, or re-export and rebuild at {at}",
                    repo.name
                ));
            }
        }
        None => lims.push(Limitation::new(
            "at_commit_unverified",
            format!("{} records no commit (no git head, no source commit), so --at-commit {sha} was recorded as given", repo.name),
        )),
    }
    Ok(Some(sha))
}

/// `--out-of-subset <path[:start[-end]]>`: a clone-relative path with no `..` that the exported tree does not
/// contain (a file that is there is cited with `--at`). Returned normalised, with its line suffix.
fn check_out_of_subset(clone: &Path, given: &str) -> Result<String> {
    let (p, lines) = match given.rsplit_once(':') {
        Some((p, l)) if !l.is_empty() && l.chars().all(|c| c.is_ascii_digit() || c == '-') => {
            parse_at(given).map_err(|e| match e {
                AppError::InvalidArguments(m) => {
                    AppError::InvalidArguments(m.replacen("--at ", "--out-of-subset ", 1))
                }
                e => e,
            })?;
            (p, Some(l))
        }
        Some((_, l)) => {
            return invalid(format!("--out-of-subset {given}: {l:?} is not a line number or range"));
        }
        None => (given, None),
    };
    let mut parts: Vec<String> = vec![];
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return invalid(format!("--out-of-subset {given:?}: an upstream path relative to the repo root, with no '..'")),
        }
    }
    if parts.is_empty() {
        return invalid("--out-of-subset needs the upstream path of the evidence");
    }
    let norm = parts.join("/");
    // walk the path inside the clone without following a link: a symlink (dangling, or pointing out of the clone) is
    // refused, and nothing outside the clone is ever touched
    let mut at = clone.to_path_buf();
    for (k, part) in parts.iter().enumerate() {
        at.push(part);
        let so_far = parts[..=k].join("/");
        let meta = match std::fs::symlink_metadata(&at) {
            Ok(m) => m,
            Err(_) => break, // not in the exported tree from here on
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            return invalid(format!(
                "--out-of-subset {norm}: {so_far} is a symlink in the exported tree; name the upstream path, not a link"
            ));
        }
        let last = k + 1 == parts.len();
        if ft.is_file() && last {
            return invalid(format!(
                "--out-of-subset {norm}: that file is in the exported tree; cite it with --at {norm}:<start>-<end>"
            ));
        }
        if ft.is_dir() && last {
            return invalid(format!(
                "--out-of-subset {norm}: that directory is in the exported tree; cite its files with --at"
            ));
        }
        if !ft.is_dir() {
            break; // a file (or other entry) in the middle: the rest of the path cannot be in the tree
        }
    }
    Ok(match lines {
        Some(l) => format!("{norm}:{l}"),
        None => norm,
    })
}

/// The live claim for (repo, feature) when it equals the one about to be written: same run, verdict, confidence,
/// author, note and locations (path, span and content hash, in order, none stale). Returned as it is stored, marked
/// `skipped`; a cited line whose text changed, or a location `check` flagged stale, is a new claim.
#[allow(clippy::too_many_arguments)]
fn equal_live_claim(
    store: &Store,
    i: &ClaimInput,
    repo_id: i64,
    feature_id: i64,
    run_id: i64,
    confidence: &str,
    note: &str,
    family: &str,
    keys: &[(String, i64, i64, String)],
    quals: (Option<&str>, Option<&str>, Option<&super::reach::Reach>),
) -> Result<Option<ClaimOut>> {
    let c = store.conn();
    let live = c
        .query_row(
            "SELECT id, run_id, verdict, confidence, author, note, created_at, supersedes_id, at_commit, out_of_subset, \
             reach, reach_path, reach_note FROM claim \
             WHERE id = (SELECT MAX(id) FROM claim WHERE repo_id = ?1 AND feature_id = ?2)",
            params![repo_id, feature_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    super::reach::Reach::from_columns(r.get(10)?, r.get(11)?, r.get(12)?),
                ))
            },
        )
        .optional()?;
    let Some((id, lrun, verdict, lconf, author, lnote, created_at, supersedes_id, lcommit, lsubset, lreach)) = live else {
        return Ok(None);
    };
    if lrun != run_id
        || verdict != i.verdict
        || lconf != confidence
        || author != i.author
        || lnote != note
        || lcommit.as_deref() != quals.0
        || lsubset.as_deref() != quals.1
        || lreach.as_ref() != quals.2
    {
        return Ok(None);
    }
    let mut st = c.prepare(
        "SELECT id, node_id, path, line_start, line_end, content_hash, stale FROM location WHERE claim_id = ?1 ORDER BY id",
    )?;
    let locations: Vec<LocationOut> = st
        .query_map(params![id], |r| {
            Ok(LocationOut {
                id: r.get(0)?,
                node_id: r.get(1)?,
                path: r.get(2)?,
                line_start: r.get(3)?,
                line_end: r.get(4)?,
                content_hash: r.get(5)?,
                stale: r.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let same = locations.len() == keys.len()
        && locations
            .iter()
            .zip(keys)
            .all(|(l, k)| l.path == k.0 && l.line_start == k.1 && l.line_end == k.2 && l.content_hash == k.3 && !l.stale);
    if !same {
        return Ok(None);
    }
    Ok(Some(ClaimOut {
        id,
        repo: i.repo.into(),
        family: family.into(),
        feature: i.feature.into(),
        run_id: lrun,
        verdict,
        confidence: lconf,
        author,
        created_at,
        supersedes_id,
        note: lnote,
        locations,
        at_commit: lcommit,
        out_of_subset: lsubset,
        reach: lreach,
        skipped: true,
        limitation_items: vec![],
    }))
}

#[derive(Debug, Serialize, Clone)]
pub struct BatchError {
    pub code: &'static str,
    pub message: String,
    /// `AppError::data` (a `none_refused` row's `NoneRefusal`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// One input line's outcome: `data` on success, `error` otherwise. `line` is 1-based in the input.
#[derive(Debug, Serialize, Clone)]
pub struct BatchRow<T> {
    pub line: usize,
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<BatchError>,
}

impl<T> BatchRow<T> {
    pub fn from_result(line: usize, r: Result<T>) -> Self {
        match r {
            Ok(d) => BatchRow { line, ok: true, data: Some(d), error: None },
            Err(e) => {
                let mut row = BatchRow::fail(line, e.code(), e.to_string());
                if let Some(b) = row.error.as_mut() {
                    b.data = e.data();
                }
                row
            }
        }
    }
    pub fn fail(line: usize, code: &'static str, message: String) -> Self {
        BatchRow { line, ok: false, data: None, error: Some(BatchError { code, message, data: None }) }
    }
}

/// What a batch did: rows seen (blank lines excluded), ok, skipped (ok rows that wrote nothing), failed. Rows are durable only once the transaction commits,
/// which is when this value is returned.
#[derive(Debug, Serialize, Clone, Copy, Default)]
pub struct BatchSummary {
    pub rows: usize,
    pub ok: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Run `each` over the non-blank lines of `lines` on one open store inside one transaction, handing every row to
/// `on_row` as soon as it is decided (so output streams). A line that is not a JSON object of type `L`, or whose `each`
/// fails, is a failed row and writes nothing; the rest proceed. A line that is not UTF-8 is a failed row; any other read
/// error (a directory, a closed pipe) ends the batch after its failed row, since `lines()` would return it forever.
/// A UTF-8 byte-order mark on the first line is ignored.
pub fn run_batch<L: serde::de::DeserializeOwned, T>(
    root: &Path,
    lines: impl std::io::BufRead,
    mut each: impl FnMut(&Store, L) -> Result<T>,
    skipped_of: impl Fn(&T) -> bool,
    mut on_row: impl FnMut(BatchRow<T>),
) -> Result<BatchSummary> {
    let store = Store::open(root)?;
    store.conn().execute_batch("BEGIN IMMEDIATE")?;
    let mut sum = BatchSummary::default();
    for (n, line) in lines.lines().enumerate() {
        let line_no = n + 1;
        let text = match line {
            Ok(t) => t,
            Err(e) => {
                sum.rows += 1;
                sum.failed += 1;
                let persistent = e.kind() != std::io::ErrorKind::InvalidData;
                on_row(BatchRow::fail(line_no, "io_error", format!("reading line {line_no}: {e}")));
                if persistent {
                    break;
                }
                continue;
            }
        };
        let text = if line_no == 1 { text.trim_start_matches('\u{feff}') } else { text.as_str() };
        if text.trim().is_empty() {
            continue;
        }
        let row = match serde_json::from_str::<L>(text) {
            Ok(l) => BatchRow::from_result(line_no, each(&store, l)),
            Err(e) => BatchRow::fail(line_no, "invalid_arguments", format!("line {line_no}: not a valid row: {e}")),
        };
        sum.rows += 1;
        if row.ok {
            sum.ok += 1;
            if row.data.as_ref().is_some_and(&skipped_of) {
                sum.skipped += 1;
            }
        } else {
            sum.failed += 1;
        }
        on_row(row);
    }
    store.conn().execute_batch("COMMIT")?;
    Ok(sum)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimLine {
    pub repo: String,
    pub feature: String,
    pub verdict: String,
    pub author: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub at: Vec<String>,
    #[serde(default)]
    pub searched: Option<String>,
    #[serde(default)]
    pub hits_read: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub at_commit: Option<String>,
    #[serde(default)]
    pub anchor: Vec<String>,
    #[serde(default)]
    pub expect: Vec<String>,
    #[serde(default)]
    pub allow_comment: bool,
    #[serde(default)]
    pub out_of_subset: Option<String>,
    #[serde(default)]
    pub no_reach: bool,
}

/// `claim` for a JSONL stream: one store open, one transaction, one `BatchRow` per non-blank line, each handed to
/// `on_row` as it is decided.
pub fn claim_batch(
    root: &Path,
    lines: impl std::io::BufRead,
    on_row: impl FnMut(BatchRow<ClaimOut>),
) -> Result<BatchSummary> {
    claim_batch_with(root, lines, true, on_row)
}

/// `claim_batch` with the per-run reach cache on or off (off rebuilds the reach state for every row, as a single
/// `claim` does): the equivalence test runs both and compares every row.
#[doc(hidden)]
pub fn claim_batch_with(
    root: &Path,
    lines: impl std::io::BufRead,
    reach_cache: bool,
    on_row: impl FnMut(BatchRow<ClaimOut>),
) -> Result<BatchSummary> {
    let mut cache = super::reach::ReachCache::new(reach_cache);
    run_batch(root, lines, |store, l: ClaimLine| {
        claim_on_with(
            store,
            &ClaimInput {
                repo: &l.repo,
                feature: &l.feature,
                family: l.family.as_deref(),
                verdict: &l.verdict,
                author: &l.author,
                at: &l.at,
                searched: l.searched.as_deref(),
                read_hits: &l.hits_read,
                note: l.note.as_deref(),
                skip_equal: true,
                evidence: Evidence {
                    at_commit: l.at_commit.as_deref(),
                    anchors: &l.anchor,
                    expect: &l.expect,
                    allow_comment: l.allow_comment,
                    out_of_subset: l.out_of_subset.as_deref(),
                    no_reach: l.no_reach,
                },
            },
            &mut cache,
        )
    }, |o: &ClaimOut| o.skipped, on_row)
}

/// Latest claim per (repo, feature) by default; every row with its supersede chain when `history`.
pub fn claims(
    root: &Path,
    repo: Option<&str>,
    family: Option<&str>,
    feature: Option<&str>,
    history: bool,
) -> Result<Vec<ClaimOut>> {
    let store = Store::open_existing(root)?;
    claims_in(&store, repo, family, feature, history)
}
/// As `claims`, on a store already open (a read-only one included).
pub fn claims_in(store: &Store, repo: Option<&str>, family: Option<&str>, feature: Option<&str>, history: bool) -> Result<Vec<ClaimOut>> {
    let c = store.conn();
    let mut st = c.prepare(
        "SELECT c.id, r.name, fa.name, f.slug, c.run_id, c.verdict, c.confidence, c.author, c.created_at, c.supersedes_id, c.note, c.at_commit, c.out_of_subset, \
         c.reach, c.reach_path, c.reach_note FROM claim c JOIN repo r ON r.id = c.repo_id JOIN feature f ON f.id = c.feature_id JOIN family fa ON fa.id = f.family_id \
         WHERE (?1 IS NULL OR r.name = ?1) AND (?2 IS NULL OR fa.name = ?2) AND (?3 IS NULL OR f.slug = ?3) \
         AND (?4 = 1 OR c.id = (SELECT MAX(id) FROM claim x WHERE x.repo_id = c.repo_id AND x.feature_id = c.feature_id)) \
         ORDER BY r.name, fa.name, f.slug, c.id",
    )?;
    let mut out = st
        .query_map(params![repo, family, feature, history as i64], |r| {
            Ok(ClaimOut {
                id: r.get(0)?,
                repo: r.get(1)?,
                family: r.get(2)?,
                feature: r.get(3)?,
                run_id: r.get(4)?,
                verdict: r.get(5)?,
                confidence: r.get(6)?,
                author: r.get(7)?,
                created_at: r.get(8)?,
                supersedes_id: r.get(9)?,
                note: r.get(10)?,
                locations: vec![],
                at_commit: r.get(11)?,
                out_of_subset: r.get(12)?,
                reach: super::reach::Reach::from_columns(r.get(13)?, r.get(14)?, r.get(15)?),
                skipped: false,
                limitation_items: vec![],
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut ls = c.prepare("SELECT id, node_id, path, line_start, line_end, content_hash, stale FROM location WHERE claim_id = ?1 ORDER BY id")?;
    for cl in &mut out {
        cl.locations = ls
            .query_map(params![cl.id], |r| {
                Ok(LocationOut {
                    id: r.get(0)?,
                    node_id: r.get(1)?,
                    path: r.get(2)?,
                    line_start: r.get(3)?,
                    line_end: r.get(4)?,
                    content_hash: r.get(5)?,
                    stale: r.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        if cl.locations.iter().any(|l| l.stale) {
            cl.confidence = "unknown".into();
        }
    }
    Ok(out)
}
