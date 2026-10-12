//! Family packs: a directory with `rows.md` (front matter + one markdown table) becomes a family of features.
//! Parsed by hand; the whole pack is validated before anything is written, and written in one transaction.
use super::{AppError, Result};
use crate::store::{FeatureInput, Store};
use serde::Serialize;
use std::path::Path;

pub const TIERS: [&str; 4] = ["basics", "common", "extra", "candidate"];
const COLS: [&str; 7] = [
    "id",
    "group",
    "feature",
    "definition",
    "counts_as_has",
    "tier",
    "weight",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackRow {
    pub id: String,
    pub group: String,
    pub slug: String,
    pub feature: String,
    pub definition: String,
    pub counts_as_has: String,
    pub tier: String,
    pub weight: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pack {
    pub family: String,
    pub description: String,
    pub rows: Vec<PackRow>,
}

fn bad(msg: String) -> AppError {
    AppError::InvalidArguments(msg)
}
fn token_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

/// Parse the text of a `rows.md`. Errors name the table row (1 = first data row).
pub fn parse_pack(text: &str) -> Result<Pack> {
    let mut lines = text.lines().peekable();
    if lines.next().map(str::trim) != Some("---") {
        return Err(bad("rows.md must start with a `---` front matter".into()));
    }
    let (mut family, mut description) = (String::new(), String::new());
    let mut closed = false;
    for l in lines.by_ref() {
        if l.trim() == "---" {
            closed = true;
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            match k.trim() {
                "family" => family = v.trim().to_string(),
                "description" => description = v.trim().to_string(),
                _ => {}
            }
        }
    }
    if !closed {
        return Err(bad("front matter is not closed with `---`".into()));
    }
    if !token_ok(&family) {
        return Err(bad(format!(
            "front matter `family: {family}` must be lowercase [a-z0-9-]"
        )));
    }
    let table: Vec<&str> = lines
        .skip_while(|l| !l.trim_start().starts_with('|'))
        .take_while(|l| l.trim_start().starts_with('|'))
        .collect();
    if table.len() < 2 {
        return Err(bad("rows.md has no table".into()));
    }
    if cells(table[0]) != COLS {
        return Err(bad(format!("table header must be: {}", COLS.join(" | "))));
    }
    let mut rows: Vec<PackRow> = Vec::new();
    for (i, line) in table.iter().skip(2).enumerate() {
        let n = i + 1;
        let c = cells(line);
        if c.len() != COLS.len() {
            return Err(bad(format!(
                "row {n}: expected {} cells, found {}",
                COLS.len(),
                c.len()
            )));
        }
        let id = c[0].clone();
        let (g, s) = id
            .split_once('.')
            .filter(|(g, s)| token_ok(g) && token_ok(s))
            .ok_or_else(|| {
                bad(format!(
                    "row {n}: id `{id}` must be `<group>.<slug>` (lowercase [a-z0-9-], one dot)"
                ))
            })?;
        if c[1] != g {
            return Err(bad(format!(
                "row {n}: group `{}` does not match id `{id}`",
                c[1]
            )));
        }
        if c[2].is_empty() {
            return Err(bad(format!("row {n}: feature title is empty")));
        }
        if c[3].is_empty() {
            return Err(bad(format!("row {n}: definition is empty")));
        }
        if c[4].is_empty() {
            return Err(bad(format!("row {n}: counts_as_has is empty")));
        }
        if !TIERS.contains(&c[5].as_str()) {
            return Err(bad(format!(
                "row {n}: tier `{}` must be one of {}",
                c[5],
                TIERS.join("|")
            )));
        }
        let weight: i64 = c[6]
            .parse()
            .ok()
            .filter(|w| (1..=3).contains(w))
            .ok_or_else(|| bad(format!("row {n}: weight `{}` must be 1..3", c[6])))?;
        if rows.iter().any(|r| r.id == id) {
            return Err(bad(format!("row {n}: duplicate feature id `{id}`")));
        }
        rows.push(PackRow {
            id: id.clone(),
            group: g.to_string(),
            slug: s.to_string(),
            feature: c[2].clone(),
            definition: c[3].clone(),
            counts_as_has: c[4].clone(),
            tier: c[5].clone(),
            weight,
        });
    }
    if rows.is_empty() {
        return Err(bad("the table has no feature rows".into()));
    }
    Ok(Pack {
        family,
        description,
        rows,
    })
}

#[derive(Debug, Serialize)]
pub struct FamilyAdded {
    pub family: String,
    pub definition_path: String,
    pub features: usize,
    pub added: Vec<String>,
    pub retired: Vec<String>,
}

pub fn add_family(root: &Path, dir: &Path) -> Result<FamilyAdded> {
    let dir = dir
        .canonicalize()
        .map_err(|e| AppError::Io(dir.to_path_buf(), e))?;
    let file = dir.join("rows.md");
    let text = std::fs::read_to_string(&file).map_err(|e| AppError::Io(file.clone(), e))?;
    let pack = parse_pack(&text)?; // validate everything before the store is touched
    let store = Store::open(root)?;
    let before: Vec<String> = store
        .features_of(&pack.family)?
        .iter()
        .filter(|f| f.tier != "retired")
        .map(|f| f.key())
        .collect();
    let tx = store
        .conn()
        .unchecked_transaction()
        .map_err(AppError::from)?;
    let path = dir.to_string_lossy().to_string();
    let fid = store.upsert_family(&pack.family, &path, &pack.description)?;
    for r in &pack.rows {
        store.upsert_feature(
            fid,
            &FeatureInput {
                id: &r.id,
                grp: &r.group,
                title: &r.feature,
                definition: &r.definition,
                counts_as_has: &r.counts_as_has,
                tier: &r.tier,
                weight: r.weight,
            },
        )?;
    }
    let keep: Vec<String> = pack.rows.iter().map(|r| r.id.clone()).collect();
    let retired = store.retire_missing(&pack.family, &keep)?;
    tx.commit().map_err(AppError::from)?;
    let added = keep
        .iter()
        .filter(|k| !before.contains(k))
        .cloned()
        .collect();
    Ok(FamilyAdded {
        family: pack.family,
        definition_path: path,
        features: keep.len(),
        added,
        retired,
    })
}

#[derive(Debug, Serialize)]
pub struct FamilySummary {
    pub name: String,
    pub definition_path: String,
    pub description: String,
    pub active: usize,
    pub retired: usize,
    pub groups: usize,
}
pub fn list_families(root: &Path) -> Result<Vec<FamilySummary>> {
    let store = Store::open_existing(root)?;
    let mut out = Vec::new();
    for f in store.families()? {
        let feats = store.features_of(&f.name)?;
        let retired = feats.iter().filter(|x| x.tier == "retired").count();
        let mut groups: Vec<&str> = feats
            .iter()
            .filter(|x| x.tier != "retired")
            .map(|x| x.grp.as_str())
            .collect();
        groups.dedup();
        out.push(FamilySummary {
            name: f.name,
            definition_path: f.definition_path,
            description: f.description,
            active: feats.len() - retired,
            retired,
            groups: groups.len(),
        });
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct FamilyShown {
    pub name: String,
    pub definition_path: String,
    pub description: String,
    pub groups: Vec<GroupShown>,
}
#[derive(Debug, Serialize)]
pub struct GroupShown {
    pub group: String,
    pub features: Vec<crate::store::FeatureRow>,
}
pub fn show_family(root: &Path, name: &str) -> Result<FamilyShown> {
    let store = Store::open_existing(root)?;
    let fam = store
        .family_by_name(name)?
        .ok_or_else(|| AppError::NotFound(format!("no family named {name}")))?;
    let mut groups: Vec<GroupShown> = Vec::new();
    for f in store.features_of(name)? {
        match groups.last_mut() {
            Some(g) if g.group == f.grp => g.features.push(f),
            _ => groups.push(GroupShown {
                group: f.grp.clone(),
                features: vec![f],
            }),
        }
    }
    Ok(FamilyShown {
        name: fam.name,
        definition_path: fam.definition_path,
        description: fam.description,
        groups,
    })
}
