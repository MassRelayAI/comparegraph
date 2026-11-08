//! `reach` on a has/partial claim (field-run ask 5, docs/reports/2026-11-07-twelve-harness-field-run.md): does a
//! running program get to the cited code? Computed once, at write time, by the graph's own breadth-first walk
//! (`graph::reach_from_entries`: callees over `calls` edges from every `entry-points` symbol), bounded like every query.
//! The run's reach state (entry points, walk caches, the walk from every entry point) is built once per run and shared
//! by every row of a batch (`graph::ReachCache`; docs/reports/2026-11-08-reach-cost.md).
//! `reached` carries the path; `unreached` means no static call path within the bounds; `unknown` says why in `note`.
pub use super::graph::ReachCache;
use super::graph::{reach_from_entries, MAX_DEPTH, MAX_NODES};
use super::{AppError, Result};
use crate::domain::{Limitation, NodeKind};
use crate::store::Store;
use rusqlite::params;
use serde::Serialize;

/// The walk's bounds: the query maxima, since an entry point is often many hops from a tool.
pub const REACH_DEPTH: usize = MAX_DEPTH;
pub const REACH_MAX_NODES: usize = MAX_NODES;
/// name-guessed (0.5) edges count; a path through one says so
pub const REACH_MIN_CONF: f64 = 0.5;

pub const STATUSES: [&str; 3] = ["reached", "unreached", "unknown"];

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Reach {
    /// reached | unreached | unknown
    pub status: String,
    /// qualified names from an entry point to the cited symbol (`reached` only; empty otherwise)
    pub path: Vec<String>,
    /// why `unknown`, or what `unreached` was bounded by
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Reach {
    fn new(status: &str, path: Vec<String>, note: impl Into<String>) -> Self {
        Reach {
            status: status.into(),
            path,
            note: Some(note.into()),
        }
    }
    /// Rebuilt from the three stored columns; `None` when the claim has no reach (older rows, none/unknown verdicts).
    pub fn from_columns(
        status: Option<String>,
        path: Option<String>,
        note: Option<String>,
    ) -> Option<Reach> {
        let status = status?;
        let path = path
            .and_then(|p| serde_json::from_str(&p).ok())
            .unwrap_or_default();
        Some(Reach { status, path, note })
    }
    pub fn path_json(&self) -> String {
        serde_json::to_string(&self.path).unwrap()
    }
}

/// How many times this thread has built a run's reach state (the entry set and the walk's caches). A batch builds it
/// once per run; the cost guard in `tests/claim_reach_cost.rs` counts it.
#[doc(hidden)]
pub fn state_builds() -> usize {
    super::graph::REACH_STATE_BUILDS.with(|n| n.get())
}

pub fn parse_status(s: &str) -> Result<&'static str> {
    STATUSES.iter().find(|x| **x == s).copied().ok_or_else(|| {
        AppError::InvalidArguments(format!("unknown reach {s} (reached|unreached|unknown)"))
    })
}

/// `reach` for a claim whose resolved locations map to `nodes` (the location `node_id`s, in order). Pushes the
/// limitations that qualify the answer.
pub(crate) fn compute(
    store: &Store,
    repo: &str,
    nodes: &[Option<i64>],
    lims: &mut Vec<Limitation>,
    cache: &mut ReachCache,
) -> Result<Reach> {
    // only a function, method or route is something a call reaches; a file or class scope line is not
    let ids: Vec<i64> = nodes.iter().flatten().copied().collect();
    let mut targets: Vec<i64> = vec![];
    {
        let mut st = store
            .conn()
            .prepare_cached("SELECT kind FROM node WHERE id = ?1")?;
        for id in &ids {
            let k: String = st.query_row(params![id], |r| r.get(0))?;
            if matches!(
                NodeKind::parse(&k),
                Some(NodeKind::Function | NodeKind::Method | NodeKind::Route)
            ) {
                targets.push(*id);
            }
        }
    }
    let unknown = |lims: &mut Vec<Limitation>, why: String| {
        lims.push(Limitation::new(
            "reach_unknown",
            format!("reach is unknown: {why}"),
        ));
        Reach::new("unknown", vec![], why)
    };
    if targets.is_empty() {
        let why = if ids.is_empty() {
            "no cited line maps to a graph node".to_string()
        } else {
            "the cited lines sit at file or class scope, not inside a function, method or route"
                .to_string()
        };
        return Ok(unknown(lims, why));
    }
    let er = reach_from_entries(
        store,
        repo,
        &targets,
        REACH_DEPTH,
        REACH_MAX_NODES,
        REACH_MIN_CONF,
        cache,
    )?;
    if !er.uncalled_targets.is_empty() {
        lims.push(Limitation::new(
            "reach_uncalled_target",
            format!(
                "{} nothing calls it, so it is an entry point only by the exported-function heuristic; it was not counted as reaching itself",
                er.uncalled_targets.join(", ")
            ),
        ));
    }
    if let Some(path) = er.path {
        if er.path_confidence < 1.0 {
            lims.push(Limitation::new(
                "reach_name_guessed",
                "the reach path uses at least one name-guessed call edge (confidence 0.5)",
            ));
        }
        let hops = path.len() - 1;
        return Ok(Reach::new(
            "reached",
            path,
            format!("{hops} call hop(s) from an entry point"),
        ));
    }
    if er.entries == 0 {
        return Ok(unknown(
            lims,
            "the graph has no entry points (entry-points finds none) to walk from".into(),
        ));
    }
    if er.truncated || er.depth_hit {
        let bound = if er.truncated {
            format!("max-nodes {REACH_MAX_NODES}")
        } else {
            format!("depth {REACH_DEPTH}")
        };
        lims.push(Limitation::new(
            "truncated",
            format!("the reach walk from {} entry point(s) stopped at {bound} before finding the cited symbol", er.entries),
        ));
        return Ok(unknown(
            lims,
            format!("the walk stopped at {bound} without reaching the cited symbol"),
        ));
    }
    lims.push(Limitation::new(
        "reach_static_only",
        "unreached means no static call path: calls through dynamic dispatch, registries, decorators, callbacks and framework wiring are not in the graph",
    ));
    Ok(Reach::new(
        "unreached",
        vec![],
        format!(
            "no call path from any of {} entry point(s) within depth {REACH_DEPTH}",
            er.entries
        ),
    ))
}
