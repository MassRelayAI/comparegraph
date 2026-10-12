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
