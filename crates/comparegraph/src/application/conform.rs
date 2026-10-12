use super::{AppError, Result};
use crate::domain::NodeKind;
use crate::providers::{by_name_in, BuildOptions};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PR {
    pub expected: usize,
    pub found: usize,
    pub matched: usize,
    pub precision: f64,
    pub recall: f64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct FixtureResult {
    pub name: String,
    pub nodes: PR,
    pub edges: PR,
    pub passed: bool,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConformReport {
    pub provider: String,
    pub fixtures: Vec<FixtureResult>,
    pub passed: bool,
}

fn pr(expected: &HashSet<String>, found: &HashSet<String>) -> PR {
    let matched = expected.intersection(found).count();
    let precision = if found.is_empty() {
        0.0
    } else {
        matched as f64 / found.len() as f64
    };
    let recall = if expected.is_empty() {
        0.0
    } else {
        matched as f64 / expected.len() as f64
    };
    PR {
        expected: expected.len(),
        found: found.len(),
        matched,
        precision,
        recall,
    }
}

pub fn conform(provider_name: &str, fixtures_dir: &Path) -> Result<ConformReport> {
    conform_in(Path::new("."), provider_name, fixtures_dir)
}

/// `workspace` locates `comparegraph.toml` for external providers.
pub fn conform_in(workspace: &Path, provider_name: &str, fixtures_dir: &Path) -> Result<ConformReport> {
    let provider = by_name_in(workspace, provider_name)
        .ok_or_else(|| AppError::NotFound(format!("no provider '{provider_name}'")))?;
    let mut dirs: Vec<_> = std::fs::read_dir(fixtures_dir)
        .map_err(|e| AppError::Io(fixtures_dir.to_path_buf(), e))?
        .flatten()
        .filter(|e| e.path().join("expected.json").exists())
        .collect();
    dirs.sort_by_key(|e| e.file_name());
    if dirs.is_empty() {
        return Err(AppError::Empty(format!(
            "no fixtures with expected.json under {}",
            fixtures_dir.display()
        )));
    }
    let mut fixtures = Vec::new();
    for d in dirs {
        let name = d.file_name().to_string_lossy().to_string();
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(d.path().join("expected.json"))
                .map_err(|e| AppError::Io(d.path(), e))?,
        )
        .map_err(|e| AppError::Empty(format!("{name}/expected.json: {e}")))?;
        let exp_nodes: HashSet<String> = expected["nodes"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let exp_edges: HashSet<String> = expected["edges"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let out = provider.build(&d.path(), &BuildOptions::default())?;
        let found_nodes: HashSet<String> = out
            .nodes
            .iter()
            .filter(|n| n.kind != NodeKind::File && n.kind != NodeKind::Test)
            .map(|n| n.qualified_name.clone())
            .collect();
        let found_edges: HashSet<String> = out
            .edges_by_qn
            .iter()
            .map(|(k, s, t, _)| format!("{} {} {}", k.as_str(), s, t))
            .collect();
        let (n, e) = (pr(&exp_nodes, &found_nodes), pr(&exp_edges, &found_edges));
        let passed =
            n.precision >= 0.95 && n.recall >= 0.95 && e.precision >= 0.85 && e.recall >= 0.85;
        fixtures.push(FixtureResult {
            name,
            nodes: n,
            edges: e,
            passed,
        });
    }
    let passed = fixtures.iter().all(|f| f.passed);
    Ok(ConformReport {
        provider: provider_name.into(),
        fixtures,
        passed,
    })
}
