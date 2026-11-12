//! `ui owners` / `ui screens-of`: join a registry screen to the files of our web app that draw it, and back.
//!
//! The join, all deterministic and read from files, never from a model:
//!   registry route (`products.<p>.routes[].pattern`, `screens[].ours.currentRoutes`)
//!   -> the route table: the JSX `<Route path=".." element={<Page />}>` literals of the root repo's web source (nested routes are joined,
//!      a path-less route with an element is a LAYOUT that wraps everything under it)
//!   -> the page file, through the imports of the file that holds the route
//!   -> its render/import closure inside the web source (depth-bounded, `.tsx`/`.jsx` components only, tests and `node_modules` never).
//!
//! The root repo is the one the store holds with role `root` (or the repo named by `--product`). Its graph supplies the file list (what exists),
//! symbol counts and the `imports` edges it resolved (`graphEdge`). The graph's `imports` edges miss the `~/` alias, so the import lines are
//! read from the clone's text; a closure edge the graph does not hold is counted in the `graph_imports_partial` limitation. Route tables in other
//! forms than JSX `<Route>` (object routers, file-system routes) are not read: a screen whose route is not found is `route_unmapped`, never guessed.
#![allow(clippy::result_large_err)]
use super::{AppError, Result};
use crate::domain::{Limitation, NodeKind};
use crate::store::Store;
use regex::Regex;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_SRC: &str = "apps/web/src";
pub const DEFAULT_DEPTH: usize = 3;
pub const MAX_DEPTH: usize = 6;
const MAX_OWNERS: usize = 100;
const EXTS: [&str; 4] = ["tsx", "ts", "jsx", "js"];

fn bad(m: impl Into<String>) -> AppError {
    AppError::InvalidArguments(m.into())
}

// ---------------------------------------------------------------- source reading

/// Blank out `/* */` comments (JSX `{/* */}` too), keeping every byte offset and newline so line numbers stay true.
fn strip_block_comments(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'/' && b[i + 1] == b'*' {
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == b'*' && b[j + 1] == b'/') {
                j += 1;
            }
            let end = (j + 2).min(b.len());
            for c in out.iter_mut().take(end).skip(i) {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| t.to_string())
}

#[derive(Clone, Debug)]
struct Imp {
    /// local identifiers the import binds
    names: Vec<String>,
    target: String,
}
struct Parsed {
    text: String,
    imports: Vec<Imp>,
}

fn is_test_file(p: &str) -> bool {
    p.contains(".test.") || p.contains(".spec.") || p.contains("/__tests__/")
}
fn is_component_file(p: &str) -> bool {
    (p.ends_with(".tsx") || p.ends_with(".jsx")) && !is_test_file(p)
}
fn dir_of(p: &str) -> &str {
    p.rfind('/').map_or("", |i| &p[..i])
}
fn normalise(p: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for s in p.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}
/// Does `<Name` appear as a tag in the text.
fn renders(text: &str, name: &str) -> bool {
    let pat = format!("<{name}");
    let mut from = 0;
    while let Some(i) = text[from..].find(&pat) {
        let end = from + i + pat.len();
        match text[end..].chars().next() {
            Some(c) if c.is_alphanumeric() || c == '_' => {}
            _ => return true,
        }
        from = end;
    }
    false
}

// ---------------------------------------------------------------- the route table

#[derive(Clone, Debug)]
struct RouteRow {
    /// absolute path of the route, `/rooms/:id`
    path: String,
    /// the element/Component identifier, if any
    element: Option<String>,
    /// `<Navigate to="..">` target
    redirect: Option<String>,
    /// outermost first
    layouts: Vec<String>,
    file: String,
    line: usize,
}

fn join_path(parent: &str, p: &str) -> String {
    let raw = if p.starts_with('/') { p.to_string() } else { format!("{}/{}", parent.trim_end_matches('/'), p) };
    let n = normalise(&raw);
    format!("/{n}")
}
fn segs(p: &str) -> Vec<&str> {
    p.split('/').filter(|s| !s.is_empty()).collect()
}
/// `exact`: the same segments (parameter names ignored). `pattern`: the table route has a parameter or a splat where the registry has a literal.
/// Returns the number of literal segments for ranking.
fn match_route(registry: &str, table: &str) -> Option<(&'static str, usize)> {
    let (r, t) = (segs(registry), segs(table));
    if t.len() == 1 && t[0] == "*" {
        return None; // the catch-all maps nothing
    }
    let mut exact = true;
    let mut lit = 0;
    for (i, ts) in t.iter().enumerate() {
        if *ts == "*" {
            return if i <= r.len() { Some(("pattern", lit)) } else { None };
        }
        let rs = r.get(i)?;
        if ts.starts_with(':') {
            if !rs.starts_with(':') {
                exact = false;
            }
        } else if ts == rs {
            lit += 1;
        } else {
            return None;
        }
    }
    if t.len() != r.len() {
        return None;
    }
    Some((if exact { "exact" } else { "pattern" }, lit))
}

// ---------------------------------------------------------------- the root repo

struct Web {
    clone: PathBuf,
    src: String,
    files: BTreeSet<String>,
    symbols: BTreeMap<String, usize>,
    graph_imports: BTreeSet<(String, String)>,
    repo: String,
    head: Option<String>,
    built: String,
    cache: RefCell<BTreeMap<String, std::rc::Rc<Parsed>>>,
}

fn clone_head(clone: &Path) -> Option<String> {
    let mut git = clone.join(".git");
    if git.is_file() {
        let t = std::fs::read_to_string(&git).ok()?;
        let g = t.trim().strip_prefix("gitdir:")?.trim().to_string();
        git = if Path::new(&g).is_absolute() { PathBuf::from(g) } else { clone.join(g) };
    }
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref:") else { return Some(head.to_string()) };
    let r = r.trim();
    if let Ok(t) = std::fs::read_to_string(git.join(r)) {
        return Some(t.trim().to_string());
    }
    let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|l| l.strip_suffix(r).map(|h| h.trim().to_string()))
}

impl Web {
    fn load(store_root: &Path, product: &str, src: &str) -> Result<(Web, Vec<Limitation>)> {
        let store = Store::open_existing(store_root)?;
        let repos = store.repos()?;
        let repo = repos
            .iter()
            .find(|r| r.name == product && r.role == "root")
            .or_else(|| repos.iter().find(|r| r.role == "root" && r.name == product))
            .or_else(|| repos.iter().find(|r| r.name == product))
            .or_else(|| repos.iter().find(|r| r.role == "root"))
            .ok_or_else(|| AppError::NotFound("the store holds no root repo (role root) to join the screens to".into()))?;
        let run = store
            .latest_done_run(repo.id.unwrap())?
            .ok_or_else(|| AppError::NotFound(format!("{} has no finished run; `comparegraph build {}` first", repo.name, repo.name)))?;
        let rid = run.id.unwrap();
        let prefix = format!("{}/", src.trim_end_matches('/'));
        let (mut files, mut symbols) = (BTreeSet::new(), BTreeMap::<String, usize>::new());
        for n in store.nodes_of_run(rid)? {
            if !n.path.starts_with(&prefix) {
                continue;
            }
            if n.kind == NodeKind::File {
                files.insert(n.path.clone());
            } else {
                *symbols.entry(n.path.clone()).or_default() += 1;
            }
        }
        if files.is_empty() {
            return Err(AppError::not_found_with(
                format!("the graph of {} holds no file under {prefix}", repo.name),
                "--src",
                vec![],
                "pass --src with the folder of the web app inside the root repo (default apps/web/src)",
            ));
        }
        let mut graph_imports = BTreeSet::new();
        {
            let mut st = store.conn().prepare(
                "SELECT s.path, t.path FROM edge e JOIN node s ON s.id = e.source_node_id JOIN node t ON t.id = e.target_node_id \
                 WHERE e.run_id = ?1 AND e.kind = 'imports' AND s.kind = 'file' AND t.kind = 'file' AND s.path LIKE ?2",
            )?;
            let rows = st.query_map(rusqlite::params![rid.0, format!("{prefix}%")], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                graph_imports.insert(r?);
            }
        }
        let clone = PathBuf::from(&repo.path);
        if !clone.join(&files.iter().next().unwrap()[..]).is_file() {
            return Err(AppError::not_found_with(
                format!("the clone of {} is not at {} (or lacks {}); the join reads route and import lines from its files", repo.name, clone.display(), files.iter().next().unwrap()),
                &repo.name,
                vec![],
                "re-point it with `comparegraph add <path> --replace --name <repo>` or pass `--root-map FROM=TO`",
            ));
        }
        let mut lims = vec![];
        let now = clone_head(&clone);
        let built = repo.head.clone();
        let moved = match (&now, &built) {
            (Some(n), Some(b)) => !n.starts_with(b.as_str()) && !b.starts_with(n.as_str()),
            _ => false,
        };
        if moved {
            lims.push(Limitation::new("root_head_moved", format!("the root graph of {} was built at {} ({}), the clone is now at {}: the file list is the graph's, the route and import lines are the clone's", repo.name, built.clone().unwrap_or_default(), run.started_at, now.clone().unwrap_or_default())));
        } else {
            lims.push(Limitation::new("root_built_at", format!("answers reflect the root graph of {} built at {} ({}); a newer commit is not in it until `comparegraph build {}` is run again", repo.name, built.clone().unwrap_or_else(|| "unknown commit".into()), run.started_at, repo.name)));
        }
        Ok((Web { clone, src: src.trim_end_matches('/').to_string(), files, symbols, graph_imports, repo: repo.name.clone(), head: built, built: run.started_at.clone(), cache: RefCell::new(BTreeMap::new()) }, lims))
    }

    fn resolve(&self, from: &str, spec: &str) -> Option<String> {
        let base = if let Some(r) = spec.strip_prefix("~/").or_else(|| spec.strip_prefix("@/")) {
            normalise(&format!("{}/{}", self.src, r))
        } else if spec.starts_with('.') {
            normalise(&format!("{}/{}", dir_of(from), spec))
        } else {
            return None;
        };
        let mut c = vec![base.clone()];
        c.extend(EXTS.iter().map(|e| format!("{base}.{e}")));
        c.extend(EXTS.iter().map(|e| format!("{base}/index.{e}")));
        c.into_iter().find(|p| self.files.contains(p))
    }

    fn parse(&self, file: &str) -> Option<std::rc::Rc<Parsed>> {
        if let Some(p) = self.cache.borrow().get(file) {
            return Some(p.clone());
        }
        let raw = std::fs::read_to_string(self.clone.join(file)).ok()?;
        let text = strip_block_comments(&raw);
        let mut imports: Vec<Imp> = vec![];
        let re = Regex::new(r#"(?s)\bimport\s+(?:type\s+)?([^;'"]*?)\s*from\s*["']([^"']+)["']"#).unwrap();
        for c in re.captures_iter(&text) {
            let (clause, spec) = (&c[1], &c[2]);
            let mut names: Vec<String> = vec![];
            let (head, braces) = match clause.find('{') {
                Some(i) => (&clause[..i], clause[i + 1..].split('}').next().unwrap_or("")),
                None => (clause, ""),
            };
            for part in head.split(',') {
                let p = part.trim();
                if let Some(ns) = p.strip_prefix("* as ") {
                    names.push(ns.trim().to_string());
                } else if !p.is_empty() {
                    names.push(p.to_string());
                }
            }
            for part in braces.split(',') {
                let p = part.trim().trim_start_matches("type ").trim();
                let local = p.rsplit(" as ").next().unwrap_or(p).trim();
                if !local.is_empty() {
                    names.push(local.to_string());
                }
            }
            if let Some(t) = self.resolve(file, spec) {
                imports.push(Imp { names, target: t });
            }
        }
        let lazy = Regex::new(r#"(?s)\b(\w+)\s*=\s*(?:React\.)?lazy\(\s*\(\s*\)\s*=>\s*import\(\s*["']([^"']+)["']"#).unwrap();
        for c in lazy.captures_iter(&text) {
            if let Some(t) = self.resolve(file, &c[2]) {
                imports.push(Imp { names: vec![c[1].to_string()], target: t });
            }
        }
        let p = std::rc::Rc::new(Parsed { text, imports });
        self.cache.borrow_mut().insert(file.to_string(), p.clone());
        Some(p)
    }

    /// Every JSX `<Route>` of the web source, nested paths joined, layouts recorded.
    fn route_table(&self) -> Vec<RouteRow> {
        let path_re = Regex::new(r#"\bpath\s*=\s*(?:\{\s*)?["'`]([^"'`]*)["'`]"#).unwrap();
        let el_re = Regex::new(r"\belement\s*=\s*\{\s*<\s*([A-Za-z_][\w.]*)").unwrap();
        let comp_re = Regex::new(r"\bComponent\s*=\s*\{\s*([A-Z][\w.]*)").unwrap();
        let to_re = Regex::new(r#"\bto\s*=\s*(?:\{\s*)?["'`]([^"'`]+)["'`]"#).unwrap();
        let idx_re = Regex::new(r"\sindex(\s|/|>|=)").unwrap();
        let mut out = vec![];
        for f in &self.files {
            if is_test_file(f) || !(f.ends_with(".tsx") || f.ends_with(".jsx")) {
                continue;
            }
            let Some(p) = self.parse(f) else { continue };
            if !p.text.contains("<Route") {
                continue;
            }
            let b = p.text.as_bytes();
            struct Frame {
                path: String,
                layout: Option<String>,
            }
            let mut stack: Vec<Frame> = vec![];
            let (mut i, mut line) = (0usize, 1usize);
            while i < b.len() {
                if b[i] == b'\n' {
                    line += 1;
                    i += 1;
                    continue;
                }
                if b[i] != b'<' {
                    i += 1;
                    continue;
                }
                if p.text[i..].starts_with("</Route>") {
                    stack.pop();
                    i += 8;
                    continue;
                }
                let boundary = matches!(b.get(i + 6), Some(c) if c.is_ascii_whitespace() || *c == b'>' || *c == b'/');
                if !(p.text[i..].starts_with("<Route") && boundary) {
                    i += 1;
                    continue;
                }
                // the tag ends at the first `>` outside braces and quotes
                let (mut j, mut depth, mut q) = (i + 6, 0i32, 0u8);
                while j < b.len() {
                    let c = b[j];
                    if q != 0 {
                        if c == q {
                            q = 0;
                        }
                    } else if c == b'"' || c == b'\'' || c == b'`' {
                        q = c;
                    } else if c == b'{' {
                        depth += 1;
                    } else if c == b'}' {
                        depth -= 1;
                    } else if c == b'>' && depth <= 0 && b[j - 1] != b'=' {
                        break;
                    }
                    j += 1;
                }
                let tag = &p.text[i..j.min(b.len())];
                let self_close = tag.trim_end().ends_with('/');
                let parent = stack.last().map(|f| f.path.clone()).unwrap_or_else(|| "/".into());
                let own = path_re.captures(tag).map(|c| c[1].to_string());
                let element = el_re.captures(tag).map(|c| c[1].to_string()).or_else(|| comp_re.captures(tag).map(|c| c[1].to_string()));
                let is_index = idx_re.is_match(tag);
                let full = match (&own, is_index) {
                    (Some(p), _) => join_path(&parent, p),
                    (None, _) => parent.clone(),
                };
                let layout_only = own.is_none() && !is_index && element.is_some();
                let redirect = if element.as_deref() == Some("Navigate") { to_re.captures(tag).map(|c| c[1].to_string()) } else { None };
                let layouts: Vec<String> = stack.iter().filter_map(|f| f.layout.clone()).collect();
                if !layout_only && (own.is_some() || is_index) && element.is_some() {
                    out.push(RouteRow { path: full.clone(), element: element.clone(), redirect, layouts, file: f.clone(), line });
                }
                if !self_close {
                    stack.push(Frame { path: full, layout: if layout_only { element } else { None } });
                }
                line += tag.matches('\n').count();
                i = j + 1;
            }
        }
        out
    }

    /// The file a component identifier names in `file`: its import, else `file` itself when it declares it.
    fn component_file(&self, file: &str, name: &str) -> Option<(String, &'static str)> {
        let head = name.split('.').next().unwrap_or(name);
        let p = self.parse(file)?;
        if let Some(imp) = p.imports.iter().find(|i| i.names.iter().any(|n| n == head)) {
            return Some((imp.target.clone(), "import"));
        }
        let decl = Regex::new(&format!(r"\b(?:function|const|class)\s+{}\b", regex::escape(head))).unwrap();
        decl.is_match(&p.text).then(|| (file.to_string(), "declared"))
    }

    /// The render/import closure of a component file: (file, depth, edge kind, via), breadth first, depth-bounded.
    fn closure(&self, start: &str, depth: usize) -> Vec<(String, usize, &'static str, String)> {
        let mut seen: BTreeSet<String> = BTreeSet::from([start.to_string()]);
        let mut level = vec![start.to_string()];
        let mut out = vec![];
        for d in 1..=depth {
            let mut next: BTreeMap<String, (&'static str, String)> = BTreeMap::new();
            for f in &level {
                let Some(p) = self.parse(f) else { continue };
                for imp in &p.imports {
                    if !is_component_file(&imp.target) || seen.contains(&imp.target) {
                        continue;
                    }
                    let kind = if imp.names.iter().any(|n| renders(&p.text, n)) { "renders" } else { "imports" };
                    let e = next.entry(imp.target.clone()).or_insert((kind, f.clone()));
                    if kind == "renders" && e.0 == "imports" {
                        *e = (kind, f.clone());
                    }
                }
            }
            level = vec![];
            for (t, (k, via)) in next {
                seen.insert(t.clone());
                level.push(t.clone());
                out.push((t, d, k, via));
            }
            if level.is_empty() {
                break;
            }
        }
        out
    }
}

// ---------------------------------------------------------------- the registry

struct Registry {
    screens: Vec<String>,
    doc: Value,
}

fn load_registry(ui_root: &Path) -> Result<Registry> {
    let p = [ui_root.join("ui").join("registry.json"), ui_root.join("registry.json")].into_iter().find(|p| p.is_file());
    let Some(p) = p else {
        return Err(AppError::NotFound(format!("no registry.json in {} or {}/ui (the ui registry with the screens and their routes)", ui_root.display(), ui_root.display())));
    };
    let t = std::fs::read_to_string(&p).map_err(|e| AppError::Io(p.clone(), e))?;
    let doc: Value = serde_json::from_str(&t).map_err(|e| bad(format!("{} is not JSON: {e}", p.display())))?;
    let mut screens: Vec<String> = doc["screens"].as_array().map(|a| a.iter().filter_map(|s| s["id"].as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(ps) = doc["products"].as_object() {
        for p in ps.values() {
            for r in p["routes"].as_array().into_iter().flatten() {
                if let Some(s) = r["screen"].as_str() {
                    if !screens.iter().any(|x| x == s) {
                        screens.push(s.to_string());
                    }
                }
            }
        }
    }
    Ok(Registry { screens, doc })
}

impl Registry {
    /// The routes the registry names for a screen: `products.<p>.routes[]` and `screens[].ours.currentRoutes`.
    fn routes_for(&self, screen: &str, product: &str) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        for r in self.doc["products"][product]["routes"].as_array().into_iter().flatten() {
            if r["screen"].as_str() == Some(screen) {
                if let Some(p) = r["pattern"].as_str() {
                    v.push(p.to_string());
                }
            }
        }
        for s in self.doc["screens"].as_array().into_iter().flatten() {
            let prod_ok = matches!(s["ours"]["product"].as_str(), None | Some("")) || s["ours"]["product"].as_str() == Some(product);
            if s["id"].as_str() == Some(screen) && prod_ok {
                for r in s["ours"]["currentRoutes"].as_array().into_iter().flatten() {
                    if let Some(p) = r.as_str() {
                        v.push(p.to_string());
                    }
                }
            }
        }
        // a registry route may carry prose after the path (`/rooms/:id (Invite)`): the path is the first word, and only a word starting with `/` is a path
        let mut v: Vec<String> = v.into_iter().filter_map(|x| x.split_whitespace().next().filter(|w| w.starts_with('/')).map(String::from)).collect();
        let mut seen = BTreeSet::new();
        v.retain(|x| seen.insert(x.clone()));
        v
    }
}

// ---------------------------------------------------------------- mapping a route

struct Mapped {
    status: &'static str,
    reason: Option<String>,
    /// (table route, page file, how the file was found, confidence, redirect hop)
    matches: Vec<Match>,
}
#[derive(Clone)]
struct Match {
    table: String,
    declared_in: String,
    line: usize,
    component: String,
    page: Option<String>,
    confidence: &'static str,
    layouts: Vec<(String, Option<String>)>,
    redirect_from: Option<String>,
}

fn map_route(web: &Web, table: &[RouteRow], all: &[RouteRow], route: &str, hop: bool) -> Mapped {
    let mut cands: Vec<(&RouteRow, &'static str, usize)> = table.iter().filter_map(|t| match_route(route, &t.path).map(|(c, l)| (t, c, l))).collect();
    cands.sort_by(|a, b| (a.1 != "exact", std::cmp::Reverse(a.2), &a.0.path).cmp(&(b.1 != "exact", std::cmp::Reverse(b.2), &b.0.path)));
    let Some(best) = cands.first().map(|c| (c.1, c.2)) else {
        let catch = table.iter().find(|t| t.path == "/*");
        let mut reason = match catch {
            Some(c) => format!("no route literal matches {route}; only the catch-all {} ({}) would", c.path, c.element.clone().unwrap_or_default()),
            None => format!("no route literal matches {route}"),
        };
        let mut elsewhere: Vec<&str> = all.iter().filter(|t| !table.iter().any(|x| std::ptr::eq(x, *t)) && match_route(route, &t.path).is_some()).map(|t| t.file.as_str()).collect();
        elsewhere.sort();
        elsewhere.dedup();
        if !elsewhere.is_empty() {
            reason.push_str(&format!(" in the selected router; other router file(s) hold it: {} (pass --router)", elsewhere.join(", ")));
        }
        return Mapped { status: "route_unmapped", reason: Some(reason), matches: vec![] };
    };
    let mut matches = vec![];
    for (t, conf, lit) in cands.iter().filter(|c| (c.1, c.2) == best) {
        let _ = lit;
        if t.element.as_deref() == Some("Navigate") {
            if let (Some(to), false) = (&t.redirect, hop) {
                let inner = map_route(web, table, all, to, true);
                for mut m in inner.matches {
                    m.redirect_from = Some(t.path.clone());
                    if *conf != "exact" {
                        m.confidence = "pattern";
                    }
                    matches.push(m);
                }
            }
            continue;
        }
        let el = t.element.clone().unwrap_or_default();
        let page = web.component_file(&t.file, &el).map(|x| x.0);
        let layouts = t.layouts.iter().map(|l| (l.clone(), web.component_file(&t.file, l).map(|x| x.0))).collect();
        matches.push(Match { table: t.path.clone(), declared_in: t.file.clone(), line: t.line, component: el, page, confidence: conf, layouts, redirect_from: None });
    }
    if matches.is_empty() {
        return Mapped { status: "route_unmapped", reason: Some(format!("{route} only reaches a redirect with no page")), matches };
    }
    Mapped { status: "mapped", reason: None, matches }
}

type Prepared = (Web, Vec<Limitation>, Registry, String, usize, Vec<RouteRow>, Vec<RouteRow>);
fn prepare(store_root: &Path, ui_root: &Path, product: Option<&str>, depth: Option<usize>, src: Option<&str>, router: Option<&str>) -> Result<Prepared> {
    let product = product.unwrap_or("massrelay").to_string();
    let depth = depth.unwrap_or(DEFAULT_DEPTH);
    if !(1..=MAX_DEPTH).contains(&depth) {
        return Err(bad(format!("--depth must be 1..={MAX_DEPTH}, got {depth}")));
    }
    let reg = load_registry(ui_root)?;
    let (web, lims) = Web::load(store_root, &product, src.unwrap_or(DEFAULT_SRC))?;
    let all = web.route_table();
    let mut lims = lims;
    let app_root = format!("{}/App.tsx", web.src);
    let wanted = router.map(normalise).or_else(|| all.iter().any(|t| t.file == app_root).then(|| app_root.clone()));
    let table: Vec<RouteRow> = match &wanted {
        Some(w) => all.iter().filter(|t| &t.file == w).cloned().collect(),
        None => all.clone(),
    };
    let routers: BTreeSet<&str> = all.iter().map(|t| t.file.as_str()).collect();
    if routers.len() > 1 {
        lims.push(Limitation::new("router_selected", format!("{} router files hold <Route> tables ({}); the table read is {} (default: the app root App.tsx; --router FILE picks another)", routers.len(), routers.iter().copied().collect::<Vec<_>>().join(", "), wanted.clone().unwrap_or_else(|| "all of them".into()))));
    }
    if table.is_empty() {
        return Err(AppError::not_found_with(
            format!("no JSX <Route path=\"..\" element={{<X/>}}> literal found in the files of {} under {}", web.repo, web.src),
            "route table",
            vec![],
            "only JSX <Route> tables are read; pass --src for the folder that holds the router",
        ));
    }
    Ok((web, lims, reg, product, depth, table, all))
}

fn root_info(web: &Web) -> Value {
    json!({ "repo": web.repo, "builtAtCommit": web.head, "builtAt": web.built, "src": web.src, "files": web.files.len() })
}

fn conf_of(c: &[&str]) -> &'static str {
    if c.is_empty() {
        "unknown"
    } else if c.iter().all(|x| *x == "exact") {
        "derived"
    } else {
        "inferred"
    }
}

fn partial_limit(missing: usize, total: usize) -> Option<Limitation> {
    (missing > 0).then(|| Limitation::new("graph_imports_partial", format!("{missing} of {total} import/render edges read from the source are not in the stored graph (its `imports` edges miss the `~/` alias); the closure is from the source text, `graphEdge` says which edges the graph also holds")))
}

// ---------------------------------------------------------------- ui owners

#[derive(Clone)]
struct Owner {
    file: String,
    scope: &'static str,
    depth: usize,
    how: &'static str,
    via: String,
    confidence: &'static str,
    route: String,
}

pub fn owners(store_root: &Path, ui_root: &Path, screen: &str, product: Option<&str>, depth: Option<usize>, src: Option<&str>, router: Option<&str>) -> Result<Value> {
    let (web, mut lims, reg, product, depth, table, all) = prepare(store_root, ui_root, product, depth, src, router)?;
    if !reg.screens.iter().any(|s| s == screen) {
        return Err(AppError::not_found_with(format!("the registry has no screen `{screen}`"), screen, reg.screens.clone(), "pick a screen id from the registry (`comparegraph ui screens <product>` lists the captured ones)"));
    }
    let routes = reg.routes_for(screen, &product);
    let mut blocks = vec![];
    let mut owners: BTreeMap<String, Owner> = BTreeMap::new();
    let mut confs: Vec<&str> = vec![];
    let mut unmapped: Vec<String> = vec![];
    if routes.is_empty() {
        lims.push(Limitation::new("route_unmapped", format!("the registry names no route for screen {screen} (product {product})")));
    }
    let mut edges_total = 0usize;
    let mut edges_missing = 0usize;
    for r in &routes {
        let m = map_route(&web, &table, &all, r, false);
        if m.status == "route_unmapped" {
            unmapped.push(format!("{r}: {}", m.reason.clone().unwrap_or_default()));
        }
        let mut ms = vec![];
        for x in &m.matches {
            confs.push(x.confidence);
            let mut owned = vec![];
            if let Some(page) = &x.page {
                let mut add = |file: &str, scope: &'static str, d: usize, how: &'static str, via: String| {
                    let o = Owner { file: file.into(), scope, depth: d, how, via, confidence: x.confidence, route: x.table.clone() };
                    let key = file.to_string();
                    let better = |a: &Owner, b: &Owner| (a.scope, a.depth, a.how != "renders").cmp(&(b.scope, b.depth, b.how != "renders")) == std::cmp::Ordering::Less;
                    match owners.get(&key) {
                        Some(old) if !better(&o, old) => {}
                        _ => {
                            owners.insert(key, o);
                        }
                    }
                };
                add(page, "page", 0, "route-element", format!("{} in {}:{}", x.table, x.declared_in, x.line));
                for (f, d, k, via) in web.closure(page, depth) {
                    edges_total += 1;
                    if !web.graph_imports.contains(&(via.clone(), f.clone())) {
                        edges_missing += 1;
                    }
                    owned.push(f.clone());
                    add(&f, "page", d, k, via);
                }
                for (name, lf) in &x.layouts {
                    if let Some(lf) = lf {
                        add(lf, "layout", 0, "layout-element", format!("<Route element={{<{name} />}}> wraps {}", x.table));
                        for (f, d, k, via) in web.closure(lf, depth) {
                            edges_total += 1;
                            if !web.graph_imports.contains(&(via.clone(), f.clone())) {
                                edges_missing += 1;
                            }
                            add(&f, "layout", d, k, via);
                        }
                    }
                }
            } else {
                lims.push(Limitation::new("page_file_unresolved", format!("route {} names <{}> but no import of it resolves inside {}", x.table, x.component, web.src)));
            }
            ms.push(json!({
                "tableRoute": x.table, "confidence": x.confidence, "component": x.component, "pageFile": x.page,
                "declaredIn": format!("{}:{}", x.declared_in, x.line), "redirectFrom": x.redirect_from,
                "layouts": x.layouts.iter().map(|(n, f)| json!({"component": n, "file": f})).collect::<Vec<_>>(),
            }));
        }
        blocks.push(json!({ "registryRoute": r, "status": m.status, "reason": m.reason, "matches": ms }));
    }
    if !unmapped.is_empty() {
        lims.push(Limitation::new("route_unmapped", format!("no route literal found for: {}", unmapped.join("; "))));
    }
    if confs.contains(&"pattern") {
        lims.push(Limitation::new("route_pattern_match", "at least one registry route matched a table route only through a parameter or splat, not a literal"));
    }
    lims.extend(partial_limit(edges_missing, edges_total));
    let mut list: Vec<Owner> = owners.into_values().collect();
    let order = |o: &Owner| (if o.scope == "page" { 0 } else { 1 }, o.depth, if o.how == "renders" || o.how == "route-element" || o.how == "layout-element" { 0 } else { 1 }, o.file.clone());
    list.sort_by_key(order);
    let total = list.len();
    if total > MAX_OWNERS {
        lims.push(Limitation::new("owners_capped", format!("{total} owning files, the first {MAX_OWNERS} are listed")));
    }
    let items: Vec<Value> = list
        .iter()
        .take(MAX_OWNERS)
        .enumerate()
        .map(|(i, o)| json!({ "rank": i + 1, "file": o.file, "scope": o.scope, "depth": o.depth, "how": o.how, "via": o.via, "confidence": o.confidence, "route": o.route, "symbols": web.symbols.get(&o.file).copied().unwrap_or(0) }))
        .collect();
    let page_files: Vec<&str> = list.iter().filter(|o| o.how == "route-element").map(|o| o.file.as_str()).collect();
    let ec = conf_of(&confs);
    Ok(json!({
        "screen": screen, "product": product, "depth": depth, "root": root_info(&web),
        "routes": blocks, "pageFiles": page_files, "owners": items, "total": total,
        "evidence_confidence": ec,
        "limitation_items": lims,
    }))
}

// ---------------------------------------------------------------- ui screens-of

pub fn screens_of(store_root: &Path, ui_root: &Path, file: &str, product: Option<&str>, depth: Option<usize>, src: Option<&str>, router: Option<&str>) -> Result<Value> {
    let (web, mut lims, reg, product, depth, table, all) = prepare(store_root, ui_root, product, depth, src, router)?;
    let mut f = file.replace('\\', "/");
    let clone = web.clone.display().to_string().replace('\\', "/");
    if let Some(r) = f.strip_prefix(&format!("{clone}/")) {
        f = r.to_string();
    }
    let f = normalise(&f);
    if !web.files.contains(&f) {
        let cands: Vec<String> = web.files.iter().filter(|p| p.ends_with(&format!("/{}", f.rsplit('/').next().unwrap_or(&f)))).take(10).cloned().collect();
        return Err(AppError::not_found_with(format!("`{f}` is not a file of the root graph under {}", web.src), &f, cands, "pass the repo-relative path (apps/web/src/...)"));
    }
    let mut hits: Vec<Value> = vec![];
    let mut unmapped: Vec<String> = vec![];
    let mut confs: Vec<&str> = vec![];
    let (mut edges_total, mut edges_missing) = (0usize, 0usize);
    let rank = |role: &str| match role {
        "page" => 0,
        "child" => 1,
        _ => 2,
    };
    for screen in &reg.screens {
        let routes = reg.routes_for(screen, &product);
        if routes.is_empty() {
            unmapped.push(format!("{screen} (the registry names no route)"));
            continue;
        }
        let mut best: Option<(usize, usize, Value)> = None;
        let mut any_mapped = false;
        for r in &routes {
            let m = map_route(&web, &table, &all, r, false);
            if m.status == "route_unmapped" {
                continue;
            }
            any_mapped = true;
            for x in &m.matches {
                let Some(page) = &x.page else { continue };
                let mut found: Vec<(&'static str, usize, &'static str, String)> = vec![];
                if *page == f {
                    found.push(("page", 0, "route-element", format!("{} in {}:{}", x.table, x.declared_in, x.line)));
                }
                for (cf, d, k, via) in web.closure(page, depth) {
                    if cf == f {
                        edges_total += 1;
                        if !web.graph_imports.contains(&(via.clone(), cf.clone())) {
                            edges_missing += 1;
                        }
                        found.push(("child", d, k, via));
                    }
                }
                for (name, lf) in &x.layouts {
                    let Some(lf) = lf else { continue };
                    if *lf == f {
                        found.push(("layout", 0, "layout-element", format!("<Route element={{<{name} />}}> wraps {}", x.table)));
                    }
                    for (cf, d, k, via) in web.closure(lf, depth) {
                        if cf == f {
                            edges_total += 1;
                            if !web.graph_imports.contains(&(via.clone(), cf.clone())) {
                                edges_missing += 1;
                            }
                            found.push(("layout-child", d, k, via));
                        }
                    }
                }
                for (role, d, how, via) in found {
                    confs.push(x.confidence);
                    let k = (rank(role), d);
                    if best.as_ref().is_none_or(|b| k < (b.0, b.1)) {
                        best = Some((k.0, k.1, json!({ "screen": screen, "route": r, "tableRoute": x.table, "role": role, "depth": d, "how": how, "via": via, "confidence": x.confidence, "pageFile": page })));
                    }
                }
            }
        }
        if let Some((_, _, v)) = best {
            hits.push(v);
        } else if !any_mapped {
            unmapped.push(format!("{screen} (no route literal for {})", routes.join(", ")));
        }
    }
    let unmapped_total = unmapped.len();
    unmapped.truncate(20);
    hits.sort_by(|a, b| (rank(a["role"].as_str().unwrap_or("")), a["depth"].as_u64(), a["screen"].as_str()).cmp(&(rank(b["role"].as_str().unwrap_or("")), b["depth"].as_u64(), b["screen"].as_str())));
    if !unmapped.is_empty() {
        lims.push(Limitation::new("route_unmapped", format!("the blast radius over {unmapped_total} screen(s) is unknown (no route found); the first {}: {}", unmapped.len(), unmapped.join("; "))));
    }
    if confs.contains(&"pattern") {
        lims.push(Limitation::new("route_pattern_match", "at least one screen reaches this file through a route matched by a parameter or splat, not a literal"));
    }
    lims.extend(partial_limit(edges_missing, edges_total));
    let ec = if hits.is_empty() && !unmapped.is_empty() { "unknown" } else { conf_of(&confs) };
    Ok(json!({
        "file": f, "product": product, "depth": depth, "root": root_info(&web),
        "symbols": web.symbols.get(&f).copied().unwrap_or(0),
        "screens": hits, "summary": { "registryScreens": reg.screens.len(), "rendering": hits.len(), "unmapped": unmapped_total },
        "unmapped": unmapped,
        "evidence_confidence": ec,
        "limitation_items": lims,
    }))
}
