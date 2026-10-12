use super::{git, AppError, Result};
use crate::store::{RepoRow, Store};
use std::path::Path;

pub fn licence_of(clone: &Path) -> Option<String> {
    let dir = std::fs::read_dir(clone).ok()?;
    let mut names: Vec<_> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_uppercase().starts_with("LICEN"))
        .collect();
    names.sort();
    let text = std::fs::read_to_string(clone.join(names.first()?)).ok()?;
    let head: String = text.lines().take(5).collect::<Vec<_>>().join(" ");
    for (needle, id) in [
        ("Apache License", "Apache-2.0"),
        ("MIT License", "MIT"),
        ("BSD", "BSD"),
        ("GNU AFFERO", "AGPL-3.0"),
        ("GNU GENERAL PUBLIC", "GPL-3.0"),
        ("Mozilla Public", "MPL-2.0"),
        ("Unlicense", "Unlicense"),
    ] {
        if head
            .to_ascii_uppercase()
            .contains(&needle.to_ascii_uppercase())
        {
            return Some(id.to_string());
        }
    }
    None
}
pub fn lane_for(licence: Option<&str>) -> &'static str {
    match licence {
        Some("MIT") | Some("Apache-2.0") | Some("BSD") | Some("Unlicense") => "take",
        _ => "study",
    }
}

pub fn add_repo(root: &Path, clone: &Path, name: Option<&str>, as_root: bool) -> Result<RepoRow> {
    let clone = clone
        .canonicalize()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    let name = name.map(String::from).unwrap_or_else(|| {
        clone
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "repo".into())
    });
    let store = Store::open(root)?;
    if store.repo_by_name(&name)?.is_some() {
        return Err(AppError::NotFound(format!(
            "repo '{name}' already exists in {}",
            store.path().display()
        )));
    }
    let mut row = RepoRow::new(&name, &clone.to_string_lossy());
    row.head = git(&clone, &["rev-parse", "--short=7", "HEAD"]);
    row.remote = git(&clone, &["remote", "get-url", "origin"]);
    row.licence = licence_of(&clone);
    row.lane = lane_for(row.licence.as_deref()).to_string();
    if as_root {
        row.role = "root".into();
    }
    let id = store.insert_repo(&row).map_err(|e| match e {
        crate::store::StoreError::Sqlite(ref s)
            if s.to_string().contains("idx_repo_one_root")
                || s.to_string().contains("repo.role") =>
        {
            AppError::NotFound(
                "this workspace already has a root; a workspace has at most one (spec §2b)".into(),
            )
        }
        other => other.into(),
    })?;
    row.id = Some(id);
    Ok(row)
}
