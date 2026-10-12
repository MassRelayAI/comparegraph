//! Communities: connected components of the file graph (imports + cross-file calls), per run. Deterministic.
use crate::domain::{Edge, EdgeKind, Node, NodeId, NodeKind};
use crate::store::CommunityRow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const STOP: [&str; 10] = [
    "get", "set", "new", "the", "and", "for", "with", "from", "init", "test",
];

fn find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

pub(crate) fn tokens(name: &str) -> Vec<String> {
    let mut out = vec![];
    for part in name.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = part.chars().collect();
        let mut cur = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let boundary = i > 0
                && c.is_uppercase()
                && (chars[i - 1].is_lowercase()
                    || chars[i - 1].is_ascii_digit()
                    || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out.into_iter()
        .map(|t| t.to_lowercase())
        .filter(|t| t.chars().count() >= 3 && !STOP.contains(&t.as_str()))
        .collect()
}

fn label_of(paths: &[&str]) -> String {
    if paths.len() == 1 {
        return paths[0].to_string();
    }
    let dirs: Vec<Vec<&str>> = paths
        .iter()
        .map(|p| {
            let mut s: Vec<&str> = p.split('/').collect();
            s.pop();
            s
        })
        .collect();
    let mut common = dirs[0].clone();
    for d in &dirs[1..] {
        let n = common.iter().zip(d).take_while(|(a, b)| a == b).count();
        common.truncate(n);
    }
    let l = common.join("/");
    let l = l.trim_end_matches('/');
    if !l.is_empty() && l != "." {
        return l.to_string();
    }
    let mut freq: BTreeMap<&str, usize> = BTreeMap::new();
    for p in paths {
        *freq.entry(p.split('/').next().unwrap_or(p)).or_default() += 1;
    }
    freq.into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(k, _)| k.to_string())
        .unwrap_or_else(|| paths[0].to_string())
}

/// A component holding more than 30% of the run's files (and at least 8) is split by the directory one level
/// below its common prefix; files directly in the prefix form their own group. Repeats on groups still over the bound.
fn split_big(group: Vec<usize>, files: &[&Node], total: usize) -> Vec<Vec<usize>> {
    if group.len() < 8 || group.len() * 10 <= total * 3 {
        return vec![group];
    }
    let dirs: Vec<Vec<&str>> = group
        .iter()
        .map(|&i| {
            let mut d: Vec<&str> = files[i].path.split('/').collect();
            d.pop();
            d
        })
        .collect();
    let mut common = dirs[0].clone();
    for d in &dirs[1..] {
        let n = common.iter().zip(d).take_while(|(a, b)| a == b).count();
        common.truncate(n);
    }
    let mut by: BTreeMap<Option<&str>, Vec<usize>> = BTreeMap::new();
    for (k, &i) in group.iter().enumerate() {
        by.entry(dirs[k].get(common.len()).copied())
            .or_default()
            .push(i);
    }
    if by.len() < 2 {
        return vec![group];
    }
    by.into_values()
        .flat_map(|g| split_big(g, files, total))
        .collect()
}

/// Every file node lands in exactly one community; symbols follow their file by path.
pub fn compute(nodes: &[Node], edges: &[Edge]) -> Vec<CommunityRow> {
    let mut files: Vec<&Node> = nodes.iter().filter(|n| n.kind == NodeKind::File).collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let idx: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.as_str(), i))
        .collect();
    let file_of: HashMap<NodeId, usize> = nodes
        .iter()
        .filter_map(|n| Some((n.id?, *idx.get(n.path.as_str())?)))
        .collect();
    let mut pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for e in edges {
        if !matches!(e.kind, EdgeKind::Imports | EdgeKind::Calls) {
            continue;
        }
        if let (Some(&a), Some(&b)) = (file_of.get(&e.source), file_of.get(&e.target)) {
            if a != b {
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    let mut parent: Vec<usize> = (0..files.len()).collect();
    for &(a, b) in &pairs {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    // components keyed by root (= smallest file index = smallest path), so iteration is path order
    let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..files.len() {
        let r = find(&mut parent, i);
        comps.entry(r).or_default().push(i);
    }
    let mut members_by_file: HashMap<usize, Vec<&Node>> = HashMap::new();
    for n in nodes {
        if let Some(&i) = idx.get(n.path.as_str()) {
            members_by_file.entry(i).or_default().push(n);
        }
    }
    let groups: Vec<Vec<usize>> = comps
        .into_values()
        .flat_map(|fs| split_big(fs, &files, files.len()))
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = vec![];
    for fs in groups {
        let paths: Vec<&str> = fs.iter().map(|&i| files[i].path.as_str()).collect();
        let mut label = label_of(&paths);
        let c = seen.entry(label.clone()).or_default();
        *c += 1;
        if *c > 1 {
            label = format!("{label} ({c})");
        }
        let n = fs.len();
        let cohesion = if n < 2 {
            None
        } else {
            let set: BTreeSet<usize> = fs.iter().copied().collect();
            let internal = pairs
                .iter()
                .filter(|&&(a, b)| set.contains(&a) && set.contains(&b))
                .count();
            Some(internal as f64 / (n * (n - 1) / 2) as f64)
        };
        let mut members: Vec<&Node> = fs
            .iter()
            .flat_map(|i| members_by_file.get(i).into_iter().flatten().copied())
            .collect();
        members.sort_by_key(|n| n.id);
        let mut freq: BTreeMap<String, usize> = BTreeMap::new();
        for n in members.iter().filter(|n| n.kind != NodeKind::File) {
            for t in tokens(&n.name) {
                *freq.entry(t).or_default() += 1;
            }
        }
        let mut kw: Vec<(String, usize)> = freq.into_iter().collect();
        kw.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.push(CommunityRow {
            id: 0,
            label,
            keywords: kw.into_iter().take(5).map(|(k, _)| k).collect(),
            cohesion,
            members: members.iter().filter_map(|n| n.id).collect(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: i64, kind: NodeKind, name: &str, path: &str) -> Node {
        Node {
            id: Some(NodeId(id)),
            kind,
            name: name.into(),
            qualified_name: format!("{path}::{name}"),
            path: path.into(),
            line_start: 1,
            line_end: 1,
            content_hash: String::new(),
            language: "x".into(),
            provider_ref: None,
        }
    }
    fn edge(k: EdgeKind, s: i64, t: i64) -> Edge {
        Edge {
            kind: k,
            source: NodeId(s),
            target: NodeId(t),
            confidence: 1.0,
        }
    }
    #[test]
    fn linked_files_join_and_label_is_common_dir() {
        let n = vec![
            node(1, NodeKind::File, "a.ts", "src/auth/a.ts"),
            node(2, NodeKind::File, "b.ts", "src/auth/b.ts"),
            node(3, NodeKind::File, "z.ts", "z.ts"),
            node(4, NodeKind::Function, "loginUser", "src/auth/a.ts"),
        ];
        let c = compute(&n, &[edge(EdgeKind::Imports, 1, 2)]);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].label, "src/auth");
        assert_eq!(c[0].cohesion, Some(1.0));
        assert_eq!(c[0].members.len(), 3);
        assert_eq!(c[0].keywords, vec!["login", "user"]);
        assert_eq!(c[1].label, "z.ts");
        assert_eq!(c[1].cohesion, None);
    }
    #[test]
    fn cohesion_is_density_of_the_file_graph() {
        let n = vec![
            node(1, NodeKind::File, "a.ts", "a.ts"),
            node(2, NodeKind::File, "b.ts", "b.ts"),
            node(3, NodeKind::File, "c.ts", "c.ts"),
        ];
        let c = compute(
            &n,
            &[edge(EdgeKind::Imports, 1, 2), edge(EdgeKind::Imports, 2, 3)],
        );
        assert_eq!(c.len(), 1);
        assert!((c[0].cohesion.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }
    #[test]
    fn no_edges_means_one_singleton_per_file() {
        let n = vec![
            node(1, NodeKind::File, "a.ts", "a.ts"),
            node(2, NodeKind::File, "b.ts", "b.ts"),
            node(3, NodeKind::File, "c.ts", "c.ts"),
        ];
        let c = compute(&n, &[]);
        assert_eq!(c.len(), 3);
        let all: BTreeSet<_> = c.iter().flat_map(|c| c.members.clone()).collect();
        assert_eq!(all.len(), 3);
    }
    #[test]
    fn cross_file_calls_link_and_labels_are_unique() {
        let n = vec![
            node(1, NodeKind::File, "a.ts", "x/a.ts"),
            node(2, NodeKind::File, "b.ts", "y/b.ts"),
            node(3, NodeKind::Function, "f", "x/a.ts"),
            node(4, NodeKind::Function, "g", "y/b.ts"),
            node(5, NodeKind::File, "c.ts", "x/c.ts"),
            node(6, NodeKind::File, "d.ts", "x/d.ts"),
        ];
        let c = compute(
            &n,
            &[edge(EdgeKind::Calls, 3, 4), edge(EdgeKind::Imports, 5, 6)],
        );
        assert_eq!(c.len(), 2);
        // {x/a,y/b} has no common dir -> most frequent first segment, alphabetical tie -> "x"; {x/c,x/d} -> "x" again
        assert_eq!(c[0].label, "x");
        assert_eq!(c[1].label, "x (2)");
    }
    fn big_component() -> (Vec<Node>, Vec<Edge>) {
        let mut n = vec![];
        let mut e = vec![];
        let dirs = ["a", "a", "a", "a", "b", "b", "b", "c", "c", "c"];
        for (i, d) in dirs.iter().enumerate() {
            n.push(node(
                i as i64 + 1,
                NodeKind::File,
                "f.ts",
                &format!("src/{d}/f{i}.ts"),
            ));
            if i > 0 {
                e.push(edge(EdgeKind::Imports, i as i64, i as i64 + 1));
            }
        }
        (n, e)
    }
    #[test]
    fn a_giant_component_is_split_by_directory() {
        let (n, e) = big_component();
        let c = compute(&n, &e);
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["src/a", "src/b", "src/c"]);
        assert_eq!(c.iter().map(|c| c.members.len()).sum::<usize>(), 10);
    }
    #[test]
    fn a_component_under_the_bound_is_untouched() {
        let (mut n, mut e) = big_component();
        for i in 0..30 {
            n.push(node(
                100 + i,
                NodeKind::File,
                "z.ts",
                &format!("lib/z{i}.ts"),
            ));
        }
        // 10 of 40 files = 25%, under 30%: stays whole
        let c = compute(&n, &e);
        assert!(c.iter().any(|c| c.label == "src" && c.members.len() == 10));
        e.clear();
    }
    #[test]
    fn split_is_deterministic() {
        let (n, e) = big_component();
        assert_eq!(compute(&n, &e), compute(&n, &e));
    }
    #[test]
    fn tokeniser_splits_camel_and_snake() {
        assert_eq!(
            tokens("parseHTTPResponse_v2"),
            vec!["parse", "http", "response"]
        );
        assert!(tokens("get_a").is_empty());
    }
}
