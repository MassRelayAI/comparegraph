//! `family propose`: a draft taxonomy seeded from the corpus alone (no model). Terms come from community labels,
//! community keywords and symbol names (the tokenizer and stoplist of communities); written as `rows.draft.md`.
use super::communities::tokens;
use super::{AppError, Result};
use crate::domain::NodeKind;
use crate::store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: String,
    pub term: String,
    pub group: String,
    pub repos: usize,
    pub occurrences: usize,
    pub per_repo: BTreeMap<String, usize>,
}
#[derive(Debug, Serialize)]
pub struct Proposed {
    pub family: String,
    pub draft_path: String,
    pub repos: Vec<String>,
    pub min_repos: usize,
    pub rows_md_exists: bool,
    pub candidates: Vec<Candidate>,
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if c != '-' || !out.ends_with('-') {
            out.push(c);
        }
    }
    out.trim_matches('-').to_string()
}
/// The last path segment of a community label, without a ` (n)` uniqueness suffix.
fn segment_of(label: &str) -> String {
    let l = match label.rfind(" (") {
        Some(i) if label.ends_with(')') => &label[..i],
        _ => label,
    };
    slug(l.rsplit('/').next().unwrap_or(l))
}

pub fn propose(
    root: &Path,
    name: &str,
    repos: &[String],
    out: Option<&Path>,
    min_repos: usize,
    max: usize,
) -> Result<Proposed> {
    let fam_ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !fam_ok {
        return Err(AppError::InvalidArguments(format!(
            "family name `{name}` must be lowercase [a-z0-9-]"
        )));
    }
    let mut names: Vec<String> = repos.iter().map(|r| r.trim().to_string()).collect();
    names.retain(|r| !r.is_empty());
    names.sort();
    names.dedup();
    if names.is_empty() {
        return Err(AppError::InvalidArguments(
            "--repos needs at least one repo".into(),
        ));
    }
    let min_repos = if names.len() == 1 {
        1
    } else {
        min_repos.max(1)
    };
    let store = Store::open(root)?;
    // term -> repo -> occurrences ; term -> group segment -> co-occurrences
    let mut counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut groups: HashMap<String, BTreeMap<String, usize>> = HashMap::new();
    for rn in &names {
        let repo = store
            .repo_by_name(rn)?
            .ok_or_else(|| AppError::NotFound(format!("no repo '{rn}'")))?;
        let rid = repo.id.expect("stored repo has an id");
        let run = store.latest_done_run(rid)?.ok_or_else(|| {
            AppError::NotFound(format!(
                "repo '{rn}' has no built run; run `comparegraph build` first"
            ))
        })?;
        let run_id = run.id.expect("stored run has an id");
        let nodes = store.nodes_of_run(run_id)?;
        let by_id: HashMap<_, _> = nodes.iter().filter_map(|n| Some((n.id?, n))).collect();
        let mut bump = |t: &str, seg: &str| {
            let t = slug(t);
            if t.is_empty() {
                return;
            }
            *counts
                .entry(t.clone())
                .or_default()
                .entry(rn.clone())
                .or_default() += 1;
            if !seg.is_empty() {
                *groups
                    .entry(t)
                    .or_default()
                    .entry(seg.to_string())
                    .or_default() += 1;
            }
        };
        for c in store.communities_of_run(run_id)? {
            let seg = segment_of(&c.label);
            for t in tokens(&seg) {
                bump(&t, &seg);
            }
            for k in &c.keywords {
                bump(k, &seg);
            }
            for m in &c.members {
                if let Some(n) = by_id.get(m).filter(|n| n.kind != NodeKind::File) {
                    for t in tokens(&n.name) {
                        bump(&t, &seg);
                    }
                }
            }
        }
    }
    let mut cands: Vec<Candidate> = counts
        .into_iter()
        .filter(|(_, per)| per.len() >= min_repos)
        .map(|(term, per_repo)| {
            let group = groups
                .get(&term)
                .and_then(|g| g.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))))
                .map(|(s, _)| s.clone())
                .unwrap_or_else(|| "misc".into());
            Candidate {
                id: format!("{group}.{term}"),
                repos: per_repo.len(),
                occurrences: per_repo.values().sum(),
                term,
                group,
                per_repo,
            }
        })
        .collect();
    cands.sort_by(|a, b| {
        b.repos
            .cmp(&a.repos)
            .then(b.occurrences.cmp(&a.occurrences))
            .then(a.term.cmp(&b.term))
    });
    cands.truncate(max);
    if cands.is_empty() {
        return Err(AppError::Empty(format!(
            "no term appears in at least {min_repos} of {} repos",
            names.len()
        )));
    }
    // pack order: by group, then by rank (stable sort keeps the ranking within a group)
    let mut rows: Vec<&Candidate> = cands.iter().collect();
    rows.sort_by(|a, b| a.group.cmp(&b.group));
    let mut text = format!(
        "---\nfamily: {name}\ndescription: draft taxonomy seeded from {} repos ({}); review and rename to rows.md\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n",
        names.len(),
        names.join(", ")
    );
    for c in rows {
        text.push_str(&format!(
            "| {} | {} | {} | draft: seen in {} of {} repos ({}) | draft: define | candidate | 1 |\n",
            c.id,
            c.group,
            c.term,
            c.repos,
            names.len(),
            c.per_repo.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    let dir: PathBuf = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join("families").join(name));
    std::fs::create_dir_all(&dir).map_err(|e| AppError::Io(dir.clone(), e))?;
    let draft = dir.join("rows.draft.md");
    std::fs::write(&draft, text).map_err(|e| AppError::Io(draft.clone(), e))?;
    Ok(Proposed {
        family: name.to_string(),
        draft_path: draft.to_string_lossy().to_string(),
        repos: names,
        min_repos,
        rows_md_exists: dir.join("rows.md").exists(),
        candidates: cands,
    })
}

// ---- `family propose <family> --from-claims <repo> --rows <jsonl>` ----

/// One scored row of the standard, in the shape of a `claim --batch` line. Only `feature` is required; fields this
/// verb does not use (`author`, `searched`, `hits_read`, ...) are accepted and ignored, so the scorer's own batch file
/// (including the rows `claim --batch` refused for an unknown id) can be passed as it is.
#[derive(serde::Deserialize)]
struct ScoredRow {
    #[serde(default)]
    repo: Option<String>,
    feature: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    at: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DraftRow {
    pub id: String,
    pub group: String,
    pub title: String,
    /// the verdicts the standard's rows gave this id, sorted
    pub verdicts: Vec<String>,
    /// the input lines (1-based) that named it
    pub lines: Vec<usize>,
    /// the citations those rows carried, sorted
    pub at: Vec<String>,
    pub note: String,
}
#[derive(Debug, Serialize)]
pub struct FromClaims {
    pub family: String,
    pub repo: String,
    /// always true: every row written is a draft (tier `candidate`, text `draft: ...`), never registered
    pub draft: bool,
    pub rows_source: String,
    /// `None` when nothing is missing (no file is written)
    pub draft_path: Option<String>,
    pub rows_md_exists: bool,
    /// how to use the draft: merge its rows into the family's rows.md; added as-is it would retire every other row
    pub merge_note: &'static str,
    /// rows read for this repo (rows without a `repo` count as this repo's)
    pub rows: usize,
    /// rows naming another repo, skipped
    pub other_repo_rows: usize,
    /// ids the rows name that are active features of the family already
    pub covered: Vec<String>,
    /// the repo's latest claims on this family in the store
    pub stored_claims: usize,
    pub drafts: Vec<DraftRow>,
}

/// One line of pack text: no `|`, no newline, whitespace collapsed, at most `cap` chars.
fn cell(s: &str, cap: usize) -> String {
    let s = s.replace('|', "/");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= cap {
        s
    } else {
        s.chars().take(cap).collect::<String>() + "..."
    }
}
/// `<group>.<slug>` from a row: a dotted feature keeps its own group; otherwise the row's `group`, else `draft`.
fn draft_id(row: &ScoredRow) -> Option<(String, String)> {
    let (g, s) = match row.feature.split_once('.') {
        Some((g, s)) => (slug(g), slug(s)),
        None => (
            row.group
                .as_deref()
                .map(slug)
                .filter(|g| !g.is_empty())
                .unwrap_or_else(|| "draft".into()),
            slug(&row.feature),
        ),
    };
    (!g.is_empty() && !s.is_empty()).then(|| (format!("{g}.{s}"), g))
}

/// Draft the feature ids a standard repo's scored rows name but the family lacks. Reads the rows and the store; writes
/// only `rows.from-claims.draft.md` (never `rows.md`, never the store). No model: ids and text come from the rows.
pub fn from_claims(
    root: &Path,
    family: &str,
    repo: &str,
    lines: impl std::io::BufRead,
    rows_source: &str,
    out: Option<&Path>,
) -> Result<FromClaims> {
    let store = Store::open_existing(root)?;
    store
        .family_by_name(family)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {family}")))?;
    store
        .repo_by_name(repo)?
        .ok_or_else(|| AppError::NotFound(format!("no repo '{repo}'")))?;
    let active: std::collections::BTreeSet<String> = store
        .features_of(family)?
        .iter()
        .filter(|f| f.tier != "retired")
        .map(|f| f.key())
        .collect();
    let stored_claims = super::claim::claims(root, Some(repo), Some(family), None, false)?.len();
    drop(store);

    let (mut rows, mut other_repo_rows) = (0usize, 0usize);
    let mut covered: std::collections::BTreeSet<String> = Default::default();
    let mut drafts: BTreeMap<String, DraftRow> = BTreeMap::new();
    for (n, line) in lines.lines().enumerate() {
        let line_no = n + 1;
        let text = line.map_err(|e| {
            AppError::InvalidArguments(format!("{rows_source}: reading line {line_no}: {e}"))
        })?;
        let text = if line_no == 1 {
            text.trim_start_matches('\u{feff}')
        } else {
            text.as_str()
        };
        if text.trim().is_empty() {
            continue;
        }
        let row: ScoredRow = serde_json::from_str(text).map_err(|e| {
            AppError::InvalidArguments(format!(
                "{rows_source}: line {line_no}: not a scored row: {e}"
            ))
        })?;
        if row.repo.as_deref().is_some_and(|r| r.trim() != repo) {
            other_repo_rows += 1;
            continue;
        }
        rows += 1;
        if let Some(v) = row.verdict.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            if !matches!(v.to_ascii_lowercase().as_str(), "has" | "partial" | "none" | "unknown") {
                return Err(AppError::InvalidArguments(format!(
                    "{rows_source}: line {line_no}: verdict `{v}` is not one of has|partial|none|unknown"
                )));
            }
        }
        let (id, group) = draft_id(&row).ok_or_else(|| {
            AppError::InvalidArguments(format!(
                "{rows_source}: line {line_no}: feature `{}` gives no id",
                row.feature
            ))
        })?;
        if active.contains(&id) {
            covered.insert(id);
            continue;
        }
        let d = drafts.entry(id.clone()).or_insert_with(|| DraftRow {
            title: cell(
                row.title
                    .as_deref()
                    .unwrap_or(match row.feature.split_once('.') {
                        Some((_, s)) => s,
                        None => &row.feature,
                    }),
                120,
            ),
            id,
            group,
            verdicts: vec![],
            lines: vec![],
            at: vec![],
            note: String::new(),
        });
        d.lines.push(line_no);
        if let Some(v) = row
            .verdict
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty())
        {
            d.verdicts.push(v);
        }
        d.at.extend(
            row.at
                .iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty()),
        );
        if d.note.is_empty() {
            d.note = cell(row.note.as_deref().unwrap_or(""), 240);
        }
    }
    let mut drafts: Vec<DraftRow> = drafts.into_values().collect();
    for d in &mut drafts {
        d.verdicts.sort();
        d.verdicts.dedup();
        d.at.sort();
        d.at.dedup();
    }
    let dir: PathBuf = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join("families").join(family));
    let draft_path = if drafts.is_empty() {
        None
    } else {
        let mut text = format!(
            "---\nfamily: {family}\ndraft: true\ndescription: draft: {} feature ids that {}'s scored rows name and {family} lacks; define each, then merge the rows into rows.md (adding this file as rows.md would retire every other row of {family}; `family add` refuses it without --accept-draft)\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n",
            drafts.len(),
            cell(repo, 120)
        );
        for d in &drafts {
            let verdicts = if d.verdicts.is_empty() {
                "scored".to_string()
            } else {
                d.verdicts.join("/")
            };
            let lines = d
                .lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            // every free-text part goes through `cell`: a `|` in the repo name would split the row
            let mut def = format!("draft: {} {verdicts} (rows line {lines})", cell(repo, 120));
            if let Some(a) = d.at.first() {
                def.push_str(&format!("; cited {}", cell(a, 120)));
            }
            if !d.note.is_empty() {
                def.push_str(&format!("; note: {}", d.note));
            }
            let title = if d.title.is_empty() {
                d.id.clone()
            } else {
                d.title.clone()
            };
            text.push_str(&format!(
                "| {} | {} | {} | {} | draft: define | candidate | 1 |\n",
                d.id, d.group, title, def
            ));
        }
        std::fs::create_dir_all(&dir).map_err(|e| AppError::Io(dir.clone(), e))?;
        let p = dir.join("rows.from-claims.draft.md");
        std::fs::write(&p, text).map_err(|e| AppError::Io(p.clone(), e))?;
        Some(p.to_string_lossy().to_string())
    };
    Ok(FromClaims {
        family: family.to_string(),
        repo: repo.to_string(),
        draft: true,
        rows_source: rows_source.to_string(),
        draft_path,
        rows_md_exists: dir.join("rows.md").exists(),
        merge_note: "merge the draft rows into the family's rows.md; adding the draft as the pack would retire every row it \
                     does not list (its front matter says `draft: true`, so `family add` refuses it without --accept-draft)",
        rows,
        other_repo_rows,
        covered: covered.into_iter().collect(),
        stored_claims,
        drafts,
    })
}
