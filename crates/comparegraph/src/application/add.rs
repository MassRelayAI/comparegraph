use super::{git, AppError, Result};
use crate::domain::Limitation;
use crate::store::{RepoRow, Store};
use std::path::{Path, PathBuf};

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
/// Every value `lane_for` can return; `add --lane` accepts only these.
pub const LANES: [&str; 2] = ["take", "study"];

/// `add --licence/--lane/--licence-from`: explicit values that win over what the clone's own files say
/// (a partial export carries no LICENSE file). `licence` and `licence_from` are exclusive.
/// Also the repo notes `--source-commit/--moved-to/--include/--exclude` (field run 2026-11-07 items 6 and 7).
#[derive(Debug, Clone, Default)]
pub struct LicenceOverride {
    pub licence: Option<String>,
    pub lane: Option<String>,
    /// another clone (the full source) whose LICENSE file is read instead
    pub licence_from: Option<PathBuf>,
    /// the upstream commit an export was taken from; `Some("")` clears it
    pub source_commit: Option<String>,
    /// where the project's core now lives (a registered repo name or a URL); `Some("")` clears it
    pub moved_to: Option<String>,
    /// the default subset `build` reads; a lone `""` clears the list
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// A note value as given: `Some("")` clears, otherwise trimmed.
fn note(v: &Option<String>) -> Option<Option<String>> {
    v.as_ref().map(|s| Some(s.trim().to_string()).filter(|s| !s.is_empty()))
}
/// A glob list as given: empty means "not given", `[""]` clears.
fn globs(v: &[String]) -> Option<Vec<String>> {
    match v {
        [] => None,
        [one] if one.is_empty() => Some(vec![]),
        _ => Some(v.to_vec()),
    }
}

impl LicenceOverride {
    /// No licence, lane or licence source given (the notes do not count).
    pub fn is_empty(&self) -> bool {
        self.licence.is_none() && self.lane.is_none() && self.licence_from.is_none()
    }
    /// No note given (`--source-commit/--moved-to/--include/--exclude`).
    pub fn notes_empty(&self) -> bool {
        self.source_commit.is_none() && self.moved_to.is_none() && self.include.is_empty() && self.exclude.is_empty()
    }
    /// Check the notes before anything is written: a commit id is 7 to 64 hex digits, and every glob compiles.
    fn validate_notes(&self) -> Result<()> {
        if let Some(Some(c)) = note(&self.source_commit) {
            if !(7..=64).contains(&c.len()) || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(AppError::InvalidArguments(format!(
                    "--source-commit '{c}' is not a commit id (7 to 64 hex digits); give the upstream sha, not a branch or tag"
                )));
            }
        }
        let inc = globs(&self.include).unwrap_or_default();
        let exc = globs(&self.exclude).unwrap_or_default();
        crate::providers::reference::walker::Subset::new(&inc, &exc).map_err(AppError::InvalidArguments)?;
        Ok(())
    }
    /// Apply the given notes over a row's current ones.
    fn apply_notes(&self, row: &mut RepoRow) {
        if let Some(c) = note(&self.source_commit) {
            row.source_commit = c.map(|c| c.to_ascii_lowercase());
        }
        if let Some(m) = note(&self.moved_to) {
            row.moved_to = m;
        }
        if let Some(i) = globs(&self.include) {
            row.include = i;
        }
        if let Some(e) = globs(&self.exclude) {
            row.exclude = e;
        }
    }
    /// The licence the override names, if any: `--licence` as given, or the one read from `--licence-from`.
    fn licence(&self) -> Result<Option<String>> {
        if self.licence.is_some() && self.licence_from.is_some() {
            return Err(AppError::InvalidArguments("give --licence or --licence-from, not both".into()));
        }
        if let Some(l) = &self.licence {
            if l.trim().is_empty() {
                return Err(AppError::InvalidArguments("--licence is empty; give an SPDX id such as MIT".into()));
            }
            return Ok(Some(l.trim().to_string()));
        }
        let Some(src) = &self.licence_from else { return Ok(None) };
        let src = src.canonicalize().map_err(|e| AppError::Io(src.to_path_buf(), e))?;
        match licence_of(&src) {
            Some(l) => Ok(Some(l)),
            None => Err(AppError::InvalidArguments(format!(
                "--licence-from {}: no recognised licence file there; give --licence <SPDX> instead",
                src.display()
            ))),
        }
    }
    fn lane(&self) -> Result<Option<String>> {
        match &self.lane {
            None => Ok(None),
            Some(l) if LANES.contains(&l.as_str()) => Ok(Some(l.clone())),
            Some(l) => Err(AppError::InvalidArguments(format!(
                "unknown lane '{l}'; a lane is one of: {}",
                LANES.join(", ")
            ))),
        }
    }
    /// Resolve against a base licence: the override's licence wins over `base`; an explicit lane wins over the
    /// licence's lane. Validates before anything is written.
    fn resolve(&self, base: Option<String>) -> Result<(Option<String>, String)> {
        let lane = self.lane()?;
        let licence = self.licence()?.or(base);
        let lane = lane.unwrap_or_else(|| lane_for(licence.as_deref()).to_string());
        Ok((licence, lane))
    }
}

pub fn add_repo(root: &Path, clone: &Path, name: Option<&str>, as_root: bool) -> Result<RepoRow> {
    add_repo_with(root, clone, name, as_root, &LicenceOverride::default())
}

/// `add` with explicit licence/lane overrides.
pub fn add_repo_with(
    root: &Path,
    clone: &Path,
    name: Option<&str>,
    as_root: bool,
    over: &LicenceOverride,
) -> Result<RepoRow> {
    let clone = clone
        .canonicalize()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    // validate the overrides before the store is touched
    let (licence, lane) = over.resolve(licence_of(&clone))?;
    over.validate_notes()?;
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
    row.licence = licence;
    row.lane = lane;
    over.apply_notes(&mut row);
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

/// What `add --replace` did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReplaceReport {
    pub name: String,
    pub old_path: String,
    pub new_path: String,
    /// the git head of the new path
    pub head: Option<String>,
    pub role: String,
    /// the row's licence and lane after the replace: kept unless `--licence/--lane/--licence-from` were given
    pub licence: Option<String>,
    pub lane: String,
    /// the upstream commit of the new path: only when given again (`--source-commit`); the old one named the old checkout
    pub source_commit: Option<String>,
    pub moved_to: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// carried to the envelope's evidence.limitations by the CLI
    pub limitation_items: Vec<Limitation>,
}

/// `add --name <existing>` with no path: set the notes (and licence/lane) of a registered repo; nothing else changes.
pub fn amend_repo(root: &Path, name: &str, over: &LicenceOverride) -> Result<RepoRow> {
    if over.is_empty() && over.notes_empty() {
        return Err(AppError::InvalidArguments(format!(
            "add --name {name} without a path amends a registered repo: give --moved-to, --source-commit, --include, --exclude, --licence or --lane"
        )));
    }
    over.validate_notes()?;
    let store = Store::open_existing(root)?;
    let Some(mut row) = store.repo_by_name(name)? else {
        return Err(AppError::NotFound(format!(
            "no repo '{name}' in {}: give a path to register it",
            store.path().display()
        )));
    };
    let id = row.id.unwrap();
    if !over.is_empty() {
        let (licence, lane) = over.resolve(row.licence.clone())?;
        store.set_repo_licence_lane(id, licence.as_deref(), &lane)?;
        (row.licence, row.lane) = (licence, lane);
    }
    over.apply_notes(&mut row);
    store.set_repo_notes(id, row.source_commit.as_deref(), row.moved_to.as_deref(), &row.include, &row.exclude)?;
    Ok(row)
}

/// `add <path> --name <existing> --replace`: re-point a registered repo at another checkout. Claims and runs stay.
pub fn replace_repo(root: &Path, clone: &Path, name: &str, as_root: bool) -> Result<ReplaceReport> {
    replace_repo_with(root, clone, name, as_root, &LicenceOverride::default())
}

/// `add --replace` with licence/lane overrides: given ones are applied to the row, otherwise licence and lane stay.
pub fn replace_repo_with(
    root: &Path,
    clone: &Path,
    name: &str,
    as_root: bool,
    over: &LicenceOverride,
) -> Result<ReplaceReport> {
    let clone = clone
        .canonicalize()
        .map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    let store = Store::open(root)?;
    let Some(row) = store.repo_by_name(name)? else {
        return Err(AppError::NotFound(format!(
            "no repo '{name}' in {}: nothing to replace; drop --replace",
            store.path().display()
        )));
    };
    let new_path = clone.to_string_lossy().to_string();
    if row.path == new_path {
        return Err(AppError::InvalidArguments(format!("repo '{name}' is already at {new_path}")));
    }
    // validate the overrides before the path moves
    let (licence, lane) = if over.is_empty() {
        (row.licence.clone(), row.lane.clone())
    } else {
        over.resolve(row.licence.clone())?
    };
    over.validate_notes()?;
    let mut notes = row.clone();
    let old_commit = notes.source_commit.take();
    over.apply_notes(&mut notes);
    let mut limitation_items = vec![Limitation::new(
        "path_replaced",
        format!("the last build read {}; run `comparegraph build {name}` to record a run at the new head", row.path),
    )];
    if let (Some(old), None) = (&old_commit, &over.source_commit) {
        limitation_items.push(Limitation::new(
            "source_commit_cleared",
            format!("source_commit {old} named the export at {}; give --source-commit again for the new path", row.path),
        ));
    }
    let head = git(&clone, &["rev-parse", "--short=7", "HEAD"]);
    store
        .set_repo_path(row.id.unwrap(), &new_path, head.as_deref(), as_root.then_some("root"))
        .map_err(|e| match e {
            crate::store::StoreError::Sqlite(ref s)
                if s.to_string().contains("idx_repo_one_root") || s.to_string().contains("repo.role") =>
            {
                AppError::NotFound("this workspace already has a root; a workspace has at most one (spec §2b)".into())
            }
            other => other.into(),
        })?;
    if !over.is_empty() {
        store.set_repo_licence_lane(row.id.unwrap(), licence.as_deref(), &lane)?;
    }
    store.set_repo_notes(row.id.unwrap(), notes.source_commit.as_deref(), notes.moved_to.as_deref(), &notes.include, &notes.exclude)?;
    Ok(ReplaceReport {
        name: name.to_string(),
        old_path: row.path.clone(),
        new_path,
        head,
        role: if as_root { "root".into() } else { row.role.clone() },
        licence,
        lane,
        source_commit: notes.source_commit,
        moved_to: notes.moved_to,
        include: notes.include,
        exclude: notes.exclude,
        limitation_items,
    })
}
