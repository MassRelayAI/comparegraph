//! `absent-check`: is a feature absent from a clone's graph? Four channels over the latest done run, each with a hit
//! count and up to five hits. `absent` is true only when EVERY channel has zero hits. Nothing here is a model: the
//! terms, the channels and the counts are the whole verdict and all of them are written into the claim note.
//!
//! Channels:
//! - `symbols`: FTS prefix match (the same as `map`), OR an exact lowercase name, OR a STEM match against the tokens of
//!   a node's name: a term (or its suffix-stripped stem) and a token match when either is a prefix of the other and the
//!   shorter has >= 4 chars (`extract`~`extraction`, `config`~`configuration`, `cach`~`cache`).
//! - `paths`: a stem match against path tokens, OR a lowercase SUBSTRING of a node's path for any term or path-segment form (`retry-backoff`,
//!   `retry_backoff`, `retrybackoff` of the slug tail); it sees what token-prefix FTS cannot (a term inside a token).
//!   A directory name that holds a term refuses every file under it (over-refusal, documented).
//! - `communities`: a community whose label tokens or keywords equal a term (the rule `map` uses).
//! - `kinds`: node kinds the feature names. A feature is said to name a kind by a word of its group, slug tail or title
//!   (never the definition): `route|routes|endpoint|endpoints` -> kind `route`; `test|tests|testing` -> kind `test`;
//!   the words `mcp` + `tool|tools` (an MCP tool) -> kind `route` (a handler registered by name) OR any function,
//!   method or class whose name or path carries `mcp` (the graph has no MCP-tool kind; this is the implementer set).
//!   A feature naming no kind has an empty kinds channel (count 0). The kinds channel is the ONLY place a named kind is
//!   used: the symbol channel always counts every node whose name carries a term (the reference provider emits no `route`
//!   nodes, so scoping by kind would hide `handle_http_request`; withdrawn after review).
//!
//! Family words: a term that appears in the term set of at least half of the family's features, and of at least 4 of
//! them (`agents` in a pack whose every row is `agents.*`, `graph` in `code-graph-tools`), names the family, not the
//! feature. It is dropped from all four channels and listed in the note as `common=<terms>`; a hit on a remaining term
//! refuses as before, and zero hits with a family word set aside also refuses, naming the uncounted words: the graph then
//! proves less, and the absence is the author's to record with `--searched`. When every term is a family word the check
//! refuses (a "pass --searched" error). An absence is recorded from the graph only when no term was set aside. (A clone-frequency rule, "a term matching > max(200, 2%) nodes is common", was withdrawn: it dropped
//! `cron` from `scheduling.cron-tasks` on a clone with 300 `cron_job_<n>` functions.)
//!
//! Coverage limit: names, paths, community terms and kinds only. Not strings, flags, config keys, abbreviations (`cfg`)
//! or synonyms (`memoize`), so a graph-absent verdict is `inferred`, never `derived`. The check reads the latest done
//! run of provider `reference` only.
use super::communities::tokens;
use super::map::feature_terms;
use super::{AppError, Result};
use crate::store::{FeatureRow, Store};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct AbsentHit {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line_start: i64,
    /// terms (or kinds) that made it a hit; for a community hit, the matched tokens
    pub matched: Vec<String>,
}
#[derive(Debug, Serialize, Clone, Default)]
pub struct Channel {
    pub count: usize,
    pub hits: Vec<AbsentHit>,
}
#[derive(Debug, Serialize, Clone)]
pub struct Channels {
    pub symbols: Channel,
    pub paths: Channel,
    pub communities: Channel,
    pub kinds: Channel,
}
/// One node a `none` must be read against: why it is a candidate and how it ranks.
#[derive(Debug, Serialize, Clone)]
pub struct Candidate {
    pub qualified_name: String,
    pub kind: String,
    pub path: String,
    pub line_start: i64,
    pub score: u32,
    pub why: Vec<String>,
}
/// The ranked candidates of a feature (its channels, plus the author's search when given): `total` is the union size,
/// `candidates` the top `cap` of it.
#[derive(Debug, Serialize, Clone)]
pub struct ReadSet {
    pub cap: usize,
    pub total: usize,
    pub candidates: Vec<Candidate>,
    /// every qualified name of the union, at any rank (the claim's stranger check)
    #[serde(skip)]
    pub(crate) names: BTreeSet<String>,
}
/// How many ranked candidates a `none` must name as read.
pub(crate) const READ_CAP: usize = 10;
/// The most FTS hits of the author's search that enter the read set.
const FTS_FETCH: usize = 1000;

#[derive(Debug, Serialize, Clone)]
pub struct AbsentCheck {
    pub repo: String,
    pub family: String,
    pub feature: String,
    pub run_id: i64,
    pub provider: String,
    pub terms: Vec<String>,
    /// feature terms dropped from every channel because they are family words (see the module header)
    pub common: Vec<String>,
    pub stems: Vec<String>,
    pub path_forms: Vec<String>,
    pub kinds_named: Vec<String>,
    pub channels: Channels,
    pub absent: bool,
    /// the `--searched` terms the read set and `absent` were also computed over
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searched: Option<String>,
    /// the ranked, capped candidates a `none` must read (see `ReadSet`)
    pub read_set: ReadSet,
    /// Why `absent` has its value: `hits` (some channel, or the `--searched` search, found something), `family words set aside: <words>` (every
    /// channel is at zero but the family-word rule dropped terms, so absence is not concluded), or `absent`.
    pub reason: String,
    /// `inferred` when absent (the graph cannot see strings, flags or abbreviations), `observed` when there are hits
    pub evidence_confidence: &'static str,
}

impl AbsentCheck {
    pub fn note(&self) -> String {
        let c = &self.channels;
        let common = if self.common.is_empty() { String::new() } else { format!("; common={}", self.common.join(",")) };
        format!(
            "graph-absent (names, paths, community terms, kinds; not strings, flags or config keys): terms={}{}; stems={}; channels=symbols:{},paths:{},communities:{},kinds:{}; run={} provider={}",
            self.terms.join(","),
            common,
            self.stems.join(","),
            c.symbols.count,
            c.paths.count,
            c.communities.count,
            c.kinds.count,
            self.run_id,
            self.provider
        )
    }
    pub fn total(&self) -> usize {
        let c = &self.channels;
        c.symbols.count + c.paths.count + c.communities.count + c.kinds.count
    }
    /// Why `claim none` is refused: counts per channel and the first hits; or, with zero hits but a family word set
    /// aside, the words that were not counted.
    pub fn refusal(&self) -> String {
        if self.total() == 0 && !self.common.is_empty() {
            return format!(
                "no hit for {} in {}; the family word(s) {} were not counted, so the graph proves less: read the clone, \
                 or pass --searched with the terms you checked",
                self.terms.join(", "),
                self.repo,
                self.common.join(", ")
            );
        }
        let c = &self.channels;
        let mut sample: Vec<String> = vec![];
        for ch in [&c.symbols, &c.paths, &c.communities, &c.kinds] {
            for h in ch.hits.iter().take(2) {
                let s = format!("{} ({}:{})", h.name, h.path, h.line_start);
                if !sample.contains(&s) {
                    sample.push(s);
                }
            }
        }
        format!(
            "the graph has {} candidate(s) for {} in {} (symbols:{}, paths:{}, communities:{}, kinds:{}; e.g. {}); \
             read them, then claim has/partial with a citation, or pass --searched \"<terms>\" to record a manual absence",
            self.total(),
            self.feature,
            self.repo,
            c.symbols.count,
            c.paths.count,
            c.communities.count,
            c.kinds.count,
            sample.join(", ")
        )
    }
}

fn words(f: &FeatureRow) -> Vec<String> {
    let slug = f.slug.rsplit('.').next().unwrap_or(&f.slug);
    format!("{} {} {}", f.grp, slug, f.title)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// The kinds a feature names, as `(label, node kinds, also_mcp)`; see the module header for the mapping.
fn kinds_named(f: &FeatureRow) -> (Vec<String>, Vec<&'static str>, bool) {
    let w = words(f);
    let has = |xs: &[&str]| w.iter().any(|x| xs.contains(&x.as_str()));
    let mut names = vec![];
    let mut kinds: Vec<&'static str> = vec![];
    let mut mcp = false;
    if has(&["route", "routes", "endpoint", "endpoints"]) {
        names.push("route".to_string());
        kinds.push("route");
    }
    if has(&["test", "tests", "testing"]) {
        names.push("test".to_string());
        kinds.push("test");
    }
    if has(&["mcp"]) && has(&["tool", "tools"]) {
        names.push("mcp-tool".to_string());
        if !kinds.contains(&"route") {
            kinds.push("route");
        }
        mcp = true;
    }
    (names, kinds, mcp)
}

fn path_forms(f: &FeatureRow, terms: &[String]) -> Vec<String> {
    let slug = f.slug.rsplit('.').next().unwrap_or(&f.slug).to_lowercase();
    let mut set: BTreeSet<String> = terms.iter().cloned().collect();
    let sep = |c: char| !c.is_alphanumeric();
    let parts: Vec<&str> = slug.split(sep).filter(|p| !p.is_empty()).collect();
    if parts.len() > 1 {
        set.insert(parts.join("-"));
        set.insert(parts.join("_"));
        set.insert(parts.concat());
    }
    set.into_iter().collect()
}


fn hit_row(r: &rusqlite::Row, matched: Vec<String>) -> rusqlite::Result<AbsentHit> {
    Ok(AbsentHit {
        name: r.get(0)?,
        kind: r.get(1)?,
        path: r.get(2)?,
        line_start: r.get(3)?,
        matched,
    })
}

/// Suffix-stripped forms of the terms (`extraction` -> `extract`, `caching` -> `cach`), kept when >= 4 chars and new.
/// Returned as `(stem, term it came from)`.
fn stems_of(terms: &[String]) -> Vec<(String, String)> {
    const SUFFIXES: [&str; 11] = [
        "ations", "ation", "ments", "ment", "ions", "ion", "ing", "ers", "er", "ed", "s",
    ];
    let mut out: Vec<(String, String)> = vec![];
    for t in terms {
        for suf in SUFFIXES {
            if let Some(stem) = t.strip_suffix(suf) {
                if stem.chars().count() >= 4 && !terms.contains(&stem.to_string()) {
                    out.push((stem.to_string(), t.clone()));
                }
                break;
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A term (or stem) and a token match when equal, or when either is a prefix of the other and the shorter has >= 4 chars.
fn stem_match(a: &str, b: &str) -> bool {
    a == b
        || (a.chars().count().min(b.chars().count()) >= 4 && (a.starts_with(b) || b.starts_with(a)))
}

/// The terms (not stems) that match any of the tokens, stems counted for the term they come from.
fn matched_terms(toks: &[String], terms: &[String], stems: &[(String, String)]) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for t in terms {
        if toks.iter().any(|k| stem_match(t, k)) {
            out.insert(t.clone());
        }
    }
    for (st, t) in stems {
        if toks.iter().any(|k| stem_match(st, k)) {
            out.insert(t.clone());
        }
    }
    out.into_iter().collect()
}

fn top(c: &Connection, ids: &BTreeSet<i64>, matched: impl Fn(&AbsentHit) -> Vec<String>) -> Result<Channel> {
    let count = ids.len();
    let mut rows = vec![];
    let mut st = c.prepare("SELECT qualified_name, kind, path, line_start FROM node WHERE id = ?1")?;
    for id in ids {
        rows.push(st.query_row(params![id], |r| hit_row(r, vec![]))?);
    }
    rows.sort_by(|a, b| (&a.name, &a.path, a.line_start).cmp(&(&b.name, &b.path, b.line_start)));
    rows.truncate(5);
    for h in &mut rows {
        h.matched = matched(h);
    }
    Ok(Channel { count, hits: rows })
}

fn hit_tokens(h: &AbsentHit) -> Vec<String> {
    let mut t = tokens(&h.name);
    t.extend(tokens(&h.path));
    t
}

/// The full-text part of the symbol channel. See `search::fts_plan_scans_fts_first` for the join order this must keep.
pub(crate) const SYMBOLS_FTS_SQL: &str =
    "SELECT n.id FROM node_fts CROSS JOIN node n ON n.id = node_fts.rowid WHERE node_fts MATCH ?1 AND n.run_id = ?2";

/// One node of the run as the symbol channel reads it: id, lowercase name, name tokens.
struct NameRow {
    id: i64,
    lower: String,
    toks: Vec<String>,
}

fn name_rows(c: &Connection, run: i64) -> Result<Vec<NameRow>> {
    let mut st = c.prepare("SELECT id, name FROM node WHERE run_id = ?1")?;
    let rows = st
        .query_map(params![run], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| NameRow { id, lower: name.to_lowercase(), toks: tokens(&name) })
        .collect())
}

/// The node ids the symbol channel matches for `terms` and `stems`: FTS prefix, exact lowercase name, stem match against the
/// name tokens. Every node counts whatever its kind.
fn symbol_ids(
    c: &Connection,
    run: i64,
    rows: &[NameRow],
    terms: &[String],
    stems: &[(String, String)],
) -> Result<BTreeSet<i64>> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    if let Some(m) = super::search::fts_query_or(&terms.join(" ")) {
        let mut st = c.prepare(SYMBOLS_FTS_SQL)?;
        for r in st.query_map(params![m, run], |r| r.get::<_, i64>(0))? {
            ids.insert(r?);
        }
    }
    let all: Vec<String> = terms.iter().cloned().chain(stems.iter().map(|s| s.0.clone())).collect();
    for n in rows {
        if all.iter().any(|t| *t == n.lower || n.toks.iter().any(|k| stem_match(t, k))) {
            ids.insert(n.id);
        }
    }
    Ok(ids)
}

/// The terms of `f` that are family words: present in the term set of at least half of the family's live features and of
/// at least 4 of them. Needs no scan of the run.
pub(crate) fn family_words(store: &Store, f: &FeatureRow, terms: &[String]) -> Result<Vec<String>> {
    let sets: Vec<BTreeSet<String>> = store
        .features_of(&f.family)?
        .iter()
        .filter(|g| g.tier != "retired")
        .map(|g| feature_terms(g).into_iter().collect())
        .collect();
    let n = sets.len();
    Ok(terms
        .iter()
        .filter(|t| {
            let k = sets.iter().filter(|s| s.contains(*t)).count();
            k >= 4 && k * 2 >= n
        })
        .cloned()
        .collect())
}

fn symbols(
    c: &Connection,
    run: i64,
    rows: &[NameRow],
    terms: &[String],
    stems: &[(String, String)],
) -> Result<(Channel, BTreeSet<i64>)> {
    let ids = symbol_ids(c, run, rows, terms, stems)?;
    let (terms, stems) = (terms.to_vec(), stems.to_vec());
    let ch = top(c, &ids, |h| {
        let mut toks = hit_tokens(h);
        toks.push(h.name.to_lowercase());
        let mut m = matched_terms(&toks, &terms, &stems);
        // FTS prefix hits: a term that is a prefix of a token or a substring of the name
        for t in &terms {
            if h.name.to_lowercase().contains(t.as_str()) && !m.contains(t) {
                m.push(t.clone());
            }
        }
        m.sort();
        m
    })?;
    Ok((ch, ids))
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn paths(
    c: &Connection,
    run: i64,
    forms: &[String],
    terms: &[String],
    stems: &[(String, String)],
) -> Result<(Channel, BTreeSet<i64>)> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    let mut st = c.prepare("SELECT id FROM node WHERE run_id = ?1 AND lower(path) LIKE ?2 ESCAPE '\\'")?;
    for t in forms {
        let pat = format!("%{}%", like_escape(t));
        for r in st.query_map(params![run, pat], |r| r.get::<_, i64>(0))? {
            ids.insert(r?);
        }
    }
    let mut st = c.prepare("SELECT id, path FROM node WHERE run_id = ?1")?;
    let rows = st
        .query_map(params![run], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut seen: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for (id, path) in rows {
        let hit = *seen.entry(path.clone()).or_insert_with(|| {
            let toks = tokens(&path);
            !matched_terms(&toks, terms, stems).is_empty()
        });
        if hit {
            ids.insert(id);
        }
    }
    let (forms, terms, stems) = (forms.to_vec(), terms.to_vec(), stems.to_vec());
    let ch = top(c, &ids, |h| {
        let low = h.path.to_lowercase();
        let mut m: BTreeSet<String> = matched_terms(&tokens(&h.path), &terms, &stems).into_iter().collect();
        for f in forms.iter().filter(|f| low.contains(f.as_str())) {
            m.insert(f.clone());
        }
        m.into_iter().collect()
    })?;
    Ok((ch, ids))
}

fn kinds(c: &Connection, run: i64, kinds: &[&str], mcp: bool) -> Result<(Channel, BTreeSet<i64>)> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    let mut st = c.prepare("SELECT id FROM node WHERE run_id = ?1 AND kind = ?2")?;
    for k in kinds {
        for r in st.query_map(params![run, k], |r| r.get::<_, i64>(0))? {
            ids.insert(r?);
        }
    }
    if mcp {
        let mut st = c.prepare(
            "SELECT id FROM node WHERE run_id = ?1 AND kind IN ('function','method','class') \
             AND (lower(name) LIKE '%mcp%' OR lower(path) LIKE '%mcp%')",
        )?;
        for r in st.query_map(params![run], |r| r.get::<_, i64>(0))? {
            ids.insert(r?);
        }
    }
    let labels: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
    let ch = top(c, &ids, |h| {
        if labels.contains(&h.kind) {
            vec![h.kind.clone()]
        } else {
            vec!["mcp-tool".to_string()]
        }
    })?;
    Ok((ch, ids))
}

fn communities(store: &Store, run: i64, terms: &[String]) -> Result<Channel> {
    let mut hits = vec![];
    for cm in store.communities_of_run(crate::domain::RunId(run))? {
        let have: BTreeSet<String> = tokens(&cm.label)
            .into_iter()
            .chain(cm.keywords.iter().map(|k| k.to_lowercase()))
            .collect();
        let matched: Vec<String> = terms.iter().filter(|t| have.contains(*t)).cloned().collect();
        if !matched.is_empty() {
            hits.push(AbsentHit {
                name: cm.label,
                kind: "community".into(),
                path: String::new(),
                line_start: 0,
                matched,
            });
        }
    }
    hits.sort_by(|a, b| b.matched.len().cmp(&a.matched.len()).then(a.name.cmp(&b.name)));
    let count = hits.len();
    hits.truncate(5);
    Ok(Channel { count, hits })
}

/// The run both the check and the claim pin: the latest done run of provider `reference` (an external provider may have
/// seen fewer files). Returns (run id, provider).
pub(crate) fn pinned_run(store: &Store, repo: &str) -> Result<(i64, String)> {
    let r = store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo named {repo}")))?;
    let run = store
        .latest_done_run_by_provider(r.id.unwrap(), "reference")?
        .ok_or_else(|| {
            AppError::NotFound(format!("{repo} has no finished run of the reference provider; build it first"))
        })?;
    Ok((run.id.unwrap().0, run.provider))
}

/// The terms and the suffix-stripped stems a feature is searched with (the manual `--searched` path must name one).
pub(crate) fn feature_search_words(f: &FeatureRow) -> Vec<String> {
    let terms = feature_terms(f);
    let stems = stems_of(&terms);
    terms.into_iter().chain(stems.into_iter().map(|s| s.0)).collect()
}

/// Words of definition prose that name nothing.
const PROSE_STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "when", "where", "which", "what", "are", "not", "any",
    "can", "has", "have", "its", "per", "via", "each", "one", "all", "use", "used", "uses", "using", "such", "than",
    "then", "they", "their", "them", "also", "only", "more", "most", "other", "over", "under", "between", "without",
    "within", "was", "were", "been", "being", "will", "would", "should", "could", "may", "might", "must", "does", "did",
    "out", "but", "yet", "too", "very", "both", "through", "about", "after", "before", "while", "during", "against", "upon", "onto", "these", "those", "there", "here", "some", "many", "same", "own", "nor", "off", "via", "etc",
];

/// The words `claim none --searched` accepts as naming the feature (guard 1): `feature_search_words` first, then the
/// tokens of the `definition` and `counts_as_has` text, minus stop words and the family words (a word the whole family
/// shares names the family, not this feature). Separate from `feature_search_words`, which the graph check keeps: the
/// definition prose is a naming vocabulary for an author's terms, never a channel term.
pub(crate) fn feature_naming_words(store: &Store, f: &FeatureRow) -> Result<Vec<String>> {
    let mut out = feature_search_words(f);
    let extra: Vec<String> = tokens(&f.definition)
        .into_iter()
        .chain(tokens(&f.counts_as_has))
        .filter(|t| !PROSE_STOP.contains(&t.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|t| !out.contains(t))
        .collect();
    let common = family_words(store, f, &extra)?;
    out.extend(extra.into_iter().filter(|t| !common.contains(t)));
    Ok(out)
}

/// Everything the channels computed for one feature: the terms, the per-channel hit lists and the node id sets.
struct Prep {
    terms: Vec<String>,
    common: Vec<String>,
    stem_pairs: Vec<(String, String)>,
    forms: Vec<String>,
    names: Vec<String>,
    channels: Channels,
    sym_ids: BTreeSet<i64>,
    path_ids: BTreeSet<i64>,
    kind_ids: BTreeSet<i64>,
}

/// `strict`: refuse a feature with no usable term (the check). Not strict (the read set of a manual `none`): such a
/// feature has empty channels and the author's search stands alone.
fn prepare(store: &Store, run: i64, f: &FeatureRow, strict: bool) -> Result<Prep> {
    let all_terms = feature_terms(f);
    if all_terms.is_empty() && strict {
        return Err(AppError::InvalidArguments(format!(
            "feature {} has no searchable term; an absence of nothing proves nothing",
            f.slug
        )));
    }
    let (names, ks, mcp) = kinds_named(f);
    let c = store.conn();
    let common = family_words(store, f, &all_terms)?;
    let terms: Vec<String> = all_terms.iter().filter(|t| !common.contains(t)).cloned().collect();
    if terms.is_empty() && strict {
        return Err(AppError::InvalidArguments(format!(
            "every term of {} is a family word in {} ({}); pass --searched with specific terms",
            f.slug,
            f.family,
            common.join(", ")
        )));
    }
    let empty = || (Channel::default(), BTreeSet::new());
    let stem_pairs = stems_of(&terms);
    let forms = path_forms(f, &terms);
    let (symbols_ch, sym_ids, paths_ch, path_ids, communities_ch) = if terms.is_empty() {
        let (a, b) = empty();
        let (c2, d) = empty();
        (a, b, c2, d, Channel::default())
    } else {
        let rows = name_rows(c, run)?;
        let (a, b) = symbols(c, run, &rows, &terms, &stem_pairs)?;
        let (c2, d) = paths(c, run, &forms, &terms, &stem_pairs)?;
        (a, b, c2, d, communities(store, run, &terms)?)
    };
    let (kinds_ch, kind_ids) = kinds(c, run, &ks, mcp)?;
    Ok(Prep {
        terms,
        common,
        stem_pairs,
        forms,
        names,
        channels: Channels { symbols: symbols_ch, paths: paths_ch, communities: communities_ch, kinds: kinds_ch },
        sym_ids,
        path_ids,
        kind_ids,
    })
}

/// The terms of an author's `--searched` text, lowercased, split as the claim splits them.
fn searched_terms(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The same guard `claim none --searched` applies: the terms must name the feature.
pub(crate) fn guard_searched(store: &Store, f: &FeatureRow, terms_text: &str) -> Result<()> {
    if super::search::fts_query_or(terms_text).is_none() {
        return Err(AppError::InvalidArguments("--searched has no searchable term".into()));
    }
    let words = feature_naming_words(store, f)?;
    let named = tokens(terms_text).iter().any(|t| words.iter().any(|w| w == t));
    if !named {
        return Err(AppError::InvalidArguments(format!(
            "searched terms do not name the feature: \"{terms_text}\" has none of {}",
            words.iter().take(15).cloned().collect::<Vec<_>>().join(", ") + if words.len() > 15 { ", …" } else { "" }
        )));
    }
    Ok(())
}

/// Union the channel ids with the author's search, score and rank them.
fn build_read_set(store: &Store, run: i64, p: &Prep, searched: Option<&str>) -> Result<ReadSet> {
    let c = store.conn();
    // the author's search: ids with their bm25 score, best first
    let mut fts: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    let mut extra_terms: Vec<String> = vec![];
    if let Some(text) = searched {
        extra_terms = searched_terms(text);
        if let Some(m) = super::search::fts_query_or(&extra_terms.join(" ")) {
            let mut st = c.prepare(
                "SELECT n.id, -bm25(node_fts) AS score FROM node_fts CROSS JOIN node n ON n.id = node_fts.rowid \
                 WHERE node_fts MATCH ?1 AND n.run_id = ?2 ORDER BY score DESC, n.id ASC LIMIT ?3",
            )?;
            for r in st.query_map(params![m, run, FTS_FETCH as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))? {
                let (id, sc) = r?;
                fts.insert(id, sc);
            }
        }
    }
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    ids.extend(&p.sym_ids);
    ids.extend(&p.path_ids);
    ids.extend(&p.kind_ids);
    ids.extend(fts.keys());

    let mut terms: Vec<String> = p.terms.clone();
    for t in &extra_terms {
        if !terms.contains(t) {
            terms.push(t.clone());
        }
    }
    let stems = stems_of(&terms);
    let mut forms = p.forms.clone();
    for t in &extra_terms {
        if !forms.contains(t) {
            forms.push(t.clone());
        }
    }
    let mut st = c.prepare(&format!(
        "SELECT n.name, n.qualified_name, n.kind, n.path, n.line_start, {} FROM node n WHERE n.id = ?1",
        super::search::LABEL_SUBQUERY
    ))?;
    let mut scored: Vec<(u32, f64, Candidate, i64)> = vec![];
    for id in &ids {
        let (name, qn, kind, path, line, label) = st.query_row(params![id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let lname = name.to_lowercase();
        let ltoks = tokens(&name);
        let lpath = path.to_lowercase();
        let lqn = qn.to_lowercase();
        let mut why: Vec<String> = vec![];
        let exact: Vec<&String> = terms.iter().filter(|t| **t == lname).collect();
        let mut tok_hits: Vec<String> = vec![];
        for t in &terms {
            if ltoks.iter().any(|k| stem_match(t, k)) {
                tok_hits.push(t.clone());
            }
        }
        for (st_, t) in &stems {
            if ltoks.iter().any(|k| stem_match(st_, k)) && !tok_hits.contains(t) {
                tok_hits.push(t.clone());
            }
        }
        let path_hits: Vec<&String> = forms.iter().filter(|f| lpath.contains(f.as_str())).collect();
        let qn_hits: Vec<&String> = terms.iter().filter(|t| lqn.contains(t.as_str())).collect();
        let mut score = 0u32;
        if !exact.is_empty() {
            score = 8;
            why.extend(exact.iter().map(|t| format!("name={t}")));
        }
        if !tok_hits.is_empty() {
            if score == 0 {
                score = 4;
            }
            why.extend(tok_hits.iter().map(|t| format!("token~{t}")));
        }
        if !path_hits.is_empty() || !qn_hits.is_empty() {
            if score == 0 {
                score = 2;
            }
            why.extend(path_hits.iter().map(|t| format!("path~{t}")));
            for t in &qn_hits {
                if !path_hits.contains(t) {
                    why.push(format!("qn~{t}"));
                }
            }
        }
        if p.kind_ids.contains(id) {
            if score == 0 {
                score = 1;
            }
            why.push(format!("kind={kind}"));
        }
        let bm = fts.get(id).copied();
        if bm.is_some() {
            if score == 0 {
                score = 1;
            }
            why.push("fts".into());
        }
        if let Some(l) = &label {
            let lt = tokens(l);
            if let Some(t) = terms.iter().find(|t| lt.contains(t)) {
                score += 1;
                why.push(format!("community~{t}"));
            }
        }
        scored.push((
            score,
            bm.unwrap_or(0.0),
            Candidate { qualified_name: qn, kind, path, line_start: line, score, why },
            *id,
        ));
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.2.qualified_name.cmp(&b.2.qualified_name))
            .then(a.3.cmp(&b.3))
    });
    let total = scored.len();
    let names: BTreeSet<String> = scored.iter().map(|x| x.2.qualified_name.clone()).collect();
    let candidates = scored.into_iter().take(READ_CAP).map(|x| x.2).collect();
    Ok(ReadSet { cap: READ_CAP, total, candidates, names })
}

/// The ranked candidates a `none` over `searched` must read: the feature's channel hits plus the author's search,
/// scored and capped at `READ_CAP`. `total` counts the union before the cap.
pub(crate) fn read_set(
    store: &Store,
    _repo: &str,
    run: i64,
    _provider: &str,
    f: &FeatureRow,
    searched: Option<&str>,
) -> Result<ReadSet> {
    let p = prepare(store, run, f, false)?;
    build_read_set(store, run, &p, searched)
}

/// The check on an open store for a feature row and a run; shared by `claim` and the verb.
pub(crate) fn check_feature(
    store: &Store,
    repo: &str,
    run: i64,
    provider: &str,
    f: &FeatureRow,
    searched: Option<&str>,
) -> Result<AbsentCheck> {
    if let Some(t) = searched {
        guard_searched(store, f, t)?;
    }
    let p = prepare(store, run, f, true)?;
    let rs = build_read_set(store, run, &p, searched)?;
    let Prep { terms, common, stem_pairs, forms, names, channels, .. } = p;
    // zero hits records an absence only when no family word was set aside: with one dropped, the graph proves less,
    // and the check refuses with the uncounted words named (the author records the absence with --searched)
    let hits = channels.symbols.count + channels.paths.count + channels.communities.count + channels.kinds.count;
    let searched_hits = searched.is_some() && rs.total > 0;
    let absent = hits == 0 && !searched_hits && common.is_empty();
    let reason = if hits > 0 || searched_hits {
        "hits".to_string()
    } else if !common.is_empty() {
        format!("family words set aside: {}", common.join(", "))
    } else {
        "absent".to_string()
    };
    Ok(AbsentCheck {
        repo: repo.into(),
        family: f.family.clone(),
        feature: f.slug.clone(),
        run_id: run,
        provider: provider.into(),
        terms,
        common,
        stems: stem_pairs.into_iter().map(|s| s.0).collect(),
        path_forms: forms,
        kinds_named: names,
        reason,
        channels,
        absent,
        searched: searched.map(String::from),
        read_set: rs,
        evidence_confidence: if absent { "inferred" } else { "observed" },
    })
}

pub fn absent_check(
    root: &Path,
    repo: &str,
    family: &str,
    feature: &str,
    searched: Option<&str>,
) -> Result<AbsentCheck> {
    let store = Store::open_existing(root)?;
    store
        .family_by_name(family)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {family}")))?;
    let (run, provider) = pinned_run(&store, repo)?;
    let f = store
        .features_of(family)?
        .into_iter()
        .find(|f| f.slug == feature)
        .ok_or_else(|| AppError::NotFound(format!("no feature {feature} in {family}")))?;
    if f.tier == "retired" {
        return Err(AppError::InvalidArguments(format!("feature {feature} is retired")));
    }
    check_feature(&store, repo, run, &provider, &f, searched)
}
