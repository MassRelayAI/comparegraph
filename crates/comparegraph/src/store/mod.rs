use crate::domain::{
    content_hash, Edge, EdgeKind, Limitation, Node, NodeId, NodeKind, RepoId, Run, RunId, RunStatus,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const SCHEMA: &str = include_str!("schema.sql");
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cannot use {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot open {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("schema: {0}")]
    Schema(String),
    #[error("{0}")]
    NotFound(String),
}
pub type Result<T> = std::result::Result<T, StoreError>;
#[derive(Debug, Clone, PartialEq)]
pub struct CommunityRow {
    /// 0 until stored
    pub id: i64,
    pub label: String,
    pub keywords: Vec<String>,
    pub cohesion: Option<f64>,
    pub members: Vec<NodeId>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RepoRow {
    pub id: Option<RepoId>,
    pub name: String,
    pub path: String,
    pub remote: Option<String>,
    pub head: Option<String>,
    pub licence: Option<String>,
    pub lane: String,
    pub role: String,
    pub category: Option<String>,
    pub stars: Option<i64>,
    pub added_at: String,
}
impl RepoRow {
    pub fn new(name: &str, path: &str) -> Self {
        Self {
            id: None,
            name: name.into(),
            path: path.into(),
            remote: None,
            head: None,
            licence: None,
            lane: "study".into(),
            role: "comparator".into(),
            category: None,
            stars: None,
            added_at: now_iso(),
        }
    }
}
pub fn now_iso() -> String {
    // seconds since epoch rendered as an RFC 3339 UTC string without a chrono dependency
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant), valid for the years we care about
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

#[derive(Debug)]
pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    /// A cache read failure is a miss, never an error: the cache is an optimisation.
    pub fn parse_cache_get(&self, hash: &str, language: &str, version: &str) -> Option<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT payload FROM parse_cache WHERE file_hash = ?1 AND language = ?2 AND provider_version = ?3",
                params![hash, language, version],
                |r| r.get(0),
            )
            .ok()
    }
    pub fn parse_cache_put_many(&self, entries: &[(String, String, String, Vec<u8>)]) {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        for (h, l, v, b) in entries {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO parse_cache(file_hash, language, provider_version, payload) VALUES (?1, ?2, ?3, ?4)",
                params![h, l, v, b],
            );
        }
        let _ = tx.commit();
    }
    /// Open a store that must already exist; never creates `.comparegraph`. Read verbs use this.
    pub fn open_existing(root: &Path) -> Result<Store> {
        if !root.join(".comparegraph").join("comparegraph.db").is_file() {
            return Err(StoreError::NotFound(format!(
                "no workspace at {} — run `comparegraph add` first",
                root.join(".comparegraph").display()
            )));
        }
        Store::open(root)
    }
    pub fn open(root: &Path) -> Result<Store> {
        let dir = root.join(".comparegraph");
        std::fs::create_dir_all(&dir).map_err(|e| StoreError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let path = dir.join("comparegraph.db");
        let open_err = |source| StoreError::Open {
            path: path.clone(),
            source,
        };
        let conn = Connection::open(&path).map_err(open_err)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(open_err)?;
        conn.execute_batch(SCHEMA).map_err(open_err)?;
        // additive column (Plan 8 lane C): a store created before it gains it here; SCHEMA_VERSION stays 1
        let has_synonyms = {
            let mut st = conn.prepare("PRAGMA table_info(feature)").map_err(open_err)?;
            let names = st
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(open_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(open_err)?;
            names.iter().any(|n| n == "synonyms")
        };
        if !has_synonyms {
            conn.execute_batch("ALTER TABLE feature ADD COLUMN synonyms TEXT NOT NULL DEFAULT ''")
                .map_err(open_err)?;
        }
        let existing: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match existing.as_deref().map(|v| v.parse::<i64>()) {
            None => {
                conn.execute(
                    "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
            }
            Some(Ok(v)) if v == SCHEMA_VERSION => {}
            Some(other) => {
                return Err(StoreError::Schema(format!(
                    "{} has schema_version {:?}, this binary needs {}",
                    path.display(),
                    other,
                    SCHEMA_VERSION
                )))
            }
        }
        Ok(Store { conn, path })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )?
            .parse()
            .unwrap_or(0))
    }

    // ---- repo ----
    pub fn insert_repo(&self, r: &RepoRow) -> Result<RepoId> {
        self.conn.execute("INSERT INTO repo(name, path, remote, head, licence, lane, role, category, stars, added_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![r.name, r.path, r.remote, r.head, r.licence, r.lane, r.role, r.category, r.stars, r.added_at])?;
        Ok(RepoId(self.conn.last_insert_rowid()))
    }
    pub fn repo_by_name(&self, name: &str) -> Result<Option<RepoRow>> {
        Ok(self.conn.query_row("SELECT id,name,path,remote,head,licence,lane,role,category,stars,added_at FROM repo WHERE name=?1", params![name], row_to_repo).optional()?)
    }
    pub fn repos(&self) -> Result<Vec<RepoRow>> {
        let mut st = self.conn.prepare("SELECT id,name,path,remote,head,licence,lane,role,category,stars,added_at FROM repo ORDER BY name")?;
        let rows = st
            .query_map([], row_to_repo)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- run ----
    pub fn insert_run(&self, run: &Run) -> Result<RunId> {
        self.conn.execute("INSERT INTO run(repo_id, provider, provider_version, started_at, seconds, status, limitations_json, store_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![run.repo.0, run.provider, run.provider_version, run.started_at, run.seconds, run.status.as_str(), serde_json::to_string(&run.limitations).unwrap(), run.store_hash])?;
        Ok(RunId(self.conn.last_insert_rowid()))
    }
    pub fn finish_run(
        &self,
        id: RunId,
        status: RunStatus,
        seconds: f64,
        limitations: &[Limitation],
        store_hash: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE run SET status=?2, seconds=?3, limitations_json=?4, store_hash=?5 WHERE id=?1",
            params![
                id.0,
                status.as_str(),
                seconds,
                serde_json::to_string(limitations).unwrap(),
                store_hash
            ],
        )?;
        Ok(())
    }
    /// Replace a finished run's limitations (the multi-provider build adds its agreement rows this way); nothing else changes.
    pub fn set_run_limitations(&self, id: RunId, limitations: &[Limitation]) -> Result<()> {
        self.conn.execute(
            "UPDATE run SET limitations_json=?2 WHERE id=?1",
            params![id.0, serde_json::to_string(limitations).unwrap()],
        )?;
        Ok(())
    }
    pub fn run(&self, id: RunId) -> Result<Option<Run>> {
        Ok(self.conn.query_row("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE id=?1", params![id.0], row_to_run).optional()?)
    }
    pub fn latest_run(&self, repo: RepoId) -> Result<Option<Run>> {
        Ok(self.conn.query_row("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE repo_id=?1 ORDER BY id DESC LIMIT 1", params![repo.0], row_to_run).optional()?)
    }
    pub fn latest_done_run(&self, repo: RepoId) -> Result<Option<Run>> {
        Ok(self.conn.query_row("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE repo_id=?1 AND status='done' ORDER BY id DESC LIMIT 1", params![repo.0], row_to_run).optional()?)
    }
    /// The latest done run of one provider at any version.
    pub fn latest_done_run_by_provider(&self, repo: RepoId, provider: &str) -> Result<Option<Run>> {
        Ok(self.conn.query_row("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE repo_id=?1 AND status='done' AND provider=?2 ORDER BY id DESC LIMIT 1", params![repo.0, provider], row_to_run).optional()?)
    }
    /// The latest done run of one provider at one version (a run of another provider in between does not hide it).
    pub fn latest_done_run_of(&self, repo: RepoId, provider: &str, version: &str) -> Result<Option<Run>> {
        Ok(self.conn.query_row("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE repo_id=?1 AND status='done' AND provider=?2 AND provider_version=?3 ORDER BY id DESC LIMIT 1", params![repo.0, provider, version], row_to_run).optional()?)
    }

    // ---- nodes / edges ----
    pub fn insert_nodes(&self, repo: RepoId, run: RunId, nodes: &mut [Node]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare("INSERT INTO node(repo_id, run_id, kind, name, qualified_name, path, line_start, line_end, content_hash, language, provider_ref) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")?;
            let mut fts = tx.prepare(
                "INSERT INTO node_fts(rowid, name, qualified_name, path) VALUES (?1,?2,?3,?4)",
            )?;
            for n in nodes.iter_mut() {
                st.execute(params![
                    repo.0,
                    run.0,
                    n.kind.as_str(),
                    n.name,
                    n.qualified_name,
                    n.path,
                    n.line_start,
                    n.line_end,
                    n.content_hash,
                    n.language,
                    n.provider_ref
                ])?;
                let id = tx.last_insert_rowid();
                n.id = Some(NodeId(id));
                fts.execute(params![id, n.name, n.qualified_name, n.path])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn insert_edges(&self, repo: RepoId, run: RunId, edges: &[Edge]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare("INSERT INTO edge(repo_id, run_id, kind, source_node_id, target_node_id, confidence) VALUES (?1,?2,?3,?4,?5,?6)")?;
            for e in edges {
                st.execute(params![
                    repo.0,
                    run.0,
                    e.kind.as_str(),
                    e.source.0,
                    e.target.0,
                    e.confidence
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn nodes_of_run(&self, run: RunId) -> Result<Vec<Node>> {
        let mut st = self.conn.prepare("SELECT id,kind,name,qualified_name,path,line_start,line_end,content_hash,language,provider_ref FROM node WHERE run_id=?1 ORDER BY path, line_start, kind, qualified_name")?;
        let rows = st
            .query_map(params![run.0], |r| {
                Ok(Node {
                    id: Some(NodeId(r.get(0)?)),
                    kind: NodeKind::parse(&r.get::<_, String>(1)?).unwrap_or(NodeKind::Function),
                    name: r.get(2)?,
                    qualified_name: r.get(3)?,
                    path: r.get(4)?,
                    line_start: r.get(5)?,
                    line_end: r.get(6)?,
                    content_hash: r.get(7)?,
                    language: r.get(8)?,
                    provider_ref: r.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn edges_of_run(&self, run: RunId) -> Result<Vec<Edge>> {
        let mut st = self.conn.prepare("SELECT kind, source_node_id, target_node_id, confidence FROM edge WHERE run_id=?1 ORDER BY kind, source_node_id, target_node_id")?;
        let rows = st
            .query_map(params![run.0], |r| {
                Ok(Edge {
                    kind: EdgeKind::parse(&r.get::<_, String>(0)?).unwrap_or(EdgeKind::Calls),
                    source: NodeId(r.get(1)?),
                    target: NodeId(r.get(2)?),
                    confidence: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    // ---- communities ----
    pub fn insert_communities(
        &self,
        repo: RepoId,
        run: RunId,
        rows: &[CommunityRow],
    ) -> Result<Vec<i64>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = vec![];
        {
            let mut c = tx.prepare("INSERT INTO community(repo_id, run_id, label, keywords_json, cohesion, size) VALUES (?1,?2,?3,?4,?5,?6)")?;
            let mut m =
                tx.prepare("INSERT INTO community_member(community_id, node_id) VALUES (?1,?2)")?;
            for r in rows {
                c.execute(params![
                    repo.0,
                    run.0,
                    r.label,
                    serde_json::to_string(&r.keywords).unwrap_or_else(|_| "[]".into()),
                    r.cohesion,
                    r.members.len() as i64
                ])?;
                let id = tx.last_insert_rowid();
                for n in &r.members {
                    m.execute(params![id, n.0])?;
                }
                ids.push(id);
            }
        }
        tx.commit()?;
        Ok(ids)
    }
    pub fn communities_of_run(&self, run: RunId) -> Result<Vec<CommunityRow>> {
        let mut st = self.conn.prepare("SELECT id, label, keywords_json, cohesion FROM community WHERE run_id=?1 ORDER BY label, id")?;
        let mut rows = st
            .query_map(params![run.0], |r| {
                Ok(CommunityRow {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    keywords: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    cohesion: r.get(3)?,
                    members: vec![],
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut ms = self.conn.prepare(
            "SELECT node_id FROM community_member WHERE community_id=?1 ORDER BY node_id",
        )?;
        for r in rows.iter_mut() {
            r.members = ms
                .query_map(params![r.id], |x| Ok(NodeId(x.get(0)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
        }
        Ok(rows)
    }
    /// Deterministic digest of a run's graph: nodes by (path, line_start, kind, qualified_name, content_hash), edges by (kind, source qn, target qn).
    pub fn store_hash(&self, run: RunId) -> Result<String> {
        let nodes = self.nodes_of_run(run)?;
        let qn: std::collections::HashMap<NodeId, &str> = nodes
            .iter()
            .map(|n| (n.id.unwrap(), n.qualified_name.as_str()))
            .collect();
        let mut lines: Vec<String> = nodes
            .iter()
            .map(|n| {
                format!(
                    "N\t{}\t{}\t{}\t{}\t{}\t{}",
                    n.path,
                    n.line_start,
                    n.line_end,
                    n.kind.as_str(),
                    n.qualified_name,
                    n.content_hash
                )
            })
            .collect();
        lines.sort();
        let mut e: Vec<String> = self
            .edges_of_run(run)?
            .iter()
            .map(|e| {
                format!(
                    "E\t{}\t{}\t{}\t{:.3}",
                    e.kind.as_str(),
                    qn.get(&e.source).unwrap_or(&"?"),
                    qn.get(&e.target).unwrap_or(&"?"),
                    e.confidence
                )
            })
            .collect();
        e.sort();
        lines.extend(e);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        Ok(content_hash(&refs))
    }
}

fn row_to_repo(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRow> {
    Ok(RepoRow {
        id: Some(RepoId(r.get(0)?)),
        name: r.get(1)?,
        path: r.get(2)?,
        remote: r.get(3)?,
        head: r.get(4)?,
        licence: r.get(5)?,
        lane: r.get(6)?,
        role: r.get(7)?,
        category: r.get(8)?,
        stars: r.get(9)?,
        added_at: r.get(10)?,
    })
}
fn row_to_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let lim: String = r.get(7)?;
    Ok(Run {
        id: Some(RunId(r.get(0)?)),
        repo: RepoId(r.get(1)?),
        provider: r.get(2)?,
        provider_version: r.get(3)?,
        started_at: r.get(4)?,
        seconds: r.get(5)?,
        status: RunStatus::parse(&r.get::<_, String>(6)?).unwrap_or(RunStatus::Failed),
        limitations: serde_json::from_str(&lim).unwrap_or_default(),
        store_hash: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    fn tmp() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path()).unwrap();
        (d, s)
    }
    fn node(path: &str, qn: &str, ls: u32) -> Node {
        Node {
            id: None,
            kind: NodeKind::Function,
            name: qn.rsplit('.').next().unwrap().to_string(),
            qualified_name: qn.to_string(),
            path: path.to_string(),
            line_start: ls,
            line_end: ls + 2,
            content_hash: content_hash(&["a", "b", "c"]),
            language: "python".into(),
            provider_ref: None,
        }
    }
    #[test]
    fn open_creates_db_and_schema_version() {
        let (d, s) = tmp();
        assert!(d.path().join(".comparegraph/comparegraph.db").exists());
        assert_eq!(s.schema_version().unwrap(), 1);
        let again = Store::open(d.path()).unwrap();
        assert_eq!(again.schema_version().unwrap(), 1);
    }
    #[test]
    fn open_adds_the_edge_indexes_to_a_store_created_without_them() {
        let d = tempfile::tempdir().unwrap();
        // a v1 store as the earlier binary made it: the schema minus the two edge indexes
        let dir = d.path().join(".comparegraph");
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = Connection::open(dir.join("comparegraph.db")).unwrap();
            let old: String = SCHEMA.lines().filter(|l| !l.contains("idx_edge_src") && !l.contains("idx_edge_tgt")).collect::<Vec<_>>().join("\n");
            conn.execute_batch(&old).unwrap();
            conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', '1')", []).unwrap();
        }
        let names = |s: &Store| -> Vec<String> {
            let mut st = s.conn.prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_edge_%' ORDER BY name").unwrap();
            st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
        };
        {
            let raw = Connection::open(dir.join("comparegraph.db")).unwrap();
            let n: i64 = raw.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('idx_edge_src','idx_edge_tgt')", [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0, "the old store has neither index");
        }
        let s = Store::open(d.path()).unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
        assert_eq!(names(&s), vec!["idx_edge_run", "idx_edge_src", "idx_edge_tgt"]);
    }
    #[test]
    fn indexes_do_not_change_store_hash() {
        let (_d, s) = tmp();
        let repo = s.insert_repo(&RepoRow::new("r", "/abs/r")).unwrap();
        let run = s
            .insert_run(&Run { id: None, repo, provider: "reference".into(), provider_version: "0.1.0".into(), started_at: "2026-10-12T00:00:00Z".into(), seconds: 0.0, status: RunStatus::Running, limitations: vec![], store_hash: None })
            .unwrap();
        let mut a = vec![node("a.py", "a.f", 1), node("a.py", "a.g", 10)];
        s.insert_nodes(repo, run, &mut a).unwrap();
        s.insert_edges(repo, run, &[Edge { kind: EdgeKind::Calls, source: a[0].id.unwrap(), target: a[1].id.unwrap(), confidence: 1.0 }]).unwrap();
        let with = s.store_hash(run).unwrap();
        s.conn.execute_batch("DROP INDEX IF EXISTS idx_edge_src; DROP INDEX IF EXISTS idx_edge_tgt;").unwrap();
        assert_eq!(s.store_hash(run).unwrap(), with);
    }
    #[test]
    fn open_refuses_unwritable_root_with_path_in_error() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let err = Store::open(&file).unwrap_err().to_string();
        assert!(err.contains("not-a-dir"), "{err}");
    }
    #[test]
    fn repo_run_nodes_edges_round_trip_and_hash_is_order_independent() {
        let (_d, s) = tmp();
        let repo = s.insert_repo(&RepoRow::new("r", "/abs/r")).unwrap();
        let run = s
            .insert_run(&Run {
                id: None,
                repo,
                provider: "reference".into(),
                provider_version: "0.1.0".into(),
                started_at: "2026-10-12T00:00:00Z".into(),
                seconds: 0.0,
                status: RunStatus::Running,
                limitations: vec![],
                store_hash: None,
            })
            .unwrap();
        let mut a = vec![node("a.py", "a.f", 1), node("a.py", "a.g", 10)];
        s.insert_nodes(repo, run, &mut a).unwrap();
        assert!(a.iter().all(|n| n.id.is_some()));
        s.insert_edges(
            repo,
            run,
            &[Edge {
                kind: EdgeKind::Calls,
                source: a[0].id.unwrap(),
                target: a[1].id.unwrap(),
                confidence: 1.0,
            }],
        )
        .unwrap();
        let h1 = s.store_hash(run).unwrap();
        // a second run with the same content inserted in reverse order hashes the same
        let run2 = s
            .insert_run(&Run {
                id: None,
                repo,
                provider: "reference".into(),
                provider_version: "0.1.0".into(),
                started_at: "2026-10-12T00:00:01Z".into(),
                seconds: 0.0,
                status: RunStatus::Running,
                limitations: vec![],
                store_hash: None,
            })
            .unwrap();
        let mut b = vec![node("a.py", "a.g", 10), node("a.py", "a.f", 1)];
        s.insert_nodes(repo, run2, &mut b).unwrap();
        s.insert_edges(
            repo,
            run2,
            &[Edge {
                kind: EdgeKind::Calls,
                source: b[1].id.unwrap(),
                target: b[0].id.unwrap(),
                confidence: 1.0,
            }],
        )
        .unwrap();
        assert_eq!(h1, s.store_hash(run2).unwrap());
        s.finish_run(
            run2,
            RunStatus::Done,
            1.5,
            &[Limitation::new("x", "y")],
            Some(&h1),
        )
        .unwrap();
        let latest = s.latest_run(repo).unwrap().unwrap();
        assert_eq!(latest.id, Some(run2));
        assert_eq!(latest.status, RunStatus::Done);
        assert_eq!(latest.limitations[0].code, "x");
        assert_eq!(s.nodes_of_run(run2).unwrap().len(), 2);
        assert_eq!(s.edges_of_run(run2).unwrap().len(), 1);
    }
    #[test]
    fn open_error_in_a_read_only_dot_comparegraph_names_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".comparegraph");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // permission bits do not bind root: skip when a write still succeeds
        if std::fs::write(dir.join("probe"), b"x").is_ok() {
            return;
        }
        let err = Store::open(d.path()).unwrap_err().to_string();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            err.contains(".comparegraph") && err.contains("cannot open"),
            "{err}"
        );
    }
    #[test]
    fn evidence_tables_are_append_only_in_the_database() {
        let (_d, s) = tmp();
        let c = s.conn();
        c.execute_batch(
            "INSERT INTO repo(id,name,path,added_at) VALUES (1,'r','/x','t');
             INSERT INTO run(id,repo_id,provider,provider_version,started_at,status) VALUES (1,1,'p','1','t','done');
             INSERT INTO family(id,name,definition_path) VALUES (1,'f','/f');
             INSERT INTO feature(id,family_id,grp,slug,definition,counts_as_has,tier) VALUES (1,1,'g','s','d','c','basics');
             INSERT INTO claim(id,repo_id,feature_id,run_id,verdict,confidence,author,created_at) VALUES (1,1,1,1,'has','derived','me','t');
             INSERT INTO location(id,claim_id,path,line_start,line_end,content_hash,resolved_at) VALUES (1,1,'p',1,2,'h','t');",
        )
        .unwrap();
        for sql in [
            "UPDATE claim SET verdict='none'",
            "DELETE FROM claim",
            "DELETE FROM location",
            "UPDATE location SET line_end = 9",
        ] {
            let e = c.execute(sql, []).unwrap_err().to_string();
            assert!(e.contains("append-only"), "{sql}: {e}");
        }
        // the one permitted flip
        c.execute("UPDATE location SET stale = 1", []).unwrap();
        // run / feature stay writable
        c.execute("UPDATE run SET status='done'", []).unwrap();
        c.execute("UPDATE feature SET tier='retired'", []).unwrap();
    }
    #[test]
    fn schema_version_mismatch_is_refused() {
        let (d, s) = tmp();
        s.conn()
            .execute("UPDATE meta SET value='2' WHERE key='schema_version'", [])
            .unwrap();
        drop(s);
        let err = Store::open(d.path()).unwrap_err().to_string();
        assert!(err.contains("schema_version"), "{err}");
    }
    #[test]
    fn hash_changes_when_a_nodes_content_hash_changes() {
        let (_d, s) = tmp();
        let repo = s.insert_repo(&RepoRow::new("r", "/abs/r")).unwrap();
        let mk = || {
            s.insert_run(&Run {
                id: None,
                repo,
                provider: "reference".into(),
                provider_version: "0.1.0".into(),
                started_at: "2026-10-12T00:00:00Z".into(),
                seconds: 0.0,
                status: RunStatus::Running,
                limitations: vec![],
                store_hash: None,
            })
            .unwrap()
        };
        let (r1, r2) = (mk(), mk());
        let mut a = vec![node("a.py", "a.f", 1)];
        let mut b = vec![node("a.py", "a.f", 1)];
        b[0].content_hash = content_hash(&["different"]);
        s.insert_nodes(repo, r1, &mut a).unwrap();
        s.insert_nodes(repo, r2, &mut b).unwrap();
        assert_ne!(s.store_hash(r1).unwrap(), s.store_hash(r2).unwrap());
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FamilyRow {
    pub id: i64,
    pub name: String,
    pub definition_path: String,
    pub description: String,
}
/// A feature row; `slug` holds the full pack id `<group>.<slug>`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FeatureRow {
    pub id: i64,
    pub family: String,
    pub grp: String,
    pub slug: String,
    pub title: String,
    pub definition: String,
    pub counts_as_has: String,
    pub tier: String,
    pub weight: i64,
    /// comma-separated words the pack author says also name the feature (empty when none)
    pub synonyms: String,
}
impl FeatureRow {
    /// The pack id, `<group>.<slug>`.
    pub fn key(&self) -> String {
        self.slug.clone()
    }
}
/// What a pack row contributes to the `feature` table (`id` is the full `<group>.<slug>`).
pub struct FeatureInput<'a> {
    pub id: &'a str,
    pub grp: &'a str,
    pub title: &'a str,
    pub definition: &'a str,
    pub counts_as_has: &'a str,
    pub tier: &'a str,
    pub weight: i64,
    pub synonyms: &'a str,
}

fn family_row(r: &rusqlite::Row) -> rusqlite::Result<FamilyRow> {
    Ok(FamilyRow {
        id: r.get(0)?,
        name: r.get(1)?,
        definition_path: r.get(2)?,
        description: r.get(3)?,
    })
}

impl Store {
    pub fn upsert_family(
        &self,
        name: &str,
        definition_path: &str,
        description: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO family(name, definition_path, description) VALUES (?1, ?2, ?3) \
             ON CONFLICT(name) DO UPDATE SET definition_path = excluded.definition_path, description = excluded.description",
            params![name, definition_path, description],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM family WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )?)
    }
    pub fn family_by_name(&self, name: &str) -> Result<Option<FamilyRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, definition_path, description FROM family WHERE name = ?1",
                params![name],
                family_row,
            )
            .optional()?)
    }
    pub fn families(&self) -> Result<Vec<FamilyRow>> {
        let mut st = self
            .conn
            .prepare("SELECT id, name, definition_path, description FROM family ORDER BY name")?;
        let rows = st
            .query_map([], family_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Insert or update one feature by (family, full id); the feature id and so its claims' key survive an update.
    pub fn upsert_feature(&self, family_id: i64, f: &FeatureInput) -> Result<()> {
        self.conn.execute(
            "INSERT INTO feature(family_id, grp, slug, title, definition, counts_as_has, tier, weight, synonyms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(family_id, slug) DO UPDATE SET grp = excluded.grp, title = excluded.title, definition = excluded.definition, \
             counts_as_has = excluded.counts_as_has, tier = excluded.tier, weight = excluded.weight, synonyms = excluded.synonyms",
            params![family_id, f.grp, f.id, f.title, f.definition, f.counts_as_has, f.tier, f.weight, f.synonyms],
        )?;
        Ok(())
    }
    /// All features of a family (retired included), sorted by group then id.
    pub fn features_of(&self, family: &str) -> Result<Vec<FeatureRow>> {
        let mut st = self.conn.prepare(
            "SELECT f.id, fa.name, f.grp, f.slug, f.title, f.definition, f.counts_as_has, f.tier, f.weight, f.synonyms \
             FROM feature f JOIN family fa ON fa.id = f.family_id WHERE fa.name = ?1 ORDER BY f.grp, f.slug",
        )?;
        let rows = st
            .query_map(params![family], |r| {
                Ok(FeatureRow {
                    id: r.get(0)?,
                    family: r.get(1)?,
                    grp: r.get(2)?,
                    slug: r.get(3)?,
                    title: r.get(4)?,
                    definition: r.get(5)?,
                    counts_as_has: r.get(6)?,
                    tier: r.get(7)?,
                    weight: r.get(8)?,
                    synonyms: r.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Mark every feature of the family whose `<group>.<slug>` is not in `keep_ids` as `retired`; returns the newly retired ids.
    pub fn retire_missing(&self, family: &str, keep_ids: &[String]) -> Result<Vec<String>> {
        let mut retired = Vec::new();
        for f in self.features_of(family)? {
            if f.tier != "retired" && !keep_ids.contains(&f.key()) {
                self.conn.execute(
                    "UPDATE feature SET tier = 'retired' WHERE id = ?1",
                    params![f.id],
                )?;
                retired.push(f.key());
            }
        }
        Ok(retired)
    }
}
