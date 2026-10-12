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
