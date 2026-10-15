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
    if matches!(i.verdict, "has" | "partial") && i.at.is_empty() {
        return invalid(format!(
            "a {} claim needs at least one --at <path>:<start>-<end>",
            i.verdict
        ));
    }
    Ok(())
}

/// The body of `claim` on an already open store; inside a batch transaction the write is a savepoint.
pub fn claim_on(store: &Store, i: &ClaimInput) -> Result<ClaimOut> {
    check_input(i)?;
    let repo = store
        .repo_by_name(i.repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {}", i.repo)))?;
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
            let rs = super::absent::read_set(store, i.repo, pin.0, &pin.1, &f, Some(terms_text))?;
            let read: Vec<String> = i
                .read_hits
                .iter()
                .flat_map(|r| r.split(|c: char| c.is_whitespace() || c == ','))
                .filter(|r| !r.is_empty())
                .map(String::from)
                .collect();
            if rs.total == 0 {
                note = format!("searched (author-supplied): {terms_text} → 0 hits");
                if !read.is_empty() {
                    note.push_str("; no hits were found, so nothing was read");
                }
            } else if read.is_empty() {
                let sample: Vec<&str> = rs.candidates.iter().take(5).map(|h| h.qualified_name.as_str()).collect();
                return invalid(format!(
                    "not none: the search for \"{terms_text}\" finds {} candidate(s) in {} (e.g. {}); read them (absent-check --searched) and pass --read-hits",
                    rs.total,
                    i.repo,
                    sample.join(", ")
                ));
            } else {
                let top: Vec<&str> = rs.candidates.iter().map(|h| h.qualified_name.as_str()).collect();
                let unread: Vec<&str> = top.iter().copied().filter(|n| !read.iter().any(|r| r == n)).collect();
                if !unread.is_empty() {
                    return invalid(format!(
                        "not none: {} of the top {} candidates were not read: {}; run absent-check --searched \"{terms_text}\" for the list, or claim has/partial",
                        unread.len(),
                        top.len(),
                        unread.join(", ")
                    ));
                }
                let strangers: Vec<&str> =
                    read.iter().map(String::as_str).filter(|r| !top.contains(r)).collect();
                if !strangers.is_empty() {
                    let strangers: Vec<&str> = strangers.into_iter().filter(|r| !rs.names.contains(*r)).collect();
                    if !strangers.is_empty() {
                        return invalid(format!(
                            "--read-hits names symbols the search does not find: {}",
                            strangers.join(", ")
                        ));
                    }
                }
                note = format!(
                    "searched: {terms_text} → {} candidates, top {} read and rejected: {}",
                    rs.total,
                    top.len(),
                    capped(&top, 20)
                );
                if rs.total > rs.cap {
                    note.push_str(&format!("; {} lower-ranked candidates not demanded", rs.total - top.len()));
                    limitation_items.push(Limitation::new(
                        "read_capped",
                        format!("{} candidates, the top {} were read; the rest are not demanded", rs.total, top.len()),
                    ));
                }
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
            let chk = super::absent::check_feature(store, i.repo, pin.0, &pin.1, &f, None)?;
            if !chk.absent {
                return invalid(chk.refusal());
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
    struct Resolved {
        path: String,
        s: usize,
        e: usize,
        hash: String,
        node: Option<i64>,
    }
    fn res_keys(r: &[Resolved]) -> Vec<(String, i64, i64)> {
        r.iter().map(|x| (x.path.clone(), x.s as i64, x.e as i64)).collect()
    }
    let mut res: Vec<Resolved> = vec![];
    for a in i.at {
        let (p, s, e) = parse_at(a)?;
        let (norm, full) = safe_path(Path::new(&repo.path), &p)?;
        let text = match std::fs::read(&full) {
            Ok(b) => String::from_utf8(b)
                .map_err(|_| AppError::InvalidArguments(format!("{norm} is not UTF-8 text")))?,
            Err(er) => return Err(AppError::Io(full.clone(), er)),
        };
        let Some(hash) = super::lines::span_hash(&text, s, e) else {
            return invalid(format!(
                "{norm}:{s}-{e}: lines out of range (need 1 ≤ start ≤ end ≤ file lines)"
            ));
        };
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

    let confidence = if i.verdict == "unknown" {
        "unknown"
    } else if read_confirmed {
        "observed"
    } else if graph_absent {
        "inferred"
    } else {
        "derived"
    };
    if i.skip_equal {
        if let Some(same) = equal_live_claim(store, i, rid.0, fid, run.0, confidence, &note, &fam, &res_keys(&res))? {
            return Ok(same);
        }
    }
    let now = now_iso();
    let tx = store.conn();
    tx.execute_batch("SAVEPOINT cg_claim")?;
    let written: Result<(Option<i64>, i64, Vec<LocationOut>)> = (|| {
        let supersedes: Option<i64> = tx.query_row(
            "SELECT MAX(id) FROM claim WHERE repo_id = ?1 AND feature_id = ?2",
            params![rid.0, fid],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, note, created_at, supersedes_id) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![rid.0, fid, run.0, i.verdict, confidence, i.author, note, now, supersedes],
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
            tx.execute_batch("RELEASE cg_claim")?;
            x
        }
        Err(e) => {
            let _ = tx.execute_batch("ROLLBACK TO cg_claim; RELEASE cg_claim");
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
        skipped: false,
        limitation_items,
    })
}

/// The live claim for (repo, feature) when it equals the one about to be written: same run, verdict, confidence,
/// author, note and locations (path and span, in order). Returned as it is stored, marked `skipped`.
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
    keys: &[(String, i64, i64)],
) -> Result<Option<ClaimOut>> {
    let c = store.conn();
    let live = c
        .query_row(
            "SELECT id, run_id, verdict, confidence, author, note, created_at, supersedes_id FROM claim \
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
                ))
            },
        )
        .optional()?;
    let Some((id, lrun, verdict, lconf, author, lnote, created_at, supersedes_id)) = live else {
        return Ok(None);
    };
    if lrun != run_id
        || verdict != i.verdict
        || lconf != confidence
        || author != i.author
        || lnote != note
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
            .all(|(l, k)| l.path == k.0 && l.line_start == k.1 && l.line_end == k.2);
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
        skipped: true,
        limitation_items: vec![],
    }))
}

#[derive(Debug, Serialize, Clone)]
pub struct BatchError {
    pub code: &'static str,
    pub message: String,
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
            Err(e) => BatchRow::fail(line, e.code(), e.to_string()),
        }
    }
    pub fn fail(line: usize, code: &'static str, message: String) -> Self {
        BatchRow { line, ok: false, data: None, error: Some(BatchError { code, message }) }
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
}

/// `claim` for a JSONL stream: one store open, one transaction, one `BatchRow` per non-blank line, each handed to
/// `on_row` as it is decided.
pub fn claim_batch(
    root: &Path,
    lines: impl std::io::BufRead,
    on_row: impl FnMut(BatchRow<ClaimOut>),
) -> Result<BatchSummary> {
    run_batch(root, lines, |store, l: ClaimLine| {
        claim_on(
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
            },
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
    let c = store.conn();
    let mut st = c.prepare(
        "SELECT c.id, r.name, fa.name, f.slug, c.run_id, c.verdict, c.confidence, c.author, c.created_at, c.supersedes_id, c.note \
         FROM claim c JOIN repo r ON r.id = c.repo_id JOIN feature f ON f.id = c.feature_id JOIN family fa ON fa.id = f.family_id \
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
