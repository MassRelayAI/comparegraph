//! Vault export: a deterministic Obsidian-style markdown vault from each repo's latest done run.
//! Wikilinks are vault-relative without `.md` (`[[repos/x]]`). No timestamps anywhere. Written whole into a temp dir, then swapped in.
use super::{AppError, Result};
use crate::domain::{Node, NodeId, NodeKind};
use crate::store::Store;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct VaultReport {
    pub repos: usize,
    pub files: usize,
    pub communities: usize,
    pub symbols: usize,
}

const MAX_NEIGHBOURS: usize = 50;

/// lowercase, non `[a-z0-9]` -> `-`, collapsed, trimmed, max 80 chars
pub fn slug(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.ends_with('-') {
            o.push('-');
        }
    }
    let t = o.trim_matches('-');
    let t: String = t.chars().take(80).collect();
    let t = t.trim_matches('-').to_string();
    if t.is_empty() {
        "x".into()
    } else {
        t
    }
}

/// unique slugs in the given (already sorted) order: collisions get -2, -3
fn unique_slugs<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for k in keys {
        let base = slug(k);
        let mut cand = base.clone();
        let mut n = 2;
        while seen.contains(&cand) {
            let suffix = format!("-{n}");
            let keep: String = base
                .chars()
                .take(80usize.saturating_sub(suffix.len()))
                .collect();
            cand = format!("{}{suffix}", keep.trim_end_matches('-'));
            n += 1;
        }
        seen.insert(cand.clone());
        out.push(cand);
    }
    out
}

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
fn qlist(v: &[String]) -> String {
    serde_json::to_string(v).unwrap()
}
/// `description: "..."` front-matter line: one line, at most 160 chars, YAML-quoted.
fn desc(s: &str) -> String {
    let one = s.replace(['\n', '\r'], " ");
    let t: String = if one.chars().count() > 160 {
        one.chars().take(159).collect::<String>() + "\u{2026}"
    } else {
        one
    };
    format!("description: {}", q(&t))
}
fn is_symbol(k: NodeKind) -> bool {
    matches!(
        k,
        NodeKind::Class | NodeKind::Function | NodeKind::Method | NodeKind::Type | NodeKind::Route
    )
}
fn is_file(k: NodeKind) -> bool {
    matches!(k, NodeKind::File | NodeKind::Test)
}
fn cohesion_str(c: Option<f64>) -> String {
    c.map(|c| format!("{c:.3}"))
        .unwrap_or_else(|| "null".into())
}

fn put(root: &Path, rel: &str, body: &str, n: &mut usize) -> Result<()> {
    let p = root.join(rel);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| AppError::Io(d.to_path_buf(), e))?;
    }
    std::fs::write(&p, body).map_err(|e| AppError::Io(p.clone(), e))?;
    *n += 1;
    Ok(())
}

pub fn export_vault(root: &Path, out: &Path) -> Result<VaultReport> {
    let version = env!("CARGO_PKG_VERSION");
    // refuse early to overwrite anything that is not a comparegraph vault
    if out.exists() && !is_comparegraph_vault(out) {
        return Err(AppError::Io(
            out.to_path_buf(),
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists and is not a comparegraph vault (no index.md with generated_by: comparegraph); refusing to replace it"),
        ));
    }
    let store = Store::open_existing(root)?;
    let mut repos = store.repos()?;
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    let mut work = Vec::new();
    for r in repos {
        if let Some(run) = store.latest_done_run(r.id.unwrap())? {
            work.push((r, run));
        }
    }
    let total: usize = work
        .iter()
        .map(|(_, run)| {
            store
                .nodes_of_run(run.id.unwrap())
                .map(|n| n.len())
                .unwrap_or(0)
        })
        .sum();
    if total == 0 {
        return Err(AppError::Empty(
            "vault export has 0 nodes: build first".into(),
        ));
    }
    let tmp = {
        let mut n = out.as_os_str().to_os_string();
        n.push(format!(".tmp-{}", std::process::id()));
        PathBuf::from(n)
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let res =
        write_all(&store, root, &work, &tmp, version).and_then(|rep| swap(&tmp, out).map(|_| rep));
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    res
}

fn is_comparegraph_vault(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("index.md"))
        .map(|s| s.contains("generated_by: comparegraph"))
        .unwrap_or(false)
}

fn swap(tmp: &Path, out: &Path) -> Result<()> {
    if out.exists() {
        if !is_comparegraph_vault(out) {
            return Err(AppError::Io(
                out.to_path_buf(),
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "not a comparegraph vault; left untouched",
                ),
            ));
        }
        std::fs::remove_dir_all(out).map_err(|e| AppError::Io(out.to_path_buf(), e))?;
    }
    std::fs::rename(tmp, out).map_err(|e| AppError::Io(out.to_path_buf(), e))
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn write_all(
    store: &Store,
    root: &Path,
    work: &[(crate::store::RepoRow, crate::domain::Run)],
    dir: &Path,
    version: &str,
) -> Result<VaultReport> {
    std::fs::create_dir_all(dir).map_err(|e| AppError::Io(dir.to_path_buf(), e))?;
    let mut nfiles = 0usize;
    let (mut ncomm, mut nsym) = (0usize, 0usize);
    let mut index_rows = String::new();
    let mut llms_links = String::new();
    let repo_slugs = unique_slugs(work.iter().map(|(r, _)| r.name.as_str()));
    let mut families = store.families()?;
    families.sort_by(|a, b| a.name.cmp(&b.name));
    let fam_slugs = unique_slugs(families.iter().map(|f| f.name.as_str()));
    let fam_slug: HashMap<&str, &str> = families
        .iter()
        .map(|f| f.name.as_str())
        .zip(fam_slugs.iter().map(|s| s.as_str()))
        .collect();
    let mut sym_by_node: HashMap<(String, i64), String> = HashMap::new();
    let mut history_repos: Vec<(String, String)> = Vec::new();
    for ((r, run), rslug) in work.iter().zip(&repo_slugs) {
        let run_id = run.id.unwrap();
        let mut nodes = store.nodes_of_run(run_id)?;
        nodes.sort_by(|a, b| {
            a.qualified_name
                .cmp(&b.qualified_name)
                .then(a.path.cmp(&b.path))
                .then(a.line_start.cmp(&b.line_start))
        });
        let by_id: HashMap<NodeId, &Node> = nodes.iter().map(|n| (n.id.unwrap(), n)).collect();
        // symbol slugs in qualified-name order
        let syms: Vec<&Node> = nodes.iter().filter(|n| is_symbol(n.kind)).collect();
        let sslugs = unique_slugs(syms.iter().map(|n| n.qualified_name.as_str()));
        let sym_link: HashMap<NodeId, String> = syms
            .iter()
            .zip(&sslugs)
            .map(|(n, s)| (n.id.unwrap(), format!("symbols/{rslug}/{s}")))
            .collect();
        for (n, l) in syms.iter().zip(&sslugs) {
            sym_by_node.insert(
                (r.name.clone(), n.id.unwrap().0),
                format!("symbols/{rslug}/{l}"),
            );
        }
        let mut comms = store.communities_of_run(run_id)?;
        comms.sort_by(|a, b| a.label.cmp(&b.label));
        let cslugs = unique_slugs(comms.iter().map(|c| c.label.as_str()));
        let mut node_comm: HashMap<NodeId, usize> = HashMap::new();
        for (i, c) in comms.iter().enumerate() {
            for m in &c.members {
                node_comm.entry(*m).or_insert(i);
            }
        }
        let file_comm: HashMap<&str, usize> = nodes
            .iter()
            .filter(|n| is_file(n.kind))
            .filter_map(|f| node_comm.get(&f.id.unwrap()).map(|c| (f.path.as_str(), *c)))
            .collect();
        let edges = store.edges_of_run(run_id)?;
        let n_files = nodes.iter().filter(|n| is_file(n.kind)).count();

        // repo note
        let n_claims = super::claim::claims(root, Some(&r.name), None, None, false)?.len();
        let latest = super::decide::decisions(root, Some(&r.name), false)?
            .into_iter()
            .find(|d| d.scope == "repo");
        let has_history = !super::claim::claims(root, Some(&r.name), None, None, true)?.is_empty()
            || !super::decide::decisions(root, Some(&r.name), true)?.is_empty();
        let rdesc = desc(&format!(
            "{} \u{2014} {}, {}, {} files, {} clusters, {} claims, decision {}",
            r.name,
            r.licence.as_deref().unwrap_or("licence unknown"),
            r.lane,
            n_files,
            comms.len(),
            n_claims,
            latest
                .as_ref()
                .map(|d| d.disposition.as_str())
                .unwrap_or("undecided")
        ));
        let mut s = String::new();
        let _ = writeln!(s, "---\ntype: repo\n{rdesc}\nname: {}\nlicence: {}\nlane: {}\nrole: {}\nhead: {}\nfiles: {}\nnodes: {}\nedges: {}\ncommunities: {}\n---",
            q(&r.name), r.licence.as_deref().map(q).unwrap_or_else(|| "null".into()), q(&r.lane), q(&r.role),
            r.head.as_deref().map(q).unwrap_or_else(|| "null".into()), n_files, nodes.len(), edges.len(), comms.len());
        let _ = writeln!(
            s,
            "\n# {}\n\nLicence {} (lane {}), role {}.\n\n## Communities\n",
            r.name,
            r.licence.as_deref().unwrap_or("unknown"),
            r.lane,
            r.role
        );
        for (c, cs) in comms.iter().zip(&cslugs) {
            let _ = writeln!(
                s,
                "- [[communities/{rslug}/{cs}]] size {} cohesion {}",
                c.members.len(),
                cohesion_str(c.cohesion)
            );
        }
        let rel = super::map::latest_relatedness(root, Some(&r.name), None)?;
        let _ = writeln!(s, "\n## Relatedness\n");
        if rel.is_empty() {
            let _ = writeln!(s, "- none scored");
        }
        for x in &rel {
            let fl = fam_slug
                .get(x.family.as_str())
                .map(|f| format!("[[families/{f}]]"))
                .unwrap_or_else(|| x.family.clone());
            let _ = writeln!(s, "- {fl} {} ({:.2})", x.band, x.score);
        }
        let _ = writeln!(s, "\n## Claims and decision\n\n- claims: {n_claims}");
        match latest {
            Some(d) => {
                let _ = writeln!(
                    s,
                    "- decision: {} by {}: {}",
                    d.disposition,
                    d.decided_by,
                    cell(&d.why)
                );
            }
            None => {
                let _ = writeln!(s, "- decision: none");
            }
        }
        if has_history {
            let _ = writeln!(s, "- history: [[history/{rslug}]]");
            history_repos.push((r.name.clone(), rslug.clone()));
        }
        put(dir, &format!("repos/{rslug}.md"), &s, &mut nfiles)?;
        let _ = writeln!(
            index_rows,
            "| [[repos/{rslug}]] | {} | {} | {} | {} | {} |",
            r.licence.as_deref().unwrap_or("unknown"),
            r.lane,
            nodes.len(),
            edges.len(),
            comms.len()
        );
        let _ = writeln!(
            llms_links,
            "- [{}](repos/{rslug}.md): {} nodes, {} communities, licence {}",
            r.name,
            nodes.len(),
            comms.len(),
            r.licence.as_deref().unwrap_or("unknown")
        );

        // community notes
        for (c, cs) in comms.iter().zip(&cslugs) {
            let mut members: Vec<&Node> = c
                .members
                .iter()
                .filter_map(|m| by_id.get(m).copied())
                .collect();
            members.sort_by(|a, b| {
                a.path
                    .cmp(&b.path)
                    .then(a.line_start.cmp(&b.line_start))
                    .then(a.qualified_name.cmp(&b.qualified_name))
            });
            let mut s = String::new();
            let cd = desc(&format!(
                "{} \u{2014} {} files, keywords {}, density {}",
                c.label,
                members.iter().filter(|n| is_file(n.kind)).count(),
                c.keywords
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                c.cohesion
                    .map(|x| format!("{x:.2}"))
                    .unwrap_or_else(|| "n/a".into())
            ));
            let _ = writeln!(s, "---\ntype: community\n{cd}\nrepo: {}\nlabel: {}\nsize: {}\ncohesion: {}\nkeywords: {}\n---",
                q(&r.name), q(&c.label), c.members.len(), cohesion_str(c.cohesion), qlist(&c.keywords));
            let _ = writeln!(
                s,
                "\n# {}\n\nParent: [[repos/{rslug}]]\n\n## Files\n",
                c.label
            );
            for n in members.iter().filter(|n| is_file(n.kind)) {
                let _ = writeln!(s, "- {}", n.path);
            }
            let _ = writeln!(s, "\n## Symbols\n");
            for n in members.iter().filter(|n| is_symbol(n.kind)) {
                let _ = writeln!(
                    s,
                    "- [[{}]] {} {}:{}",
                    sym_link[&n.id.unwrap()],
                    n.kind.as_str(),
                    n.path,
                    n.line_start
                );
            }
            put(
                dir,
                &format!("communities/{rslug}/{cs}.md"),
                &s,
                &mut nfiles,
            )?;
            ncomm += 1;
        }

        // caller/callee maps
        let mut callers: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut callees: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for e in edges
            .iter()
            .filter(|e| e.kind == crate::domain::EdgeKind::Calls)
        {
            callees.entry(e.source).or_default().push(e.target);
            callers.entry(e.target).or_default().push(e.source);
        }
        let render = |ids: Option<&Vec<NodeId>>| -> String {
            let mut items: BTreeMap<String, String> = BTreeMap::new();
            for id in ids.into_iter().flatten() {
                if let Some(n) = by_id.get(id) {
                    let line = match sym_link.get(id) {
                        Some(l) => format!("- [[{l}]]"),
                        None => format!("- {}", n.qualified_name),
                    };
                    items.insert(n.qualified_name.clone(), line);
                }
            }
            let mut o = String::new();
            for l in items.values().take(MAX_NEIGHBOURS) {
                o.push_str(l);
                o.push('\n');
            }
            if o.is_empty() {
                "- none\n".into()
            } else {
                o
            }
        };
        // symbol notes
        for n in &syms {
            let id = n.id.unwrap();
            // fall back to the community of the containing file
            let comm = node_comm
                .get(&id)
                .or_else(|| file_comm.get(n.path.as_str()));
            let comm_link = comm.map(|i| format!("communities/{rslug}/{}", cslugs[*i]));
            let mut s = String::new();
            let sd = desc(&format!(
                "{} {} in {}:{}-{}, {} callers, {} callees",
                n.kind.as_str(),
                n.qualified_name,
                n.path,
                n.line_start,
                n.line_end,
                callers.get(&id).map_or(0, |v| v.len()),
                callees.get(&id).map_or(0, |v| v.len())
            ));
            let _ = writeln!(s, "---\ntype: symbol\n{sd}\nrepo: {}\nkind: {}\npath: {}\nline_start: {}\nline_end: {}\ncommunity: {}\ncontent_hash: {}\n---",
                q(&r.name), n.kind.as_str(), q(&n.path), n.line_start, n.line_end,
                comm.map(|i| q(&comms[*i].label)).unwrap_or_else(|| "null".into()), q(&n.content_hash));
            let _ = writeln!(s, "\n# {}\n\nIn [[repos/{rslug}]]{}. Snippets are not stored: run `comparegraph show {} {}`.\n\n## Callers\n\n{}\n## Callees\n\n{}",
                n.qualified_name, comm_link.map(|l| format!(", community [[{l}]]")).unwrap_or_default(), r.name, n.qualified_name,
                render(callers.get(&id)), render(callees.get(&id)));
            put(dir, &format!("{}.md", sym_link[&id]), &s, &mut nfiles)?;
            nsym += 1;
        }
    }
    // family and feature notes
    let mut fam_links = String::new();
    let mut feature_path: HashMap<(String, String), String> = HashMap::new();
    let repo_link: HashMap<&str, &String> = work
        .iter()
        .map(|(r, _)| r.name.as_str())
        .zip(&repo_slugs)
        .collect();
    for f in &families {
        let fs = fam_slug[f.name.as_str()];
        let m = super::matrix::matrix(root, &f.name)?;
        let feats: Vec<_> = store
            .features_of(&f.name)?
            .into_iter()
            .filter(|x| x.tier != "retired")
            .collect();
        let fslugs = unique_slugs(
            feats
                .iter()
                .map(|x| x.key())
                .collect::<Vec<_>>()
                .iter()
                .map(|k| k.as_str()),
        );
        let flink: HashMap<String, String> = feats
            .iter()
            .map(|x| x.key())
            .zip(fslugs.iter().cloned())
            .collect();
        let claimed = (0..m.columns.len())
            .filter(|i| m.rows.iter().any(|r| r.cells[*i] != "-"))
            .count();
        let gaps = if m.rows.iter().any(|r| r.gap.is_some()) {
            format!(
                ", {} gaps",
                m.rows.iter().filter(|r| r.gap == Some(true)).count()
            )
        } else {
            String::new()
        };
        let fd = desc(&format!(
            "{} \u{2014} {} features, {} repos claimed{gaps}",
            f.name,
            feats.len(),
            claimed
        ));
        for x in &feats {
            feature_path.insert(
                (f.name.clone(), x.key()),
                format!("features/{fs}/{}", flink[&x.key()]),
            );
        }
        let mut s = String::new();
        let _ = writeln!(
            s,
            "---\ntype: family\n{fd}\nname: {}\nfeatures: {}\nrepos: {}\n---\n\n# {}\n\n{}\n",
            q(&f.name),
            feats.len(),
            m.columns.len(),
            f.name,
            f.description
        );
        let head: Vec<String> = m
            .columns
            .iter()
            .map(|c| match repo_link.get(c.repo.as_str()) {
                Some(l) => format!("[[repos/{l}]]"),
                None => c.repo.clone(),
            })
            .collect();
        let _ = writeln!(
            s,
            "| feature | tier | weight | {} | prevalence |",
            head.join(" | ")
        );
        let _ = writeln!(s, "|---|---|---|{}---|", "---|".repeat(head.len()));
        for row in &m.rows {
            let title = feats
                .iter()
                .find(|x| x.key() == row.id)
                .map(|x| x.title.as_str())
                .unwrap_or(&row.id);
            let prev = row
                .prevalence
                .map(|p| format!("{p:.2}"))
                .unwrap_or_else(|| "-".into());
            let _ = writeln!(
                s,
                "| [[features/{fs}/{}]] {} | {} | {} | {} | {prev} |",
                flink[&row.id],
                cell(title),
                row.tier,
                row.weight,
                row.cells.join(" | ")
            );
        }
        put(dir, &format!("families/{fs}.md"), &s, &mut nfiles)?;
        let _ = writeln!(
            fam_links,
            "- [[families/{fs}]] {} features, {} repos",
            feats.len(),
            m.columns.len()
        );
        let all = super::claim::claims(root, None, Some(&f.name), None, false)?;
        for x in &feats {
            let key = x.key();
            let cells: &[String] = m
                .rows
                .iter()
                .find(|r| r.id == key)
                .map(|r| r.cells.as_slice())
                .unwrap_or(&[]);
            let cnt = |v: &str| cells.iter().filter(|c| c.as_str() == v).count();
            let unk = cells
                .iter()
                .filter(|c| !["has", "partial", "none"].contains(&c.as_str()))
                .count();
            let xd = desc(&format!(
                "{} \u{2014} {}: {} has, {} partial, {} none, {} unknown/unclaimed",
                key,
                x.title,
                cnt("has"),
                cnt("partial"),
                cnt("none"),
                unk
            ));
            let mut s = String::new();
            let _ = writeln!(
                s,
                "---\ntype: feature\n{xd}\nfamily: {}\nid: {}\ngroup: {}\ntier: {}\nweight: {}\n---\n",
                q(&f.name),
                q(&key),
                q(&x.grp),
                q(&x.tier),
                x.weight
            );
            let _ = writeln!(s, "# {}\n\nFamily: [[families/{fs}]]\n\n## Definition\n\n{}\n\n## Counts as has\n\n{}\n\n## Per repo\n", x.title, x.definition, x.counts_as_has);
            let mut any = false;
            for c in all.iter().filter(|c| c.feature == key) {
                any = true;
                let rl = match repo_link.get(c.repo.as_str()) {
                    Some(l) => format!("[[repos/{l}]]"),
                    None => c.repo.clone(),
                };
                let stale = c.locations.iter().any(|l| l.stale);
                let _ = writeln!(
                    s,
                    "- {rl}: {} (by {}){}",
                    if stale {
                        "unknown*"
                    } else {
                        c.verdict.as_str()
                    },
                    c.author,
                    if c.note.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", cell(&c.note))
                    }
                );
                for l in &c.locations {
                    match l
                        .node_id
                        .and_then(|n| sym_by_node.get(&(c.repo.clone(), n)))
                    {
                        Some(sl) => {
                            let _ = writeln!(
                                s,
                                "  - [[{sl}]] {}:{}-{}",
                                l.path, l.line_start, l.line_end
                            );
                        }
                        None => {
                            let _ = writeln!(s, "  - {}:{}-{}", l.path, l.line_start, l.line_end);
                        }
                    }
                }
            }
            if !any {
                let _ = writeln!(s, "- no claims yet");
            }
            put(
                dir,
                &format!("features/{fs}/{}.md", flink[&key]),
                &s,
                &mut nfiles,
            )?;
        }
    }
    let mut feat_unique: HashMap<&str, Vec<&String>> = HashMap::new();
    for ((_, k), p) in &feature_path {
        feat_unique.entry(k.as_str()).or_default().push(p);
    }
    for (name, rslug) in &history_repos {
        let mut cl = super::claim::claims(root, Some(name), None, None, true)?;
        cl.sort_by(|a, b| (&a.created_at, a.id).cmp(&(&b.created_at, b.id)));
        let mut dl = super::decide::decisions(root, Some(name), true)?;
        dl.sort_by(|a, b| (&a.decided_at, a.id).cmp(&(&b.decided_at, b.id)));
        let latest = cl
            .iter()
            .map(|c| c.created_at.as_str())
            .chain(dl.iter().map(|d| d.decided_at.as_str()))
            .max()
            .map(|d| d.chars().take(10).collect::<String>())
            .unwrap_or_else(|| "none".into());
        let hd = desc(&format!(
            "{name} \u{2014} {} claim rows, {} decision rows, latest {latest}",
            cl.len(),
            dl.len()
        ));
        let mut s = String::new();
        let _ = writeln!(
            s,
            "---\ntype: history\n{hd}\nrepo: {}\n---\n\n# History of {name}\n\nRepo: [[repos/{rslug}]]\n\n## Claims\n",
            q(name)
        );
        for c in &cl {
            let fl = match feature_path.get(&(c.family.clone(), c.feature.clone())) {
                Some(p) => format!("[[{p}]]"),
                None => c.feature.clone(),
            };
            let by = cl.iter().find(|x| x.supersedes_id == Some(c.id));
            let locs: Vec<String> = c
                .locations
                .iter()
                .map(|l| format!("{}:{}-{}", l.path, l.line_start, l.line_end))
                .collect();
            let _ = writeln!(
                s,
                "- #{} {} | {fl} | {} | {} | {} | {} | {}",
                c.id,
                c.created_at,
                c.verdict,
                c.confidence,
                c.author,
                by.map(|x| format!("superseded by #{}", x.id))
                    .unwrap_or_else(|| "current".into()),
                if locs.is_empty() {
                    "no locations".into()
                } else {
                    locs.join(", ")
                }
            );
        }
        if cl.is_empty() {
            let _ = writeln!(s, "- none");
        }
        let _ = writeln!(s, "\n## Decisions\n");
        for d in &dl {
            let target = match &d.feature {
                Some(f) => match feat_unique.get(f.as_str()) {
                    Some(v) if v.len() == 1 => format!("feature [[{}]]", v[0]),
                    _ => format!("feature {f}"),
                },
                None => "repo".into(),
            };
            let by = dl.iter().find(|x| x.supersedes_id == Some(d.id));
            let _ = writeln!(
                s,
                "- #{} {} | {target} | {} | {} | by {} | {}",
                d.id,
                d.decided_at,
                d.disposition,
                cell(&d.why),
                d.decided_by,
                by.map(|x| format!("superseded by #{}", x.id))
                    .unwrap_or_else(|| "current".into())
            );
        }
        if dl.is_empty() {
            let _ = writeln!(s, "- none");
        }
        put(dir, &format!("history/{rslug}.md"), &s, &mut nfiles)?;
    }
    let fam_section = if fam_links.is_empty() {
        String::new()
    } else {
        format!("\n## Families\n\n{fam_links}")
    };
    let idesc = desc(&format!(
        "CompareGraph vault \u{2014} {} repos, {} communities, {} symbols, {} families, {} history notes",
        work.len(),
        ncomm,
        nsym,
        families.len(),
        history_repos.len()
    ));
    let groups = format!(
        "\n## Notes\n\n- repos/: {} (one per repo)\n- communities/: {}\n- symbols/: {}\n- families/: {}\n- features/: {}\n- history/: {} (dated claim and decision trails)\n",
        work.len(), ncomm, nsym, families.len(), feature_path.len(), history_repos.len()
    );
    let index = format!("---\ntype: index\n{idesc}\nrepos: {}\ngenerated_by: comparegraph {version}\nschema: 1\n---\n\n# CompareGraph vault\n{groups}\n| repo | licence | lane | nodes | edges | communities |\n|---|---|---|---|---|---|\n{index_rows}{fam_section}", work.len());
    put(dir, "index.md", &index, &mut nfiles)?;
    let llms = format!("# CompareGraph vault\n\n> Deterministic knowledge graph of {} cloned repositories (communities and symbols), generated by comparegraph {version}.\n\n- [index](index.md)\n- [agent guide](AGENTS.md)\n\n## Repositories\n\n{llms_links}", work.len());
    put(dir, "llms.txt", &llms, &mut nfiles)?;
    let agents = "# Reading this vault\n\n1. Open `index.md` first: it lists every repo with licence and lane.\n2. Open `repos/<repo>.md` for a repo's communities (module clusters), then `communities/<repo>/<slug>.md` for its files and symbols.\n3. `families/<family>.md` holds the matrix of a taxonomy against every repo, and `features/<family>/<feature>.md` one feature with its claims and citations.
4. `symbols/<repo>/<slug>.md` holds the location, content hash, callers and callees of one symbol.\n5. The vault stores no source code. To read a symbol's lines, run `comparegraph show <repo> <qualified-name> --detail full`.\n6. Respect each repo's licence lane before copying anything from it.\n7. Links are vault-relative wikilinks without `.md`. The vault is regenerated whole; edits are lost.
8. `history/<repo>.md` holds the dated trail of claims and decisions of one repo.

## How to work with this vault

- Read `index.md` first, then triage by the one-line `description:` in each note's front matter; open only what you need.
- Run `comparegraph check` before trusting a verdict: a claim turns stale only after `check` has run.
- The workflow is `relate \u{2192} map \u{2192} claim (with --at citations and --author) \u{2192} matrix \u{2192} decide` (decide is a person's act).
- `relate` scores a repo against a family, `map` lists candidate code per feature, `claim` records a verdict with `--at path:start-end` citations.
- `matrix` compares every repo on one family; `decide` records adopt, reject or park with a reason.
- Code bodies come from `comparegraph show <repo> <qualified-name> --detail full`, never from the vault.
- Past verdicts and decisions are in `history/<repo>.md`; the newest row of each chain is the current one.
- The vault is regenerated by `comparegraph export vault`; never edit it.
";
    put(dir, "AGENTS.md", agents, &mut nfiles)?;
    Ok(VaultReport {
        repos: work.len(),
        files: nfiles,
        communities: ncomm,
        symbols: nsym,
    })
}
