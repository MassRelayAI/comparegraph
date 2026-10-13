//! The agreement score between two providers' runs of one clone: per node kind the Jaccard over qualified names, per edge kind
//! the Jaccard over (source, target) qualified names. A kind below `FLOOR` becomes an `agreement_<kind>` limitation on the later
//! run. Nothing is stored beyond the limitation (the schema stays v1).
use super::Result;
use crate::domain::{Limitation, RunId};
use crate::store::Store;
use std::collections::{BTreeMap, BTreeSet};

pub const FLOOR: f64 = 0.8;

type KindSets = BTreeMap<String, BTreeSet<String>>;

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgreementRow {
    /// `nodes` or `edges`
    pub scope: String,
    pub kind: String,
    pub baseline: String,
    pub other: String,
    pub baseline_count: usize,
    pub other_count: usize,
    pub both: usize,
    pub jaccard: f64,
}

pub fn compute(store: &Store, base: RunId, other: RunId, base_name: &str, other_name: &str) -> Result<Vec<AgreementRow>> {
    let mut rows = Vec::new();
    let sets = |run: RunId| -> Result<(KindSets, KindSets)> {
        let nodes = store.nodes_of_run(run)?;
        let qn: BTreeMap<_, _> = nodes.iter().map(|n| (n.id.unwrap(), n.qualified_name.clone())).collect();
        let mut nk: KindSets = BTreeMap::new();
        for n in &nodes {
            nk.entry(n.kind.as_str().to_string()).or_default().insert(n.qualified_name.clone());
        }
        let mut ek: KindSets = BTreeMap::new();
        for e in store.edges_of_run(run)? {
            if let (Some(s), Some(t)) = (qn.get(&e.source), qn.get(&e.target)) {
                ek.entry(e.kind.as_str().to_string()).or_default().insert(format!("{s}\t{t}"));
            }
        }
        Ok((nk, ek))
    };
    let (an, ae) = sets(base)?;
    let (bn, be) = sets(other)?;
    for (scope, a, b) in [("nodes", an, bn), ("edges", ae, be)] {
        let kinds: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        let empty = BTreeSet::new();
        for k in kinds {
            let (x, y) = (a.get(k).unwrap_or(&empty), b.get(k).unwrap_or(&empty));
            let both = x.intersection(y).count();
            let union = x.len() + y.len() - both;
            rows.push(AgreementRow {
                scope: scope.into(),
                kind: k.clone(),
                baseline: base_name.into(),
                other: other_name.into(),
                baseline_count: x.len(),
                other_count: y.len(),
                both,
                jaccard: if union == 0 { 1.0 } else { (both as f64 / union as f64 * 10_000.0).round() / 10_000.0 },
            });
        }
    }
    Ok(rows)
}

/// `agreement_<kind>` for every row below the floor.
pub fn limitations(rows: &[AgreementRow]) -> Vec<Limitation> {
    rows.iter()
        .filter(|r| r.jaccard < FLOOR)
        .map(|r| {
            Limitation::new(
                &format!("agreement_{}", r.kind),
                format!(
                    "{} {}: Jaccard {:.3} between {} ({}) and {} ({}), {} in common; below {FLOOR}",
                    r.kind, r.scope, r.jaccard, r.baseline, r.baseline_count, r.other, r.other_count, r.both
                ),
            )
        })
        .collect()
}

/// The table `build --providers` prints on stderr.
pub fn table(rows: &[AgreementRow]) -> String {
    let mut s = String::from("agreement (Jaccard over qualified names / (source,target) pairs)\nscope  kind        baseline -> other           base  other  both  jaccard\n");
    for r in rows {
        s.push_str(&format!(
            "{:<6} {:<11} {:<8} -> {:<14} {:>5} {:>6} {:>5}  {:.3}{}\n",
            r.scope, r.kind, r.baseline, r.other, r.baseline_count, r.other_count, r.both, r.jaccard,
            if r.jaccard < FLOOR { "  <" } else { "" }
        ));
    }
    s
}
