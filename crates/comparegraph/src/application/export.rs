use super::{AppError, Result};
use crate::store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub fn export_json(root: &Path, repo: Option<&str>) -> Result<Value> {
    let store = Store::open_existing(root)?;
    let repos: Vec<_> = store
        .repos()?
        .into_iter()
        .filter(|r| repo.map(|n| n == r.name).unwrap_or(true))
        .collect();
    if let Some(n) = repo {
        if repos.is_empty() {
            return Err(AppError::NotFound(format!("no repo '{n}'")));
        }
    }
    let mut out = Vec::new();
    let mut total_nodes = 0;
    for r in repos {
        let Some(run) = store.latest_done_run(r.id.unwrap())? else {
            out.push(json!({ "repo": r, "run": null, "nodes": [], "edges": [] }));
            continue;
        };
        let nodes = store.nodes_of_run(run.id.unwrap())?;
        let qn: HashMap<_, _> = nodes
            .iter()
            .map(|n| (n.id.unwrap(), n.qualified_name.clone()))
            .collect();
        let edges: Vec<Value> = store.edges_of_run(run.id.unwrap())?.iter().map(|e| json!({ "kind": e.kind.as_str(), "source_qn": qn[&e.source], "target_qn": qn[&e.target], "confidence": e.confidence })).collect();
        total_nodes += nodes.len();
        out.push(json!({ "repo": r, "run": run, "nodes": nodes, "edges": edges }));
    }
    if total_nodes == 0 {
        return Err(AppError::Empty("export has 0 nodes: build first".into()));
    }
    // the analysis layer: families with their matrices, latest relatedness, latest claims and decisions (with history counts)
    let mut families = Vec::new();
    for f in store.families()? {
        let feats = store.features_of(&f.name)?;
        let matrix = super::matrix::matrix(root, &f.name)?;
        families.push(json!({ "name": f.name, "description": f.description, "features": feats, "matrix": matrix }));
    }
    let relatedness = super::map::latest_relatedness(root, repo, None)?;
    let all_claims = super::claim::claims(root, repo, None, None, true)?;
    let mut n_claims: HashMap<(String, String, String), usize> = HashMap::new();
    for c in &all_claims {
        *n_claims
            .entry((c.repo.clone(), c.family.clone(), c.feature.clone()))
            .or_default() += 1;
    }
    let claims: Vec<Value> = super::claim::claims(root, repo, None, None, false)?
        .into_iter()
        .map(|c| {
            let h = n_claims[&(c.repo.clone(), c.family.clone(), c.feature.clone())];
            let mut v = serde_json::to_value(&c).unwrap();
            v["history"] = json!(h);
            v
        })
        .collect();
    let mut n_dec: HashMap<(String, String, Option<String>), usize> = HashMap::new();
    for d in super::decide::decisions(root, repo, true)? {
        *n_dec.entry((d.repo, d.scope, d.feature)).or_default() += 1;
    }
    let decisions: Vec<Value> = super::decide::decisions(root, repo, false)?
        .into_iter()
        .map(|d| {
            let h = n_dec[&(d.repo.clone(), d.scope.clone(), d.feature.clone())];
            let mut v = serde_json::to_value(&d).unwrap();
            v["history"] = json!(h);
            v
        })
        .collect();
    Ok(
        json!({ "schema": 1, "repos": out, "families": families, "relatedness": relatedness, "claims": claims, "decisions": decisions }),
    )
}

// ---- CSV (RFC 4180 quoting; LF line endings, not CRLF: the files are read by tools and diffed in git) ----

/// One CSV field: quoted when it holds `,` `"` CR or LF, with `"` doubled.
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
fn csv_row(out: &mut String, fields: &[String]) {
    let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
    out.push_str(&line.join(","));
    out.push('\n');
}
fn write_file(dir: &Path, name: &str, body: &str) -> Result<String> {
    let p = dir.join(name);
    std::fs::write(&p, body).map_err(|e| AppError::Io(p.clone(), e))?;
    Ok(name.to_string())
}

/// Write `nodes.csv`, `edges.csv`, `claims.csv`, `decisions.csv` and `matrix-<family>.csv` into `out`.
/// `repo` filters nodes, edges, claims and decisions (matrices always span every repo). Refuses on 0 nodes.
pub fn export_csv(root: &Path, repo: Option<&str>, out: &Path) -> Result<Value> {
    let store = Store::open_existing(root)?;
    let mut repos = store.repos()?;
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(n) = repo {
        repos.retain(|r| r.name == n);
        if repos.is_empty() {
            return Err(AppError::NotFound(format!("no repo '{n}'")));
        }
    }
    let mut nodes: Vec<(String, String, u32, String, Vec<String>)> = Vec::new();
    let mut edges: Vec<Vec<String>> = Vec::new();
    for r in &repos {
        let Some(run) = store.latest_done_run(r.id.unwrap())? else {
            continue;
        };
        let ns = store.nodes_of_run(run.id.unwrap())?;
        let qn: HashMap<_, _> = ns
            .iter()
            .map(|n| (n.id.unwrap(), n.qualified_name.clone()))
            .collect();
        for n in &ns {
            nodes.push((
                r.name.clone(),
                n.path.clone(),
                n.line_start,
                n.qualified_name.clone(),
                vec![
                    r.name.clone(),
                    n.kind.as_str().to_string(),
                    n.name.clone(),
                    n.qualified_name.clone(),
                    n.path.clone(),
                    n.line_start.to_string(),
                    n.line_end.to_string(),
                    n.language.clone(),
                    n.content_hash.clone(),
                ],
            ));
        }
        for e in store.edges_of_run(run.id.unwrap())? {
            edges.push(vec![
                r.name.clone(),
                e.kind.as_str().to_string(),
                qn.get(&e.source).cloned().unwrap_or_default(),
                qn.get(&e.target).cloned().unwrap_or_default(),
                e.confidence.to_string(),
            ]);
        }
    }
    if nodes.is_empty() {
        return Err(AppError::Empty("export has 0 nodes: build first".into()));
    }
    nodes.sort_by(|a, b| (&a.0, &a.1, a.2, &a.3).cmp(&(&b.0, &b.1, b.2, &b.3)));
    edges.sort_by(|a, b| (&a[0], &a[1], &a[2], &a[3]).cmp(&(&b[0], &b[1], &b[2], &b[3])));

    std::fs::create_dir_all(out).map_err(|e| AppError::Io(out.to_path_buf(), e))?;
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut files = Vec::new();

    let mut body = String::new();
    csv_row(&mut body, &s(&["repo", "kind", "name", "qualified_name", "path", "line_start", "line_end", "language", "content_hash"]));
    for n in &nodes {
        csv_row(&mut body, &n.4);
    }
    files.push(write_file(out, "nodes.csv", &body)?);

    let mut body = String::new();
    csv_row(&mut body, &s(&["repo", "kind", "source", "target", "confidence"]));
    for e in &edges {
        csv_row(&mut body, e);
    }
    files.push(write_file(out, "edges.csv", &body)?);

    let mut cl = super::claim::claims(root, repo, None, None, false)?;
    cl.sort_by(|a, b| (&a.repo, &a.feature, &a.created_at, a.id).cmp(&(&b.repo, &b.feature, &b.created_at, b.id)));
    let mut body = String::new();
    csv_row(&mut body, &s(&["id", "repo", "family", "feature", "verdict", "confidence", "author", "created_at", "supersedes_id", "note", "locations"]));
    for c in &cl {
        let locs: Vec<String> = c
            .locations
            .iter()
            .map(|l| format!("{}:{}-{}{}", l.path, l.line_start, l.line_end, if l.stale { " (stale)" } else { "" }))
            .collect();
        csv_row(&mut body, &[
            c.id.to_string(), c.repo.clone(), c.family.clone(), c.feature.clone(), c.verdict.clone(),
            c.confidence.clone(), c.author.clone(), c.created_at.clone(),
            c.supersedes_id.map(|i| i.to_string()).unwrap_or_default(), c.note.clone(), locs.join(";"),
        ]);
    }
    files.push(write_file(out, "claims.csv", &body)?);

    let mut ds = super::decide::decisions(root, repo, false)?;
    ds.sort_by(|a, b| (&a.repo, &a.decided_at, a.id).cmp(&(&b.repo, &b.decided_at, b.id)));
    let mut body = String::new();
    csv_row(&mut body, &s(&["id", "repo", "scope", "feature", "disposition", "why", "decided_by", "decided_at", "supersedes_id"]));
    for d in &ds {
        csv_row(&mut body, &[
            d.id.to_string(), d.repo.clone(), d.scope.clone(), d.feature.clone().unwrap_or_default(),
            d.disposition.clone(), d.why.clone(), d.decided_by.clone(), d.decided_at.clone(),
            d.supersedes_id.map(|i| i.to_string()).unwrap_or_default(),
        ]);
    }
    files.push(write_file(out, "decisions.csv", &body)?);

    for f in store.families()? {
        let m = super::matrix::matrix(root, &f.name)?;
        let mut head = s(&["group", "id", "tier", "weight"]);
        head.extend(m.columns.iter().map(|c| c.repo.clone())); // root first, then repos by name
        head.extend(s(&["prevalence", "gap", "differentiator"]));
        let mut body = String::new();
        csv_row(&mut body, &head);
        let mut rows: Vec<_> = m.rows.iter().collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        for r in rows {
            let mut row = vec![r.group.clone(), r.id.clone(), r.tier.clone(), r.weight.to_string()];
            row.extend(r.cells.iter().cloned());
            row.push(r.prevalence.map(|p| format!("{p:.3}")).unwrap_or_default());
            row.push(r.gap.map(|b| b.to_string()).unwrap_or_default());
            row.push(r.differentiator.map(|b| b.to_string()).unwrap_or_default());
            csv_row(&mut body, &row);
        }
        let safe: String = f.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        files.push(write_file(out, &format!("matrix-{safe}.csv"), &body)?);
    }
    Ok(json!({ "written": out, "files": files, "nodes": nodes.len(), "edges": edges.len(), "claims": cl.len(), "decisions": ds.len() }))
}
