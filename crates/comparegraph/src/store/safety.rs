//! Store methods of the store-safety verbs (lane CG-STORE): backup, restore-store, check --mark, gc, archive flags, the
//! action-log head. Every query the new application files need lives here, so the application ring holds no new SQL.
//! None of these touches `store/schema.sql`: `meta`, `run.status`, `location.stale` and `parse_cache` are the parts the
//! schema leaves writable.
use super::{row_to_run, Result, Store, StoreError};
use crate::domain::{Run, RunId, RunStatus};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Rows of one provider version in the parse cache.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CacheVersion {
    pub provider_version: String,
    pub rows: i64,
    pub payload_bytes: i64,
}

/// One cited location of a repo's claims, with whether its claim is the live one.
#[derive(Debug, Clone)]
pub struct LocationCheckRow {
    pub id: i64,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub content_hash: String,
    pub live: bool,
    pub stale: bool,
}

/// The tables a claims-only backup carries: everything that cannot be rebuilt from the clones, plus the run rows
/// (without their graph) the claims cite. Order is insertion order for a restore (parents first).
pub const NON_REBUILDABLE: &[&str] = &["meta", "repo", "family", "feature", "run", "claim", "location", "decision", "relatedness"];
/// The tables a full backup's row counts cover (the graph too).
pub const COUNTED: &[&str] = &[
    "meta", "repo", "family", "feature", "run", "claim", "location", "decision", "relatedness", "node", "edge", "community",
    "community_member", "parse_cache",
];

impl Store {
    /// Open an existing store with SQLITE_OPEN_READ_ONLY: nothing in the file can be written by this connection, a store
    /// that predates columns is served through temp views (never migrated), and no checkpoint runs on close.
    pub fn open_read_only(root: &Path) -> Result<Store> {
        let dir = root.join(".comparegraph");
        let path = dir.join("comparegraph.db");
        if !path.is_file() {
            return Err(StoreError::NotFound(format!(
                "no workspace at {} — run `comparegraph add` first",
                dir.display()
            )));
        }
        if let Ok(mut d) = super::LAST_DIR.lock() {
            *d = Some(dir.clone());
        }
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let open_err = |source: rusqlite::Error| StoreError::Open { path: path.clone(), source };
        let conn = Connection::open_with_flags(&path, flags).map_err(open_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(super::wait_secs())).map_err(open_err)?;
        conn.execute_batch(&format!("PRAGMA cache_size = {};", super::CACHE_SIZE_KIB)).map_err(open_err)?;
        super::check_schema_version(&conn, &path)?;
        if super::needs_migration(&conn).map_err(open_err)? {
            super::shadow_missing_columns(&conn).map_err(open_err)?;
        }
        let holder = super::holder_note(&dir);
        Ok(Store { conn, path, holder, writer: false })
    }
    /// Open an existing store read-write WITHOUT migrating it: the store-safety verbs (backup, restore-store, check
    /// --mark, gc, archive, restore) write only `meta`, `location.stale`, `run.status` and `parse_cache`, which every v1
    /// store has, so a store written by an older binary keeps its column set (the chain's binary keeps reading it). A
    /// store that predates columns is read through temp views; the writes name `main.` tables, so they never hit a view.
    pub fn open_guarded(root: &Path) -> Result<Store> {
        let dir = root.join(".comparegraph");
        let path = dir.join("comparegraph.db");
        if !path.is_file() {
            return Err(StoreError::NotFound(format!(
                "no workspace at {} — run `comparegraph add` first",
                dir.display()
            )));
        }
        if let Ok(mut d) = super::LAST_DIR.lock() {
            *d = Some(dir.clone());
        }
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let open_err = |source: rusqlite::Error| if super::is_busy(&source) { super::locked() } else { StoreError::Open { path: path.clone(), source } };
        let conn = Connection::open_with_flags(&path, flags).map_err(open_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(super::wait_secs())).map_err(open_err)?;
        if conn.is_readonly(rusqlite::DatabaseName::Main).map_err(open_err)? {
            return Err(StoreError::Schema(format!("{} is read-only; this verb writes", path.display())));
        }
        conn.execute_batch(&format!(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA cache_size = {};",
            super::CACHE_SIZE_KIB
        ))
        .map_err(open_err)?;
        super::check_schema_version(&conn, &path)?;
        if super::needs_migration(&conn).map_err(open_err)? {
            super::shadow_missing_columns(&conn).map_err(open_err)?;
        }
        let holder = super::holder_note(&dir);
        Ok(Store { conn, path, holder, writer: false })
    }
    /// True when this store lacks columns or objects this binary's schema has (it was written by an older binary and
    /// has not been migrated; read through temp views).
    pub fn predates_this_binary(&self) -> Result<bool> {
        Ok(super::needs_migration(&self.conn)?)
    }
    /// Run `f` inside one read transaction: every read sees the same snapshot (WAL), whatever commits meanwhile.
    pub fn read_snapshot<T, E: From<StoreError>>(&self, f: impl FnOnce(&Store) -> std::result::Result<T, E>) -> std::result::Result<T, E> {
        self.conn.execute_batch("BEGIN DEFERRED").map_err(|e| E::from(e.into()))?;
        let r = f(self);
        let _ = self.conn.execute_batch("COMMIT");
        r
    }
    /// The `.comparegraph` directory of this store.
    pub fn dir(&self) -> PathBuf {
        self.path.parent().map(Path::to_path_buf).unwrap_or_default()
    }
    /// The other live comparegraph processes that have this store open (from their holder notes), sorted.
    pub fn other_holders(&self) -> Vec<String> {
        super::live_holders(&self.dir())
    }
    /// Bytes of the database file plus its WAL (what a copy of the store costs).
    pub fn file_bytes(&self) -> u64 {
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) + std::fs::metadata(wal).map(|m| m.len()).unwrap_or(0)
    }
    /// Row counts of the given tables (a table this store lacks counts as absent), in the order given.
    pub fn row_counts(&self, tables: &[&str]) -> Result<Vec<(String, i64)>> {
        let mut out = Vec::new();
        for t in tables {
            let exists = self
                .conn
                .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", params![t], |_| Ok(()))
                .optional()?
                .is_some();
            if exists {
                let n: i64 = self.conn.query_row(&format!("SELECT COUNT(*) FROM main.{t}"), [], |r| r.get(0))?;
                out.push((t.to_string(), n));
            }
        }
        Ok(out)
    }
    /// A consistent, compacted copy of the whole store at `out` (SQLite `VACUUM INTO`, which reads one snapshot and does
    /// not block readers or writers under WAL). `out` must not exist.
    pub fn vacuum_into(&self, out: &Path) -> Result<()> {
        self.with_views_suspended(|| {
            self.conn.execute("VACUUM INTO ?1", params![out.to_string_lossy()])?;
            Ok(())
        })
    }
    /// VACUUM re-creates every index of the main schema by name, and a temp view named as a table (the shadow of a
    /// store that predates columns) captures that name: `views may not be indexed`. Drop the temp views, run `f`,
    /// recreate them from their stored SQL (nothing in the store file changes either way).
    fn with_views_suspended<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let views: Vec<(String, String)> = {
            let mut st = self.conn.prepare("SELECT name, sql FROM temp.sqlite_master WHERE type = 'view'")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (name, _) in &views {
            self.conn.execute_batch(&format!("DROP VIEW temp.\"{name}\""))?;
        }
        let r = f();
        for (name, sql) in &views {
            // sqlite_master stores a temp view as `CREATE VIEW ...`; re-creating it needs the TEMP keyword back
            let select = sql.split_once(" AS ").map_or(sql.as_str(), |(_, s)| s);
            self.conn.execute_batch(&format!("CREATE TEMP VIEW \"{name}\" AS {select}"))?;
        }
        r
    }
    /// `VACUUM` the store, then truncate the WAL so the file size is the real one.
    pub fn vacuum(&self) -> Result<()> {
        self.with_views_suspended(|| Ok(self.conn.execute_batch("VACUUM;")?))?;
        self.checkpoint_truncate()
    }
    /// Move the WAL into the database file and truncate it.
    pub fn checkpoint_truncate(&self) -> Result<()> {
        self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }
    /// Take the write lock without waiting (BEGIN IMMEDIATE with a zero busy timeout); false when another connection
    /// holds it. The transaction is rolled back at once.
    pub fn write_lock_free(&self) -> Result<bool> {
        self.conn.busy_timeout(std::time::Duration::ZERO)?;
        let r = self.conn.execute_batch("BEGIN IMMEDIATE");
        let free = match r {
            Ok(()) => {
                self.conn.execute_batch("ROLLBACK")?;
                true
            }
            Err(e) if super::is_busy(&e) => false,
            Err(e) => return Err(e.into()),
        };
        self.conn.busy_timeout(std::time::Duration::from_secs(super::wait_secs()))?;
        Ok(free)
    }

    // ---- meta ----
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }
    /// Every `meta` row whose key starts with `prefix`, sorted by key.
    pub fn meta_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare("SELECT key, value FROM meta WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key")?;
        let rows = st.query_map(params![prefix], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO main.meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
    pub fn meta_delete(&self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM main.meta WHERE key = ?1", params![key])?;
        Ok(())
    }
    /// Run `f` under BEGIN IMMEDIATE (the store's write lock, waiting within `--wait`); committed when it returns Ok.
    pub fn with_write_lock<T, E: From<StoreError>>(&self, f: impl FnOnce(&Store) -> std::result::Result<T, E>) -> std::result::Result<T, E> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| E::from(e.into()))?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(|e| E::from(e.into()))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // ---- parse cache ----
    /// Rows and payload bytes per provider version, sorted by version.
    pub fn parse_cache_versions(&self) -> Result<Vec<CacheVersion>> {
        let mut st = self.conn.prepare(
            "SELECT provider_version, COUNT(*), COALESCE(SUM(length(payload)), 0) FROM parse_cache GROUP BY provider_version ORDER BY provider_version",
        )?;
        let rows = st
            .query_map([], |r| Ok(CacheVersion { provider_version: r.get(0)?, rows: r.get(1)?, payload_bytes: r.get(2)? }))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    /// Delete every cache row whose provider version is not in `keep`; the number of rows deleted.
    pub fn prune_parse_cache_except(&self, keep: &[String]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS cg_keep_versions(v TEXT PRIMARY KEY); DELETE FROM cg_keep_versions;")?;
        for v in keep {
            tx.execute("INSERT OR IGNORE INTO cg_keep_versions(v) VALUES (?1)", params![v])?;
        }
        let n = tx.execute("DELETE FROM main.parse_cache WHERE provider_version NOT IN (SELECT v FROM cg_keep_versions)", [])?;
        tx.execute_batch("DROP TABLE cg_keep_versions;")?;
        tx.commit()?;
        Ok(n)
    }
    /// Prune-on-build: for the files a build just parsed, delete their cache rows of every other provider version.
    pub fn prune_parse_cache_files(&self, file_hashes: &[String], version: &str) -> Result<usize> {
        if file_hashes.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut n = 0;
        {
            let mut st = tx.prepare("DELETE FROM main.parse_cache WHERE file_hash = ?1 AND provider_version != ?2")?;
            for h in file_hashes {
                n += st.execute(params![h, version])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    // ---- runs ----
    /// Every run with this status, by id.
    pub fn runs_with_status(&self, status: RunStatus) -> Result<Vec<Run>> {
        let mut st = self.conn.prepare("SELECT id,repo_id,provider,provider_version,started_at,seconds,status,limitations_json,store_hash FROM run WHERE status = ?1 ORDER BY id")?;
        let rows = st.query_map(params![status.as_str()], row_to_run)?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    /// Change only a run's status (`gc --stale-runs`); `run` is one of the tables the triggers leave writable.
    pub fn set_run_status(&self, id: RunId, status: RunStatus) -> Result<()> {
        self.conn.execute("UPDATE main.run SET status = ?2 WHERE id = ?1", params![id.0, status.as_str()])?;
        Ok(())
    }
    /// The git head a run read; None when it has none (or the column predates it).
    pub fn run_head_opt(&self, id: RunId) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT head FROM run WHERE id = ?1", params![id.0], |r| r.get(0)).optional()?.flatten())
    }
    /// Nodes of one run (a count, not the rows).
    pub fn node_count(&self, run: RunId) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM node WHERE run_id = ?1", params![run.0], |r| r.get(0))?)
    }
    /// The newest timestamp in the store: max of claim `created_at` and run `started_at` (the deterministic "now").
    pub fn newest_timestamp(&self) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT MAX(t) FROM (SELECT MAX(created_at) AS t FROM claim UNION ALL SELECT MAX(started_at) FROM run)",
            [],
            |r| r.get(0),
        )?)
    }
    /// The largest claim id (0 when there is none).
    pub fn max_claim_id(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM claim", [], |r| r.get(0))?)
    }

    // ---- relevance / archive ----
    /// Live claims (the latest per repo and feature): (repo, family, verdict, created_at), sorted.
    pub fn live_claims(&self) -> Result<Vec<(String, String, String, String)>> {
        let mut st = self.conn.prepare(
            "WITH l AS (SELECT MAX(id) AS id FROM claim GROUP BY repo_id, feature_id)              SELECT r.name, fa.name, c.verdict, c.created_at FROM l JOIN claim c ON c.id = l.id JOIN repo r ON r.id = c.repo_id              JOIN feature f ON f.id = c.feature_id JOIN family fa ON fa.id = f.family_id ORDER BY r.name, fa.name, c.id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    /// The repos whose latest decision of some scope (the repo, or one of its features) is `adopt`, sorted.
    pub fn adopted_repos(&self) -> Result<Vec<String>> {
        let mut st = self.conn.prepare(
            "WITH l AS (SELECT MAX(id) AS id FROM decision GROUP BY repo_id, scope, feature_id)              SELECT DISTINCT r.name FROM l JOIN decision d ON d.id = l.id JOIN repo r ON r.id = d.repo_id WHERE d.disposition = 'adopt' ORDER BY r.name",
        )?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    /// Every run of a repo: (id, provider_version, status, store_hash, nodes), by id.
    pub fn runs_of_repo(&self, repo: crate::domain::RepoId) -> Result<Vec<serde_json::Value>> {
        let mut st = self.conn.prepare(
            "SELECT u.id, u.provider_version, u.status, u.store_hash, (SELECT COUNT(*) FROM node n WHERE n.run_id = u.id) FROM run u WHERE u.repo_id = ?1 ORDER BY u.id",
        )?;
        let rows = st
            .query_map(params![repo.0], |r| {
                Ok(serde_json::json!({"id": r.get::<_, i64>(0)?, "provider_version": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?,
                    "store_hash": r.get::<_, Option<String>>(3)?, "nodes": r.get::<_, i64>(4)?}))
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Re-point a repo at another folder, nothing else (`restore --path`); names `main.repo`, so a store read through
    /// temp views is written, not the view.
    pub fn set_repo_path_only(&self, id: crate::domain::RepoId, path: &str) -> Result<()> {
        self.conn.execute("UPDATE main.repo SET path = ?2 WHERE id = ?1", params![id.0, path])?;
        Ok(())
    }

    // ---- locations (check) ----
    /// Every location of a repo's claims with its live flag and stored stale flag, by id.
    pub fn location_check_rows(&self, repo: crate::domain::RepoId) -> Result<Vec<LocationCheckRow>> {
        let mut st = self.conn.prepare(
            "SELECT l.id, l.path, l.line_start, l.line_end, l.content_hash, \
             c.id = (SELECT MAX(id) FROM claim x WHERE x.repo_id = c.repo_id AND x.feature_id = c.feature_id), l.stale \
             FROM location l JOIN claim c ON c.id = l.claim_id WHERE c.repo_id = ?1 ORDER BY l.id",
        )?;
        let rows = st
            .query_map(params![repo.0], |x| {
                Ok(LocationCheckRow {
                    id: x.get(0)?,
                    path: x.get(1)?,
                    line_start: x.get(2)?,
                    line_end: x.get(3)?,
                    content_hash: x.get(4)?,
                    live: x.get(5)?,
                    stale: x.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
    /// Set or clear one location's stale flag (the one column of `location` the triggers leave writable).
    pub fn set_location_stale(&self, id: i64, stale: bool) -> Result<()> {
        self.conn.execute("UPDATE main.location SET stale = ?2 WHERE id = ?1", params![id, stale as i64])?;
        Ok(())
    }

    // ---- claims-only backup ----
    /// Every row of a table as JSON values in column order, by rowid (a BLOB is `{"hex": ".."}`).
    pub fn dump_table(&self, table: &str) -> Result<(Vec<String>, Vec<Vec<serde_json::Value>>)> {
        let mut st = self.conn.prepare(&format!("SELECT * FROM main.{table} ORDER BY rowid"))?;
        let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let n = cols.len();
        let rows = st
            .query_map([], |r| {
                (0..n)
                    .map(|i| {
                        Ok(match r.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(v) => serde_json::json!(v),
                            rusqlite::types::ValueRef::Real(v) => serde_json::json!(v),
                            rusqlite::types::ValueRef::Text(t) => serde_json::Value::String(String::from_utf8_lossy(t).into_owned()),
                            rusqlite::types::ValueRef::Blob(b) => serde_json::json!({ "hex": hex::encode(b) }),
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok((cols, rows))
    }
    /// Insert dumped rows (`dump_table`'s shape) into a table of a fresh store, ids kept. Foreign keys are off for the
    /// connection while it runs (a claims-only restore has no graph, so `location.node_id` points at no node yet).
    pub fn insert_dumped(&self, table: &str, cols: &[String], rows: &[Vec<serde_json::Value>]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let have = super::table_columns(&self.conn, table)?;
        // columns the dump has and this store has; a column this store adds later keeps its default
        let idx: Vec<usize> = (0..cols.len()).filter(|i| have.contains(&cols[*i])).collect();
        let names: Vec<&str> = idx.iter().map(|i| cols[*i].as_str()).collect();
        let marks: Vec<String> = (1..=idx.len()).map(|i| format!("?{i}")).collect();
        let verb = if table == "meta" { "INSERT OR REPLACE" } else { "INSERT" };
        let sql = format!("{verb} INTO main.{table}({}) VALUES ({})", names.join(","), marks.join(","));
        let mut st = self.conn.prepare(&sql)?;
        for row in rows {
            let vals: Vec<rusqlite::types::Value> = idx
                .iter()
                .map(|i| match &row[*i] {
                    serde_json::Value::Null => rusqlite::types::Value::Null,
                    serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(*b as i64),
                    serde_json::Value::Number(n) => match n.as_i64() {
                        Some(v) => rusqlite::types::Value::Integer(v),
                        None => rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0)),
                    },
                    serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                    serde_json::Value::Object(o) => match o.get("hex").and_then(|h| h.as_str()).and_then(|h| hex::decode(h).ok()) {
                        Some(b) => rusqlite::types::Value::Blob(b),
                        None => rusqlite::types::Value::Text(serde_json::Value::Object(o.clone()).to_string()),
                    },
                    serde_json::Value::Array(a) => rusqlite::types::Value::Text(serde_json::Value::Array(a.clone()).to_string()),
                })
                .collect();
            st.execute(rusqlite::params_from_iter(vals))?;
        }
        Ok(rows.len())
    }
    /// Turn foreign-key enforcement on or off for this connection (a claims-only restore needs it off).
    pub fn set_foreign_keys(&self, on: bool) -> Result<()> {
        self.conn.execute_batch(if on { "PRAGMA foreign_keys = ON;" } else { "PRAGMA foreign_keys = OFF;" })?;
        Ok(())
    }
    /// Run statuses after a claims-only restore: every run row comes back `restored` (it has no graph).
    pub fn mark_all_runs(&self, status: RunStatus) -> Result<usize> {
        Ok(self.conn.execute("UPDATE main.run SET status = ?1, store_hash = NULL", params![status.as_str()])?)
    }
    /// A freshly created store: no repo, claim or run rows.
    pub fn is_empty_store(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM repo) + (SELECT COUNT(*) FROM claim) + (SELECT COUNT(*) FROM run)",
            [],
            |r| r.get(0),
        )?;
        Ok(n == 0)
    }
    /// The SQLite library version this binary bundles.
    pub fn sqlite_version() -> &'static str {
        rusqlite::version()
    }
    /// The stored `PRAGMA synchronous` and `cache_size` of this connection (for the settings report).
    pub fn connection_settings(&self) -> Result<serde_json::Value> {
        let sync: i64 = self.conn.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
        let cache: i64 = self.conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;
        let jm: String = self.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        Ok(serde_json::json!({ "synchronous": sync, "cache_size": cache, "journal_mode": jm }))
    }
}

/// `PRAGMA quick_check` of a database file opened read-only: "ok" or the first problem.
pub fn quick_check(path: &Path) -> Result<String> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| StoreError::Open { path: path.to_path_buf(), source })?;
    Ok(conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?)
}
/// Row counts of a database file opened read-only (a backup), for the tables given that it has.
pub fn file_row_counts(path: &Path, tables: &[&str]) -> Result<Vec<(String, i64)>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|source| StoreError::Open { path: path.to_path_buf(), source })?;
    let mut out = Vec::new();
    for t in tables {
        let exists = conn
            .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", params![t], |_| Ok(()))
            .optional()?
            .is_some();
        if exists {
            let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM main.{t}"), [], |r| r.get(0))?;
            out.push((t.to_string(), n));
        }
    }
    Ok(out)
}
