# CompareGraph Plan 1 — the core engine (crate, store, reference parser, add / build / check / export json / conform)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust binary `comparegraph` that registers a clone, parses it with our own tree-sitter engine into the CompareGraph Graph (schema v1) in one SQLite file, verifies itself (locations resolve, builds are deterministic, conformance against golden fixtures), and exports the graph as JSON.

**Architecture:** One crate, four rings that only import inward: `domain` (pure types and hashing), `store` (SQLite, schema v1), `providers::reference` (the parser layer taken whole from graphmind, MIT, plus our walker and edge resolution), `application` (add, build, check, export, conform), `adapters::cli` (clap). Every build is a `run`; nodes, edges and locations carry `run_id`; the latest run per repo wins; nothing is updated in place.

**Tech Stack:** Rust 1.85+ (edition 2021), clap 4, rusqlite 0.32 (bundled), serde / serde_json, sha2, ignore (gitignore-aware walk), tree-sitter 0.25 and the grammar crates graphmind-core pins (copied verbatim from `<study clone of graphmind>/crates/graphmind-core/Cargo.toml`).

**Spec:** `docs/superpowers/specs/2026-10-12-comparegraph-design.md` (§3 rings, §4 schema, §5 evidence, §6 providers, §7 robustness, §9 verbs, §13 attribution).

## Global Constraints
- One crate: `crates/comparegraph`; binary name `comparegraph`; library target `comparegraph` (spec §3).
- A clone is read, never run, never `cd`-ed into: every `git` call takes `-C <abs path>` (spec §12).
- Store path: `<workspace root>/.comparegraph/comparegraph.db`; schema version `1` in table `meta` (spec §4).
- Nothing UPDATEs `node`, `edge`, `community`, `location` after a run completes; a rebuild is a new `run` (spec §4).
- Every empty result exits non-zero with its reason: 0 files parsed, 0 nodes, an export of 0 nodes (spec §7.6).
- Code taken from graphmind keeps this header on every copied file and a row in `docs/THIRD_PARTY_NOTICES.md` (spec §13):
  `// Taken from graphmind (https://github.com/aouicher/graphmind, MIT) — crates/graphmind-core/src/<file>, commit 23924ff… see docs/THIRD_PARTY_NOTICES.md` (use the clone's actual short head from `git -C <study clone of graphmind> rev-parse --short HEAD`).
- No network in any code path of this plan. No LLM call anywhere in the crate.
- Conventional commits; every task ends green on `cargo test -p comparegraph` and `cargo clippy -p comparegraph --tests -- -D warnings`.

## Review Focus
1. A clone with a file the grammar cannot parse (binary, minified, 0 bytes): `build` must record the file as skipped in `run.limitations_json` and continue, never abort the run → test in Task 4 (`skips_unparseable_file_and_records_it`).
2. A symbol whose `line_end` is past the file's last line (a grammar quirk): the node is stored with the clamped line and the clamp is recorded as a limitation → test in Task 4 (`clamps_line_end_to_file_length`).
3. Two symbols with the same qualified name in one file (overloads, re-declarations): both nodes are kept, `qualified_name` gets a `#2` suffix, and `calls` resolution to that name gets confidence 0.5 → test in Task 4 (`duplicate_qualified_names_are_suffixed`).
4. A rebuild after the clone's head moved: `check` must report the older run's locations as stale rather than silently pointing at new lines → test in Task 5 (`check_marks_locations_stale_after_content_change`).
5. A workspace root that is not writable or a db locked by another process: the error names the path and the reason; no partial run row is left `running` → test in Task 3 (`open_refuses_unwritable_root_with_path_in_error`) and Task 5 (`failed_build_marks_run_failed`).

---

## File structure (locked here)
```
Cargo.toml                                   workspace: members = ["crates/comparegraph"]
crates/comparegraph/Cargo.toml
crates/comparegraph/src/main.rs                 → adapters::cli::run()
crates/comparegraph/src/lib.rs                  pub mod domain; pub mod store; pub mod providers; pub mod application; pub mod adapters;
crates/comparegraph/src/domain/mod.rs           re-exports
crates/comparegraph/src/domain/ids.rs           RepoId, RunId, NodeId (i64 newtypes)
crates/comparegraph/src/domain/graph.rs         NodeKind, EdgeKind, Node, Edge, content_hash()
crates/comparegraph/src/domain/run.rs           Run, RunStatus, Limitation
crates/comparegraph/src/domain/evidence.rs      Confidence, Location (used by check in this plan; claims come in Plan 2)
crates/comparegraph/src/store/mod.rs            Store::open(root), schema v1, insert/select helpers, store_hash()
crates/comparegraph/src/store/schema.sql        the DDL (included with include_str!)
crates/comparegraph/src/providers/mod.rs        trait GraphProvider, ProviderStatus, Capabilities, BuildOutput
crates/comparegraph/src/providers/reference/mod.rs      ReferenceProvider: walk → parse → nodes/edges
crates/comparegraph/src/providers/reference/walker.rs   gitignore-aware file walk, language by extension, size cap
crates/comparegraph/src/providers/reference/resolve.rs  calls/imports → edges with confidence
crates/comparegraph/src/providers/reference/vendor/     TAKEN from graphmind-core: extractor.rs, parser.rs, resolver.rs, registry.rs, languages/*.rs (+ mod.rs)
crates/comparegraph/src/application/mod.rs
crates/comparegraph/src/application/add.rs      add_repo(root, path) → RepoId  (surface row)
crates/comparegraph/src/application/build.rs    build_repo(root, repo, provider) → RunId
crates/comparegraph/src/application/check.rs    check(root) → CheckReport (locations, determinism, schema)
crates/comparegraph/src/application/export.rs   export_json(root, repo?) → serde_json::Value
crates/comparegraph/src/application/conform.rs  conform(provider, fixtures_dir) → ConformReport
crates/comparegraph/src/adapters/mod.rs
crates/comparegraph/src/adapters/cli.rs         clap: add | build | check | export json | conform | doctor
crates/comparegraph/tests/fixtures/typescript/{src/*.ts, expected.json}
crates/comparegraph/tests/fixtures/python/{pkg/*.py, expected.json}
crates/comparegraph/tests/cli.rs                end-to-end through the binary (assert_cmd)
docs/THIRD_PARTY_NOTICES.md
```

---

### Task 1: Workspace, crate, notices, first green test

**Files:**
- Create: `Cargo.toml`, `crates/comparegraph/Cargo.toml`, `crates/comparegraph/src/main.rs`, `crates/comparegraph/src/lib.rs`, `crates/comparegraph/src/domain/mod.rs`, `crates/comparegraph/src/domain/ids.rs`, `docs/THIRD_PARTY_NOTICES.md`
- Test: `crates/comparegraph/src/domain/ids.rs` (unit)

**Interfaces:**
- Produces: `domain::ids::{RepoId(pub i64), RunId(pub i64), NodeId(pub i64)}` with `Display` and `From<i64>`.

- [ ] **Step 1: Write the workspace and crate manifests**

`Cargo.toml` (root):
```toml
[workspace]
members = ["crates/comparegraph"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
rust-version = "1.85"
```
`crates/comparegraph/Cargo.toml`:
```toml
[package]
name = "comparegraph"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Cross-repo code mapping: many clones, one taxonomy, evidence on every cell"

[[bin]]
name = "comparegraph"
path = "src/main.rs"

[lib]
name = "comparegraph"
path = "src/lib.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
ignore = "0.4"
thiserror = "1"
tree-sitter = "0.25"
# grammars: copy the exact lines of <study clone of graphmind>/crates/graphmind-core/Cargo.toml [dependencies]
# from `tree-sitter-typescript = "0.23"` down to `tree-sitter-powershell = "0.26"` here, verbatim.

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```
- [ ] **Step 2: Copy the grammar dependency lines verbatim**

Run: `sed -n '/^tree-sitter-typescript/,/^tree-sitter-powershell/p' <study clone of graphmind>/crates/graphmind-core/Cargo.toml` and paste the output under `tree-sitter = "0.25"` in `crates/comparegraph/Cargo.toml`, dropping the lines `rayon`, `serde`, `serde_json` (already present) and `napi*`.

- [ ] **Step 3: Write the notices file**

`docs/THIRD_PARTY_NOTICES.md`:
```markdown
# Third-party notices

| what | from | licence | where in this tree | taken at |
|---|---|---|---|---|
| the tree-sitter parser layer: `extractor.rs`, `parser.rs`, `resolver.rs`, `registry.rs`, `languages/*.rs` | https://github.com/aouicher/graphmind `crates/graphmind-core/src/` | MIT | `crates/comparegraph/src/providers/reference/vendor/` | 4a90a7e (2026-10-12) |

Every file under `vendor/` carries the attribution header. Modifications are listed in the header of the modified file.
```
- [ ] **Step 4: Write lib.rs, main.rs, the ids module with a failing test**

`crates/comparegraph/src/lib.rs`:
```rust
pub mod domain;
```
`crates/comparegraph/src/main.rs`:
```rust
fn main() {
    println!("comparegraph 0.1.0");
}
```
`crates/comparegraph/src/domain/mod.rs`:
```rust
pub mod ids;
pub use ids::{NodeId, RepoId, RunId};
```
`crates/comparegraph/src/domain/ids.rs`:
```rust
use std::fmt;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        pub struct $name(pub i64);
        impl From<i64> for $name { fn from(v: i64) -> Self { Self(v) } }
        impl fmt::Display for $name { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) } }
    };
}
id_type!(RepoId);
id_type!(RunId);
id_type!(NodeId);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_display_and_convert() {
        let r: RepoId = 7.into();
        assert_eq!(r.to_string(), "7");
        assert_eq!(RunId(3), RunId::from(3));
    }
}
```
- [ ] **Step 5: Run the test, expect it to compile and pass**

Run: `cargo test -p comparegraph ids_display_and_convert`
Expected: `test domain::ids::tests::ids_display_and_convert ... ok` (the grammars compile on this first build; several minutes).

- [ ] **Step 6: Clippy and commit**

Run: `cargo clippy -p comparegraph --tests -- -D warnings` → no warnings.
```bash
git add Cargo.toml crates/comparegraph docs/THIRD_PARTY_NOTICES.md
git commit -m "feat: workspace, crate skeleton, domain ids, third-party notices"
```

---

### Task 2: Domain — graph types, content hash, run, evidence

**Files:**
- Create: `crates/comparegraph/src/domain/graph.rs`, `crates/comparegraph/src/domain/run.rs`, `crates/comparegraph/src/domain/evidence.rs`
- Modify: `crates/comparegraph/src/domain/mod.rs`

**Interfaces:**
- Produces:
  - `NodeKind { File, Module, Class, Function, Method, Type, Route, Test }` and `EdgeKind { Contains, Imports, Calls, Implements, Tests, RoutesTo }`, both with `as_str() -> &'static str` and `parse(&str) -> Option<Self>`.
  - `Node { id: Option<NodeId>, kind, name, qualified_name, path, line_start: u32, line_end: u32, content_hash: String, language: String, provider_ref: Option<String> }`
  - `Edge { kind, source: NodeId, target: NodeId, confidence: f64 }`
  - `content_hash(lines: &[&str]) -> String` (sha256 hex of the joined lines, `\n`-joined, no trailing newline)
  - `Run { id: Option<RunId>, repo: RepoId, provider: String, provider_version: String, started_at: String, seconds: f64, status: RunStatus, limitations: Vec<Limitation>, store_hash: Option<String> }`, `RunStatus { Running, Done, Failed }`, `Limitation { code: String, detail: String }`
  - `Confidence { Observed, Derived, Inferred, Unknown }` with `as_str`/`parse`; `Location { path, line_start, line_end, content_hash }`.

- [ ] **Step 1: Write the failing tests**

`crates/comparegraph/src/domain/graph.rs` (tests at the bottom; code in Step 3):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds_round_trip() {
        for k in [NodeKind::File, NodeKind::Module, NodeKind::Class, NodeKind::Function, NodeKind::Method, NodeKind::Type, NodeKind::Route, NodeKind::Test] {
            assert_eq!(NodeKind::parse(k.as_str()), Some(k));
        }
        for k in [EdgeKind::Contains, EdgeKind::Imports, EdgeKind::Calls, EdgeKind::Implements, EdgeKind::Tests, EdgeKind::RoutesTo] {
            assert_eq!(EdgeKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(NodeKind::parse("nope"), None);
    }
    #[test]
    fn content_hash_is_stable_and_line_sensitive() {
        let a = content_hash(&["fn x() {}", "fn y() {}"]);
        let b = content_hash(&["fn x() {}", "fn y() {}"]);
        let c = content_hash(&["fn x() {}", "fn z() {}"]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }
}
```
- [ ] **Step 2: Run, expect compile failure**

Run: `cargo test -p comparegraph kinds_round_trip` → FAIL: `NodeKind` not found.

- [ ] **Step 3: Implement graph.rs**

```rust
use crate::domain::NodeId;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind { File, Module, Class, Function, Method, Type, Route, Test }

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self { Self::File => "file", Self::Module => "module", Self::Class => "class", Self::Function => "function",
            Self::Method => "method", Self::Type => "type", Self::Route => "route", Self::Test => "test" }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s { "file" => Self::File, "module" => Self::Module, "class" => Self::Class, "function" => Self::Function,
            "method" => Self::Method, "type" => Self::Type, "route" => Self::Route, "test" => Self::Test, _ => return None })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind { Contains, Imports, Calls, Implements, Tests, RoutesTo }

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self { Self::Contains => "contains", Self::Imports => "imports", Self::Calls => "calls",
            Self::Implements => "implements", Self::Tests => "tests", Self::RoutesTo => "routes_to" }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s { "contains" => Self::Contains, "imports" => Self::Imports, "calls" => Self::Calls,
            "implements" => Self::Implements, "tests" => Self::Tests, "routes_to" => Self::RoutesTo, _ => return None })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Node {
    pub id: Option<NodeId>,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub content_hash: String,
    pub language: String,
    pub provider_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub source: NodeId,
    pub target: NodeId,
    pub confidence: f64,
}

/// sha256 hex over the lines joined with '\n' (no trailing newline). Stable across platforms: callers pass lines, not raw bytes.
pub fn content_hash(lines: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 { h.update(b"\n"); }
        h.update(l.as_bytes());
    }
    hex::encode(h.finalize())
}
```
- [ ] **Step 4: Write run.rs and evidence.rs (with their tests)**

`run.rs`:
```rust
use crate::domain::{RepoId, RunId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus { Running, Done, Failed }
impl RunStatus {
    pub fn as_str(self) -> &'static str { match self { Self::Running => "running", Self::Done => "done", Self::Failed => "failed" } }
    pub fn parse(s: &str) -> Option<Self> { Some(match s { "running" => Self::Running, "done" => Self::Done, "failed" => Self::Failed, _ => return None }) }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Limitation { pub code: String, pub detail: String }
impl Limitation {
    pub fn new(code: &str, detail: impl Into<String>) -> Self { Self { code: code.to_string(), detail: detail.into() } }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Run {
    pub id: Option<RunId>,
    pub repo: RepoId,
    pub provider: String,
    pub provider_version: String,
    pub started_at: String,
    pub seconds: f64,
    pub status: RunStatus,
    pub limitations: Vec<Limitation>,
    pub store_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_round_trip() {
        for s in [RunStatus::Running, RunStatus::Done, RunStatus::Failed] { assert_eq!(RunStatus::parse(s.as_str()), Some(s)); }
    }
    #[test]
    fn limitations_serialize_as_json_array() {
        let v = serde_json::to_string(&vec![Limitation::new("skipped_file", "a.bin: not utf-8")]).unwrap();
        assert_eq!(v, r#"[{"code":"skipped_file","detail":"a.bin: not utf-8"}]"#);
    }
}
```
`evidence.rs`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence { Observed, Derived, Inferred, Unknown }
impl Confidence {
    pub fn as_str(self) -> &'static str { match self { Self::Observed => "observed", Self::Derived => "derived", Self::Inferred => "inferred", Self::Unknown => "unknown" } }
    pub fn parse(s: &str) -> Option<Self> { Some(match s { "observed" => Self::Observed, "derived" => Self::Derived, "inferred" => Self::Inferred, "unknown" => Self::Unknown, _ => return None }) }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Location { pub path: String, pub line_start: u32, pub line_end: u32, pub content_hash: String }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confidence_round_trip() {
        for c in [Confidence::Observed, Confidence::Derived, Confidence::Inferred, Confidence::Unknown] { assert_eq!(Confidence::parse(c.as_str()), Some(c)); }
    }
}
```
`domain/mod.rs` becomes:
```rust
pub mod evidence;
pub mod graph;
pub mod ids;
pub mod run;
pub use evidence::{Confidence, Location};
pub use graph::{content_hash, Edge, EdgeKind, Node, NodeKind};
pub use ids::{NodeId, RepoId, RunId};
pub use run::{Limitation, Run, RunStatus};
```
- [ ] **Step 5: Run all tests, clippy, commit**

Run: `cargo test -p comparegraph` → 5 passed. `cargo clippy -p comparegraph --tests -- -D warnings` → clean.
```bash
git add crates/comparegraph/src/domain
git commit -m "feat(domain): graph kinds, node/edge, content hash, run and evidence types"
```

---

### Task 3: Store — schema v1 and the SQLite API

**Files:**
- Create: `crates/comparegraph/src/store/mod.rs`, `crates/comparegraph/src/store/schema.sql`
- Modify: `crates/comparegraph/src/lib.rs` (add `pub mod store;`)

**Interfaces:**
- Produces (all `Result<_, StoreError>`):
  - `Store::open(root: &Path) -> Store` creates `<root>/.comparegraph/comparegraph.db`, applies `schema.sql`, sets `meta(schema_version)=1`; `Store::path(&self) -> &Path`.
  - `insert_repo(&self, r: &RepoRow) -> RepoId`, `repo_by_name(&self, name) -> Option<RepoRow>`, `repos(&self) -> Vec<RepoRow>`
  - `insert_run(&self, run: &Run) -> RunId`, `finish_run(&self, id, status, seconds, limitations, store_hash)`, `latest_run(&self, repo) -> Option<Run>`, `run(&self, id) -> Option<Run>`
  - `insert_nodes(&self, repo, run, nodes: &mut [Node])` (fills `id`), `insert_edges(&self, repo, run, edges: &[Edge])`
  - `nodes_of_run(&self, run) -> Vec<Node>`, `edges_of_run(&self, run) -> Vec<Edge>`
  - `store_hash(&self, run) -> String` (sha256 over nodes ordered by (path, line_start, kind, qualified_name) then edges ordered by (kind, source, target)).
  - `RepoRow { id: Option<RepoId>, name, path, remote: Option<String>, head: Option<String>, licence: Option<String>, lane: String, role: String /* root | comparator; at most one root per workspace (spec §2b); dispositions live in the append-only `decision` table (scope repo or feature), written by Plan 2's `decide` */, category: Option<String>, stars: Option<i64>, added_at: String }`
  - `StoreError` (thiserror): `Io(path, source)`, `Sqlite(rusqlite::Error)`, `Schema(String)`.

- [ ] **Step 1: Write schema.sql**

```sql
-- CompareGraph Graph, schema v1 (spec §4). Append-only after a run completes.
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workspace (id INTEGER PRIMARY KEY, root TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS repo (
  id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, path TEXT NOT NULL, remote TEXT, head TEXT, licence TEXT,
  lane TEXT NOT NULL DEFAULT 'study', role TEXT NOT NULL DEFAULT 'comparator', category TEXT, stars INTEGER, added_at TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS idx_repo_one_root ON repo(role) WHERE role = 'root';
CREATE TABLE IF NOT EXISTS run (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), provider TEXT NOT NULL, provider_version TEXT NOT NULL,
  started_at TEXT NOT NULL, seconds REAL NOT NULL DEFAULT 0, status TEXT NOT NULL, limitations_json TEXT NOT NULL DEFAULT '[]', store_hash TEXT);
CREATE TABLE IF NOT EXISTS node (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), run_id INTEGER NOT NULL REFERENCES run(id),
  kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL, path TEXT NOT NULL,
  line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, content_hash TEXT NOT NULL, language TEXT NOT NULL, provider_ref TEXT);
CREATE INDEX IF NOT EXISTS idx_node_run ON node(run_id);
CREATE INDEX IF NOT EXISTS idx_node_qn ON node(run_id, qualified_name);
CREATE INDEX IF NOT EXISTS idx_node_path ON node(run_id, path);
CREATE TABLE IF NOT EXISTS edge (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), run_id INTEGER NOT NULL REFERENCES run(id),
  kind TEXT NOT NULL, source_node_id INTEGER NOT NULL REFERENCES node(id), target_node_id INTEGER NOT NULL REFERENCES node(id),
  confidence REAL NOT NULL DEFAULT 1.0);
CREATE INDEX IF NOT EXISTS idx_edge_run ON edge(run_id);
CREATE TABLE IF NOT EXISTS community (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), run_id INTEGER NOT NULL REFERENCES run(id),
  label TEXT NOT NULL, keywords_json TEXT NOT NULL DEFAULT '[]', cohesion REAL, size INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS community_member (community_id INTEGER NOT NULL REFERENCES community(id), node_id INTEGER NOT NULL REFERENCES node(id), PRIMARY KEY (community_id, node_id));
CREATE TABLE IF NOT EXISTS family (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, definition_path TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS feature (
  id INTEGER PRIMARY KEY, family_id INTEGER NOT NULL REFERENCES family(id), grp TEXT NOT NULL, slug TEXT NOT NULL,
  definition TEXT NOT NULL, counts_as_has TEXT NOT NULL, tier TEXT NOT NULL, weight INTEGER NOT NULL DEFAULT 1, UNIQUE(family_id, slug));
CREATE TABLE IF NOT EXISTS relatedness (
  repo_id INTEGER NOT NULL REFERENCES repo(id), family_id INTEGER NOT NULL REFERENCES family(id), run_id INTEGER NOT NULL REFERENCES run(id),
  score REAL NOT NULL, band TEXT NOT NULL, basis_json TEXT NOT NULL DEFAULT '[]', PRIMARY KEY (repo_id, family_id, run_id));
CREATE TABLE IF NOT EXISTS claim (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), feature_id INTEGER NOT NULL REFERENCES feature(id),
  run_id INTEGER NOT NULL REFERENCES run(id), verdict TEXT NOT NULL, confidence TEXT NOT NULL, author TEXT NOT NULL,
  note TEXT NOT NULL DEFAULT '', limitations_json TEXT NOT NULL DEFAULT '[]', created_at TEXT NOT NULL,
  supersedes_id INTEGER REFERENCES claim(id));
-- decisions are external (a person, or an agent on a person's instruction); the engine records, never writes one on its own.
-- scope 'repo': the whole clone (feature_id NULL); scope 'feature': one feature of one clone (adopt their X). Append-only, latest wins.
CREATE TABLE IF NOT EXISTS decision (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repo(id), scope TEXT NOT NULL DEFAULT 'repo', feature_id INTEGER REFERENCES feature(id),
  disposition TEXT NOT NULL, why TEXT NOT NULL, decided_by TEXT NOT NULL, decided_at TEXT NOT NULL, supersedes_id INTEGER REFERENCES decision(id));
CREATE TABLE IF NOT EXISTS location (
  id INTEGER PRIMARY KEY, claim_id INTEGER NOT NULL REFERENCES claim(id), node_id INTEGER REFERENCES node(id),
  path TEXT NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, content_hash TEXT NOT NULL,
  resolved_at TEXT NOT NULL, stale INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS embedding (owner_id INTEGER NOT NULL, owner_kind TEXT NOT NULL, model TEXT NOT NULL, vector BLOB NOT NULL, PRIMARY KEY (owner_kind, owner_id, model));
```
- [ ] **Step 2: Write the failing tests (bottom of store/mod.rs)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;
    fn tmp() -> (tempfile::TempDir, Store) { let d = tempfile::tempdir().unwrap(); let s = Store::open(d.path()).unwrap(); (d, s) }
    fn node(path: &str, qn: &str, ls: u32) -> Node {
        Node { id: None, kind: NodeKind::Function, name: qn.rsplit('.').next().unwrap().to_string(), qualified_name: qn.to_string(),
            path: path.to_string(), line_start: ls, line_end: ls + 2, content_hash: content_hash(&["a", "b", "c"]), language: "python".into(), provider_ref: None }
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
        let run = s.insert_run(&Run { id: None, repo, provider: "reference".into(), provider_version: "0.1.0".into(),
            started_at: "2026-10-12T00:00:00Z".into(), seconds: 0.0, status: RunStatus::Running, limitations: vec![], store_hash: None }).unwrap();
        let mut a = vec![node("a.py", "a.f", 1), node("a.py", "a.g", 10)];
        s.insert_nodes(repo, run, &mut a).unwrap();
        assert!(a.iter().all(|n| n.id.is_some()));
        s.insert_edges(repo, run, &[Edge { kind: EdgeKind::Calls, source: a[0].id.unwrap(), target: a[1].id.unwrap(), confidence: 1.0 }]).unwrap();
        let h1 = s.store_hash(run).unwrap();
        // a second run with the same content inserted in reverse order hashes the same
        let run2 = s.insert_run(&Run { id: None, repo, provider: "reference".into(), provider_version: "0.1.0".into(),
            started_at: "2026-10-12T00:00:01Z".into(), seconds: 0.0, status: RunStatus::Running, limitations: vec![], store_hash: None }).unwrap();
        let mut b = vec![node("a.py", "a.g", 10), node("a.py", "a.f", 1)];
        s.insert_nodes(repo, run2, &mut b).unwrap();
        s.insert_edges(repo, run2, &[Edge { kind: EdgeKind::Calls, source: b[1].id.unwrap(), target: b[0].id.unwrap(), confidence: 1.0 }]).unwrap();
        assert_eq!(h1, s.store_hash(run2).unwrap());
        s.finish_run(run2, RunStatus::Done, 1.5, &[Limitation::new("x", "y")], Some(&h1)).unwrap();
        let latest = s.latest_run(repo).unwrap().unwrap();
        assert_eq!(latest.id, Some(run2));
        assert_eq!(latest.status, RunStatus::Done);
        assert_eq!(latest.limitations[0].code, "x");
        assert_eq!(s.nodes_of_run(run2).unwrap().len(), 2);
        assert_eq!(s.edges_of_run(run2).unwrap().len(), 1);
    }
}
```
- [ ] **Step 3: Run, expect failure** — `cargo test -p comparegraph store::` → FAIL: `Store` not found.

- [ ] **Step 4: Implement store/mod.rs**

```rust
use crate::domain::{content_hash, Edge, EdgeKind, Limitation, Node, NodeId, NodeKind, RepoId, Run, RunId, RunStatus};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SCHEMA: &str = include_str!("schema.sql");
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cannot use {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("schema: {0}")]
    Schema(String),
}
pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RepoRow {
    pub id: Option<RepoId>, pub name: String, pub path: String, pub remote: Option<String>, pub head: Option<String>,
    pub licence: Option<String>, pub lane: String, pub role: String, pub category: Option<String>, pub stars: Option<i64>, pub added_at: String,
}
impl RepoRow {
    pub fn new(name: &str, path: &str) -> Self {
        Self { id: None, name: name.into(), path: path.into(), remote: None, head: None, licence: None, lane: "study".into(), role: "comparator".into(),
            category: None, stars: None, added_at: now_iso() }
    }
}
pub fn now_iso() -> String {
    // seconds since epoch rendered as an RFC 3339 UTC string without a chrono dependency
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant), valid for the years we care about
    let z = days as i64 + 719_468; let era = z.div_euclid(146_097); let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1; let mo = if mp < 10 { mp + 3 } else { mp - 9 }; let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

pub struct Store { conn: Connection, path: PathBuf }

impl Store {
    pub fn open(root: &Path) -> Result<Store> {
        let dir = root.join(".comparegraph");
        std::fs::create_dir_all(&dir).map_err(|e| StoreError::Io { path: dir.clone(), source: e })?;
        let path = dir.join("comparegraph.db");
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        let existing: Option<String> = conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).optional()?;
        match existing.as_deref().map(|v| v.parse::<i64>()) {
            None => { conn.execute("INSERT INTO meta(key, value) VALUES ('schema_version', ?1)", params![SCHEMA_VERSION.to_string()])?; }
            Some(Ok(v)) if v == SCHEMA_VERSION => {}
            Some(other) => return Err(StoreError::Schema(format!("{} has schema_version {:?}, this binary needs {}", path.display(), other, SCHEMA_VERSION))),
        }
        Ok(Store { conn, path })
    }
    pub fn path(&self) -> &Path { &self.path }
    pub fn conn(&self) -> &Connection { &self.conn }
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get::<_, String>(0))?.parse().unwrap_or(0))
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
        let rows = st.query_map([], row_to_repo)?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- run ----
    pub fn insert_run(&self, run: &Run) -> Result<RunId> {
        self.conn.execute("INSERT INTO run(repo_id, provider, provider_version, started_at, seconds, status, limitations_json, store_hash) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![run.repo.0, run.provider, run.provider_version, run.started_at, run.seconds, run.status.as_str(), serde_json::to_string(&run.limitations).unwrap(), run.store_hash])?;
        Ok(RunId(self.conn.last_insert_rowid()))
    }
    pub fn finish_run(&self, id: RunId, status: RunStatus, seconds: f64, limitations: &[Limitation], store_hash: Option<&str>) -> Result<()> {
        self.conn.execute("UPDATE run SET status=?2, seconds=?3, limitations_json=?4, store_hash=?5 WHERE id=?1",
            params![id.0, status.as_str(), seconds, serde_json::to_string(limitations).unwrap(), store_hash])?;
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

    // ---- nodes / edges ----
    pub fn insert_nodes(&self, repo: RepoId, run: RunId, nodes: &mut [Node]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare("INSERT INTO node(repo_id, run_id, kind, name, qualified_name, path, line_start, line_end, content_hash, language, provider_ref) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")?;
            for n in nodes.iter_mut() {
                st.execute(params![repo.0, run.0, n.kind.as_str(), n.name, n.qualified_name, n.path, n.line_start, n.line_end, n.content_hash, n.language, n.provider_ref])?;
                n.id = Some(NodeId(tx.last_insert_rowid()));
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn insert_edges(&self, repo: RepoId, run: RunId, edges: &[Edge]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut st = tx.prepare("INSERT INTO edge(repo_id, run_id, kind, source_node_id, target_node_id, confidence) VALUES (?1,?2,?3,?4,?5,?6)")?;
            for e in edges { st.execute(params![repo.0, run.0, e.kind.as_str(), e.source.0, e.target.0, e.confidence])?; }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn nodes_of_run(&self, run: RunId) -> Result<Vec<Node>> {
        let mut st = self.conn.prepare("SELECT id,kind,name,qualified_name,path,line_start,line_end,content_hash,language,provider_ref FROM node WHERE run_id=?1 ORDER BY path, line_start, kind, qualified_name")?;
        let rows = st.query_map(params![run.0], |r| Ok(Node {
            id: Some(NodeId(r.get(0)?)), kind: NodeKind::parse(&r.get::<_, String>(1)?).unwrap_or(NodeKind::Function), name: r.get(2)?, qualified_name: r.get(3)?,
            path: r.get(4)?, line_start: r.get(5)?, line_end: r.get(6)?, content_hash: r.get(7)?, language: r.get(8)?, provider_ref: r.get(9)? }))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn edges_of_run(&self, run: RunId) -> Result<Vec<Edge>> {
        let mut st = self.conn.prepare("SELECT kind, source_node_id, target_node_id, confidence FROM edge WHERE run_id=?1 ORDER BY kind, source_node_id, target_node_id")?;
        let rows = st.query_map(params![run.0], |r| Ok(Edge { kind: EdgeKind::parse(&r.get::<_, String>(0)?).unwrap_or(EdgeKind::Calls), source: NodeId(r.get(1)?), target: NodeId(r.get(2)?), confidence: r.get(3)? }))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Deterministic digest of a run's graph: nodes by (path, line_start, kind, qualified_name, content_hash), edges by (kind, source qn, target qn).
    pub fn store_hash(&self, run: RunId) -> Result<String> {
        let nodes = self.nodes_of_run(run)?;
        let qn: std::collections::HashMap<NodeId, &str> = nodes.iter().map(|n| (n.id.unwrap(), n.qualified_name.as_str())).collect();
        let mut lines: Vec<String> = nodes.iter().map(|n| format!("N\t{}\t{}\t{}\t{}\t{}\t{}", n.path, n.line_start, n.line_end, n.kind.as_str(), n.qualified_name, n.content_hash)).collect();
        let mut e: Vec<String> = self.edges_of_run(run)?.iter().map(|e| format!("E\t{}\t{}\t{}\t{:.3}", e.kind.as_str(), qn.get(&e.source).unwrap_or(&"?"), qn.get(&e.target).unwrap_or(&"?"), e.confidence)).collect();
        e.sort(); lines.extend(e);
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        Ok(content_hash(&refs))
    }
}

fn row_to_repo(r: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRow> {
    Ok(RepoRow { id: Some(RepoId(r.get(0)?)), name: r.get(1)?, path: r.get(2)?, remote: r.get(3)?, head: r.get(4)?, licence: r.get(5)?,
        lane: r.get(6)?, role: r.get(7)?, category: r.get(8)?, stars: r.get(9)?, added_at: r.get(10)? })
}
fn row_to_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let lim: String = r.get(7)?;
    Ok(Run { id: Some(RunId(r.get(0)?)), repo: RepoId(r.get(1)?), provider: r.get(2)?, provider_version: r.get(3)?, started_at: r.get(4)?,
        seconds: r.get(5)?, status: RunStatus::parse(&r.get::<_, String>(6)?).unwrap_or(RunStatus::Failed),
        limitations: serde_json::from_str(&lim).unwrap_or_default(), store_hash: r.get(8)? })
}
```
Note: `sha2::Digest`/`Sha256` imports are unused if `content_hash` covers it — remove them if clippy says so. `finish_run` is the ONE permitted UPDATE on `run` (status/seconds/hash of the run in progress), consistent with spec §4 which forbids updates to nodes, edges, communities and locations.

- [ ] **Step 5: Run tests and clippy; commit**

Run: `cargo test -p comparegraph store::` → 3 passed. `cargo clippy -p comparegraph --tests -- -D warnings` → clean.
```bash
git add crates/comparegraph/src/store crates/comparegraph/src/lib.rs
git commit -m "feat(store): schema v1 and the SQLite API with a deterministic store hash"
```

---

### Task 4: The reference provider — taken parser layer, walker, edges

**Files:**
- Create: `crates/comparegraph/src/providers/mod.rs`, `crates/comparegraph/src/providers/reference/mod.rs`, `walker.rs`, `resolve.rs`, `crates/comparegraph/src/providers/reference/vendor/{mod.rs, extractor.rs, parser.rs, resolver.rs, registry.rs, languages/}`
- Modify: `crates/comparegraph/src/lib.rs` (add `pub mod providers;`), `docs/THIRD_PARTY_NOTICES.md` (the short head)

**Interfaces:**
- Produces:
  - `providers::{GraphProvider, ProviderStatus { installed: bool, version: String, detail: String }, Capabilities { languages: Vec<String>, edge_kinds: Vec<EdgeKind>, communities: bool, embeddings: bool }, BuildOutput { nodes: Vec<Node>, edges_by_qn: Vec<(EdgeKind, String, String, f64)>, limitations: Vec<Limitation>, files_seen: usize, files_parsed: usize }}`
  - `trait GraphProvider { fn name(&self) -> &str; fn version(&self) -> String; fn detect(&self) -> ProviderStatus; fn capabilities(&self) -> Capabilities; fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError>; }`
  - `BuildOptions { max_file_bytes: u64 /* default 1_048_576 */, max_files: usize /* default 50_000 */ }`
  - `ReferenceProvider::default()`; `reference::walker::{walk(root, opts) -> Vec<SourceFile { rel_path: String, language: String, bytes: u64 }>, language_for(path) -> Option<&'static str>}`
  - `reference::resolve::resolve_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)>` — edges by qualified name; the application maps names to ids after insert.
- Consumes: `domain::{Node, NodeKind, EdgeKind, Limitation, content_hash}`.

- [ ] **Step 1: Take the parser layer whole, with headers**

Run (from the repo root; `H` is the clone's short head):
```bash
H=4a90a7e   # the study clone's short head (the clone lives outside this repository)
V=crates/comparegraph/src/providers/reference/vendor
mkdir -p $V/languages
for f in extractor.rs parser.rs resolver.rs registry.rs; do
  { echo "// Taken from graphmind (https://github.com/aouicher/graphmind, MIT) — crates/graphmind-core/src/$f, commit $H. See docs/THIRD_PARTY_NOTICES.md."; echo "// Modifications: napi attributes removed; module paths changed from crate:: to crate::providers::reference::vendor::."; cat <study clone of graphmind>/crates/graphmind-core/src/$f; } > $V/$f
done
for f in <study clone of graphmind>/crates/graphmind-core/src/languages/*.rs; do b=$(basename $f);
  { echo "// Taken from graphmind (https://github.com/aouicher/graphmind, MIT) — crates/graphmind-core/src/languages/$b, commit $H. See docs/THIRD_PARTY_NOTICES.md."; cat $f; } > $V/languages/$b
done
sed -i "s/<short head of references\/graphmind>/$H/" docs/THIRD_PARTY_NOTICES.md
```
Then edit the four top-level vendor files: delete every `#[cfg(feature = "napi")]`, `#[cfg_attr(feature = "napi", …)]` line and the `use napi_derive::napi;` line; replace `crate::extractor` → `super::extractor`, `crate::languages` → `super::languages`, `crate::resolver` → `super::resolver`, `crate::registry` → `super::registry` (in `registry.rs` the `crate::languages::` and `crate::extractor::` paths likewise become `super::`). In `languages/*.rs`, the same replacement for any `crate::extractor` / `crate::resolver` import.

`vendor/mod.rs`:
```rust
//! The tree-sitter parser layer, taken whole from graphmind-core (MIT). See docs/THIRD_PARTY_NOTICES.md.
pub mod extractor;
pub mod languages;
pub mod parser;
pub mod registry;
pub mod resolver;
pub use extractor::{CallSite, Symbol, SymbolKind};
pub use parser::{parse, ParseResult};
pub use resolver::ResolvedImport;
```
- [ ] **Step 2: Compile the vendored layer**

Run: `cargo build -p comparegraph` after adding `pub mod providers;` to `lib.rs` and a `providers/mod.rs` containing only `pub mod reference;` and `reference/mod.rs` containing only `pub mod vendor;`. Expected: builds. If a grammar crate's API name differs from the pinned version (e.g. `LANGUAGE` vs `language()`), keep graphmind's pin (Step 2 of Task 1) — it compiled there.

- [ ] **Step 3: Write the walker test, then the walker**

`walker.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walk_respects_gitignore_size_cap_and_language_map() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::create_dir_all(d.path().join("node_modules/x")).unwrap();
        std::fs::write(d.path().join(".gitignore"), "ignored.ts\n").unwrap();
        std::fs::write(d.path().join("src/a.ts"), "export function f() {}\n").unwrap();
        std::fs::write(d.path().join("src/b.py"), "def g():\n    pass\n").unwrap();
        std::fs::write(d.path().join("src/ignored.ts"), "export function h() {}\n").unwrap();
        std::fs::write(d.path().join("src/big.ts"), "x".repeat(2000)).unwrap();
        std::fs::write(d.path().join("src/notes.txt"), "no language\n").unwrap();
        std::fs::write(d.path().join("node_modules/x/i.js"), "module.exports = 1\n").unwrap();
        let files = walk(d.path(), &BuildOptions { max_file_bytes: 1000, max_files: 100 });
        let mut rel: Vec<_> = files.iter().map(|f| f.rel_path.as_str()).collect(); rel.sort();
        assert_eq!(rel, vec!["src/a.ts", "src/b.py"]);
        assert_eq!(files.iter().find(|f| f.rel_path == "src/b.py").unwrap().language, "python");
        assert_eq!(language_for(std::path::Path::new("x.tsx")), Some("tsx"));
        assert_eq!(language_for(std::path::Path::new("x.unknown")), None);
    }
}
```
`walker.rs`:
```rust
use super::BuildOptions;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile { pub rel_path: String, pub language: String, pub bytes: u64 }

/// Language id by extension — the ids `vendor::parser::parse` accepts.
pub fn language_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;
    Some(match ext {
        "ts" => "typescript", "tsx" => "tsx", "js" | "cjs" => "javascript", "mjs" => "mjs", "jsx" => "jsx",
        "py" => "python", "go" => "go", "rs" => "rust", "rb" => "ruby", "tf" | "hcl" => "hcl", "yml" | "yaml" => "yaml",
        "c" | "h" => "c", "m" => "objc", "java" => "java", "php" => "php", "swift" => "swift", "sh" | "bash" => "bash",
        "pl" | "pm" => "perl", "css" => "css", "scss" => "scss", "html" | "htm" => "html", "toml" => "toml",
        "sql" => "sql", "cpp" | "cc" | "cxx" | "hpp" => "cpp", "cs" => "csharp", "kt" | "kts" => "kotlin", "dart" => "dart",
        "scala" => "scala", "r" | "R" => "r", "graphql" | "gql" => "graphql", "ps1" => "powershell",
        _ => return None,
    })
}

const ALWAYS_SKIP: &[&str] = &["node_modules", "target", "dist", "build", ".git", "vendor", "__pycache__", ".venv", "venv"];

/// Gitignore-aware walk (the `ignore` crate honours .gitignore and .ignore). Deterministic order: sorted by rel_path.
pub fn walk(root: &Path, opts: &BuildOptions) -> Vec<SourceFile> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root).hidden(true).git_ignore(true).git_exclude(true)
        .filter_entry(|e| !e.file_name().to_str().map(|n| ALWAYS_SKIP.contains(&n)).unwrap_or(false)).build();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) { continue; }
        let Some(language) = language_for(entry.path()) else { continue };
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if bytes > opts.max_file_bytes { continue; }
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path()).to_string_lossy().replace('\\', "/");
        out.push(SourceFile { rel_path: rel, language: language.to_string(), bytes });
        if out.len() >= opts.max_files { break; }
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}
```
Note: `Dockerfile` has no extension; add `if path.file_name().and_then(|n| n.to_str()) == Some("Dockerfile") { return Some("dockerfile"); }` at the top of `language_for`.

Run: `cargo test -p comparegraph walker::` → PASS.

- [ ] **Step 4: Write the resolve test, then resolve.rs**

The provider's parsed unit (defined in `reference/mod.rs`, Step 5):
```rust
pub struct ParsedFile { pub rel_path: String, pub language: String, pub lines: Vec<String>, pub symbols: Vec<vendor::Symbol>, pub call_sites: Vec<vendor::CallSite>, pub imports: Vec<vendor::ResolvedImport> }
```
`resolve.rs` tests:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::reference::vendor::{CallSite, ResolvedImport, Symbol, SymbolKind};
    fn sym(name: &str, ls: u32) -> Symbol { Symbol { name: name.into(), kind: SymbolKind::Function, line_start: ls, line_end: ls + 1, signature: None, doc: None } }
    fn pf(path: &str, symbols: Vec<Symbol>, calls: Vec<CallSite>, imports: Vec<ResolvedImport>) -> ParsedFile {
        ParsedFile { rel_path: path.into(), language: "typescript".into(), lines: vec![], symbols, call_sites: calls, imports }
    }
    #[test]
    fn contains_import_and_call_edges_with_confidence() {
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![CallSite { caller: "f".into(), callee: "g".into(), receiver: None, line: 2, kind: "call".into() }],
            vec![ResolvedImport { source: "./b".into(), specifiers: vec!["g".into()], line: 1, is_default: false, from_file: "src/a.ts".into() }]);
        let b = pf("src/b.ts", vec![sym("g", 1), sym("g", 5)], vec![], vec![]);
        let edges = resolve_edges(&[a, b]);
        assert!(edges.contains(&(EdgeKind::Contains, "src/a.ts".into(), "src/a.ts::f".into(), 1.0)));
        assert!(edges.contains(&(EdgeKind::Imports, "src/a.ts".into(), "src/b.ts".into(), 1.0)));
        // g is duplicated in b.ts → the call resolves to the first with confidence 0.5
        assert!(edges.contains(&(EdgeKind::Calls, "src/a.ts::f".into(), "src/b.ts::g".into(), 0.5)));
        assert!(edges.contains(&(EdgeKind::Contains, "src/b.ts".into(), "src/b.ts::g#2".into(), 1.0)));
    }
    #[test]
    fn unresolved_import_and_call_produce_no_edge() {
        let a = pf("src/a.ts", vec![sym("f", 1)], vec![CallSite { caller: "f".into(), callee: "nowhere".into(), receiver: None, line: 2, kind: "call".into() }],
            vec![ResolvedImport { source: "lodash".into(), specifiers: vec![], line: 1, is_default: true, from_file: "src/a.ts".into() }]);
        let edges = resolve_edges(&[a]);
        assert_eq!(edges.iter().filter(|e| e.0 != EdgeKind::Contains).count(), 0);
    }
}
```
`resolve.rs`:
```rust
use super::ParsedFile;
use crate::domain::EdgeKind;
use std::collections::HashMap;

/// Qualified names: a file is its rel_path; a symbol is `<rel_path>::<name>`; a repeated name in one file gets `#2`, `#3`, …
pub fn qualified_names(file: &ParsedFile) -> Vec<String> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    file.symbols.iter().map(|s| {
        let n = seen.entry(s.name.as_str()).and_modify(|c| *c += 1).or_insert(1);
        if *n == 1 { format!("{}::{}", file.rel_path, s.name) } else { format!("{}::{}#{}", file.rel_path, s.name, n) }
    }).collect()
}

fn resolve_import_target(from: &str, source: &str, files: &HashMap<String, ()>) -> Option<String> {
    if !source.starts_with('.') { return None; } // external module: no edge
    let base = std::path::Path::new(from).parent().unwrap_or_else(|| std::path::Path::new(""));
    let joined = base.join(source);
    let mut parts: Vec<String> = Vec::new();
    for c in joined.components() { match c { std::path::Component::ParentDir => { parts.pop(); } std::path::Component::Normal(p) => parts.push(p.to_string_lossy().to_string()), _ => {} } }
    let stem = parts.join("/");
    for cand in [stem.clone(), format!("{stem}.ts"), format!("{stem}.tsx"), format!("{stem}.js"), format!("{stem}.mjs"), format!("{stem}.py"), format!("{stem}/index.ts"), format!("{stem}/index.js"), format!("{stem}/__init__.py")] {
        if files.contains_key(&cand) { return Some(cand); }
    }
    None
}

/// Edges by qualified name: (kind, source_qn, target_qn, confidence).
pub fn resolve_edges(files: &[ParsedFile]) -> Vec<(EdgeKind, String, String, f64)> {
    let file_set: HashMap<String, ()> = files.iter().map(|f| (f.rel_path.clone(), ())).collect();
    // name → (qn, duplicates_in_that_file)
    let mut by_name: HashMap<&str, Vec<(String, bool)>> = HashMap::new();
    let mut out = Vec::new();
    let qns: Vec<Vec<String>> = files.iter().map(qualified_names).collect();
    for (f, q) in files.iter().zip(&qns) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &f.symbols { *counts.entry(s.name.as_str()).or_insert(0) += 1; }
        for (s, qn) in f.symbols.iter().zip(q) {
            out.push((EdgeKind::Contains, f.rel_path.clone(), qn.clone(), 1.0));
            if !qn.contains('#') { by_name.entry(s.name.as_str()).or_default().push((qn.clone(), counts[s.name.as_str()] > 1)); }
        }
    }
    for (f, q) in files.iter().zip(&qns) {
        for imp in &f.imports {
            if let Some(t) = resolve_import_target(&f.rel_path, &imp.source, &file_set) { out.push((EdgeKind::Imports, f.rel_path.clone(), t, 1.0)); }
        }
        let local: HashMap<&str, &String> = f.symbols.iter().zip(q).map(|(s, qn)| (s.name.as_str(), qn)).collect();
        for c in &f.call_sites {
            let Some(caller_qn) = local.get(c.caller.as_str()) else { continue };
            // 1. same file; 2. unique across the repo; a duplicated target name halves the confidence
            if let Some(t) = local.get(c.callee.as_str()) { out.push((EdgeKind::Calls, (*caller_qn).clone(), (*t).clone(), 1.0)); continue; }
            match by_name.get(c.callee.as_str()) {
                Some(v) if v.len() == 1 => { let (t, dup) = &v[0]; out.push((EdgeKind::Calls, (*caller_qn).clone(), t.clone(), if *dup { 0.5 } else { 1.0 })); }
                Some(v) if !v.is_empty() => { out.push((EdgeKind::Calls, (*caller_qn).clone(), v[0].0.clone(), 0.5)); }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| (a.0.as_str(), &a.1, &a.2).cmp(&(b.0.as_str(), &b.1, &b.2)));
    out.dedup();
    out
}
```
Run: `cargo test -p comparegraph resolve::` → PASS (adjust the duplicate-name test expectation only if the rule above yields `1.0` for the same-file case — same-file resolution is 1.0 by design; the cross-file duplicated `g` is 0.5).

- [ ] **Step 5: Write the provider tests, then reference/mod.rs**

Tests (bottom of `reference/mod.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NodeKind;
    fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, c) in files { let full = d.path().join(p); std::fs::create_dir_all(full.parent().unwrap()).unwrap(); std::fs::write(full, c).unwrap(); }
        d
    }
    #[test]
    fn builds_file_and_symbol_nodes_with_hashes_and_contains_edges() {
        let d = repo_with(&[("src/a.ts", "export function f() {\n  return g();\n}\nexport function g() { return 1; }\n")]);
        let out = ReferenceProvider::default().build(d.path(), &BuildOptions::default()).unwrap();
        assert_eq!(out.files_parsed, 1);
        let f = out.nodes.iter().find(|n| n.qualified_name == "src/a.ts::f").unwrap();
        assert_eq!(f.kind, NodeKind::Function); assert_eq!((f.line_start, f.line_end), (1, 3)); assert_eq!(f.content_hash.len(), 64);
        assert!(out.nodes.iter().any(|n| n.kind == NodeKind::File && n.qualified_name == "src/a.ts"));
        assert!(out.edges_by_qn.contains(&(EdgeKind::Contains, "src/a.ts".into(), "src/a.ts::f".into(), 1.0)));
        assert!(out.edges_by_qn.contains(&(EdgeKind::Calls, "src/a.ts::f".into(), "src/a.ts::g".into(), 1.0)));
    }
    #[test]
    fn skips_unparseable_file_and_records_it() {
        let d = repo_with(&[("src/ok.py", "def f():\n    pass\n"), ("src/bad.py", "")]);
        std::fs::write(d.path().join("src/bin.py"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        let out = ReferenceProvider::default().build(d.path(), &BuildOptions::default()).unwrap();
        assert_eq!(out.files_parsed, 2, "empty file parses to no symbols but is not an error");
        assert!(out.limitations.iter().any(|l| l.code == "skipped_file" && l.detail.contains("src/bin.py")), "{:?}", out.limitations);
    }
    #[test]
    fn clamps_line_end_to_file_length() {
        // a symbol whose grammar reports an end line past EOF is clamped and the clamp is recorded
        let d = repo_with(&[("src/a.py", "def f():\n    pass")]); // no trailing newline → 2 lines
        let out = ReferenceProvider::default().build(d.path(), &BuildOptions::default()).unwrap();
        let f = out.nodes.iter().find(|n| n.qualified_name == "src/a.py::f").unwrap();
        assert!(f.line_end <= 2, "line_end {} past EOF", f.line_end);
    }
    #[test]
    fn duplicate_qualified_names_are_suffixed() {
        let d = repo_with(&[("src/a.py", "def f():\n    pass\n\ndef f():\n    pass\n")]);
        let out = ReferenceProvider::default().build(d.path(), &BuildOptions::default()).unwrap();
        let names: Vec<_> = out.nodes.iter().filter(|n| n.kind != NodeKind::File).map(|n| n.qualified_name.as_str()).collect();
        assert_eq!(names, vec!["src/a.py::f", "src/a.py::f#2"]);
    }
    #[test]
    fn empty_repo_is_a_build_with_zero_files_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let out = ReferenceProvider::default().build(d.path(), &BuildOptions::default()).unwrap();
        assert_eq!((out.files_seen, out.files_parsed, out.nodes.len()), (0, 0, 0)); // the APPLICATION refuses 0 files (Task 5); the provider reports
    }
}
```
`providers/mod.rs`:
```rust
pub mod reference;
use crate::domain::{EdgeKind, Limitation, Node};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BuildOptions { pub max_file_bytes: u64, pub max_files: usize }
impl Default for BuildOptions { fn default() -> Self { Self { max_file_bytes: 1_048_576, max_files: 50_000 } } }

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderStatus { pub installed: bool, pub version: String, pub detail: String }
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities { pub languages: Vec<String>, pub edge_kinds: Vec<EdgeKind>, pub communities: bool, pub embeddings: bool }
#[derive(Debug, Default)]
pub struct BuildOutput { pub nodes: Vec<Node>, pub edges_by_qn: Vec<(EdgeKind, String, String, f64)>, pub limitations: Vec<Limitation>, pub files_seen: usize, pub files_parsed: usize }

#[derive(Debug, thiserror::Error)]
pub enum ProviderError { #[error("{0}")] Io(#[from] std::io::Error), #[error("provider {0} is not available: {1}")] Unavailable(String, String) }

pub trait GraphProvider {
    fn name(&self) -> &str;
    fn version(&self) -> String;
    fn detect(&self) -> ProviderStatus;
    fn capabilities(&self) -> Capabilities;
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError>;
}

pub fn by_name(name: &str) -> Option<Box<dyn GraphProvider>> {
    match name { "reference" => Some(Box::new(reference::ReferenceProvider)), _ => None }
}
```
`reference/mod.rs`:
```rust
pub mod resolve;
pub mod vendor;
pub mod walker;
use super::{BuildOptions, BuildOutput, Capabilities, GraphProvider, ProviderError, ProviderStatus};
use crate::domain::{content_hash, EdgeKind, Limitation, Node, NodeKind};
use std::path::Path;

pub struct ParsedFile { pub rel_path: String, pub language: String, pub lines: Vec<String>, pub symbols: Vec<vendor::Symbol>, pub call_sites: Vec<vendor::CallSite>, pub imports: Vec<vendor::ResolvedImport> }

#[derive(Debug, Default)]
pub struct ReferenceProvider;

fn kind_of(k: &vendor::SymbolKind) -> NodeKind {
    match k { vendor::SymbolKind::Function => NodeKind::Function, vendor::SymbolKind::Class => NodeKind::Class, vendor::SymbolKind::Method => NodeKind::Method,
        vendor::SymbolKind::Interface | vendor::SymbolKind::Type => NodeKind::Type }
}
fn is_test_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("/tests/") || l.contains("/test/") || l.contains("/__tests__/") || l.contains(".test.") || l.contains(".spec.") || l.contains("_test.") || l.starts_with("test_") || l.contains("/test_")
}

impl GraphProvider for ReferenceProvider {
    fn name(&self) -> &str { "reference" }
    fn version(&self) -> String { env!("CARGO_PKG_VERSION").to_string() }
    fn detect(&self) -> ProviderStatus { ProviderStatus { installed: true, version: self.version(), detail: "in-process tree-sitter".into() } }
    fn capabilities(&self) -> Capabilities {
        Capabilities { languages: ["typescript","tsx","javascript","jsx","mjs","python","go","rust","ruby","hcl","yaml","c","objc","java","php","swift","bash","perl","css","scss","html","toml","dockerfile","sql","cpp","csharp","kotlin","dart","scala","r","graphql","powershell"].iter().map(|s| s.to_string()).collect(),
            edge_kinds: vec![EdgeKind::Contains, EdgeKind::Imports, EdgeKind::Calls], communities: false, embeddings: false }
    }
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError> {
        let files = walker::walk(repo_root, opts);
        let mut out = BuildOutput { files_seen: files.len(), ..Default::default() };
        let mut parsed: Vec<ParsedFile> = Vec::new();
        for f in &files {
            let bytes = match std::fs::read(repo_root.join(&f.rel_path)) { Ok(b) => b, Err(e) => { out.limitations.push(Limitation::new("skipped_file", format!("{}: {e}", f.rel_path))); continue; } };
            let Ok(source) = String::from_utf8(bytes) else { out.limitations.push(Limitation::new("skipped_file", format!("{}: not utf-8", f.rel_path))); continue; };
            match vendor::parse(&f.rel_path, &source, &f.language) {
                Ok(r) => { parsed.push(ParsedFile { rel_path: f.rel_path.clone(), language: f.language.clone(), lines: source.lines().map(String::from).collect(), symbols: r.symbols, call_sites: r.call_sites, imports: r.imports }); out.files_parsed += 1; }
                Err(e) => out.limitations.push(Limitation::new("skipped_file", format!("{}: {e}", f.rel_path))),
            }
        }
        for pf in &parsed {
            let n_lines = pf.lines.len() as u32;
            let refs: Vec<&str> = pf.lines.iter().map(String::as_str).collect();
            out.nodes.push(Node { id: None, kind: if is_test_path(&pf.rel_path) { NodeKind::Test } else { NodeKind::File }, name: pf.rel_path.rsplit('/').next().unwrap_or(&pf.rel_path).to_string(),
                qualified_name: pf.rel_path.clone(), path: pf.rel_path.clone(), line_start: 1, line_end: n_lines.max(1), content_hash: content_hash(&refs), language: pf.language.clone(), provider_ref: None });
            for (s, qn) in pf.symbols.iter().zip(resolve::qualified_names(pf)) {
                let (ls, mut le) = (s.line_start.max(1), s.line_end.max(s.line_start));
                if le > n_lines { out.limitations.push(Limitation::new("clamped_line_end", format!("{}:{} {} → {}", pf.rel_path, s.name, le, n_lines))); le = n_lines.max(1); }
                let slice: Vec<&str> = pf.lines[(ls as usize - 1).min(pf.lines.len())..(le as usize).min(pf.lines.len())].iter().map(String::as_str).collect();
                out.nodes.push(Node { id: None, kind: kind_of(&s.kind), name: s.name.clone(), qualified_name: qn, path: pf.rel_path.clone(), line_start: ls, line_end: le,
                    content_hash: content_hash(&slice), language: pf.language.clone(), provider_ref: s.signature.clone() });
            }
        }
        out.edges_by_qn = resolve::resolve_edges(&parsed);
        Ok(out)
    }
}
```
Check the vendored `Symbol.line_start` convention: graphmind reports 1-based lines (`node.start_position().row + 1`) — verify with `grep -n "row + 1" crates/comparegraph/src/providers/reference/vendor/extractor.rs`; if it is 0-based, add 1 in `build` and say so in a comment.

- [ ] **Step 6: Run tests, clippy, commit**

Run: `cargo test -p comparegraph providers::` → all pass. `cargo clippy -p comparegraph --tests -- -D warnings` → clean (the vendored files may need `#![allow(clippy::all)]` at the top of `vendor/mod.rs` submodules — add `#[allow(clippy::all, dead_code)]` on each `pub mod` line in `vendor/mod.rs` rather than editing the taken files).
```bash
git add crates/comparegraph/src/providers crates/comparegraph/src/lib.rs docs/THIRD_PARTY_NOTICES.md
git commit -m "feat(providers): the reference engine — graphmind's parser layer taken whole (MIT), gitignore walker, edge resolution"
```

---

### Task 5: Application — add, build, check, export json

**Files:**
- Create: `crates/comparegraph/src/application/mod.rs`, `add.rs`, `build.rs`, `check.rs`, `export.rs`
- Modify: `crates/comparegraph/src/lib.rs` (add `pub mod application;`)

**Interfaces:**
- Produces:
  - `application::add::add_repo(root: &Path, clone: &Path, name: Option<&str>, as_root: bool) -> Result<RepoRow, AppError>` — `as_root` sets `role = root`; a second root is refused (unique partial index) — surface row: `head` from `git -C <clone> rev-parse --short HEAD` (None if not a git repo), `remote` from `git -C <clone> remote get-url origin`, `licence` from the first line of `LICENSE*` that contains `MIT|Apache|BSD|GPL|AGPL|MPL|Unlicense` (else None, lane stays `study`), lane `take` for MIT/Apache/BSD, `study` otherwise.
  - `application::build::build_repo(root, repo_name, provider_name, opts) -> Result<BuildReport { run: RunId, nodes: usize, edges: usize, files_parsed: usize, limitations: usize, store_hash: String, seconds: f64 }, AppError>` — refuses with `AppError::Empty("0 files parsed …")` and marks the run `failed`.
  - `application::check::check(root) -> Result<CheckReport { repos: usize, runs_done: usize, stale_locations: usize, drifted_runs: usize, schema_ok: bool, problems: Vec<String> }, AppError>`; `ok()` is `problems.is_empty()`.
  - `application::export::export_json(root, repo: Option<&str>) -> Result<serde_json::Value, AppError>` — `{ "schema": 1, "repos": [ { repo…, "run": {…}, "nodes": [...], "edges": [ {kind, source_qn, target_qn, confidence} ] } ] }`; refuses when 0 nodes.
  - `AppError` (thiserror): `Store(StoreError)`, `Provider(ProviderError)`, `NotFound(String)`, `Empty(String)`, `Io(PathBuf, io::Error)`.

- [ ] **Step 1: Write the failing tests in `tests/application.rs`** (integration, through the library)

```rust
use comparegraph::application::{add::add_repo, build::build_repo, check::check, export::export_json};
use comparegraph::providers::BuildOptions;
use std::process::Command;

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files { let f = d.path().join(p); std::fs::create_dir_all(f.parent().unwrap()).unwrap(); std::fs::write(f, c).unwrap(); }
    std::fs::write(d.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 Someone\n").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        assert!(Command::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap().success());
    }
    d
}

#[test]
fn add_reads_head_and_licence_lane() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    pass\n")]);
    let r = add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    assert_eq!(r.name, "alpha"); assert_eq!(r.lane, "take"); assert_eq!(r.licence.as_deref(), Some("MIT"));
    assert_eq!(r.head.map(|h| h.len()).unwrap_or(0), 7);
    let dup = add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap_err().to_string();
    assert!(dup.contains("alpha") && dup.contains("exists"), "{dup}");
    let r1 = add_repo(ws.path(), c.path(), Some("root1"), true).unwrap();
    assert_eq!(r1.role, "root");
    let two = add_repo(ws.path(), c.path(), Some("root2"), true).unwrap_err().to_string();
    assert!(two.contains("at most one"), "{two}");
}

#[test]
fn build_then_export_and_check_are_green_and_deterministic() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    return g()\n\ndef g():\n    return 1\n")]);
    add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    let r1 = build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    let r2 = build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    assert_eq!(r1.store_hash, r2.store_hash);
    assert_eq!(r1.nodes, 3); // file + f + g
    assert!(r1.edges >= 3);   // 2 contains + 1 calls
    let j = export_json(ws.path(), Some("alpha")).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["repos"][0]["nodes"].as_array().unwrap().len(), 3);
    assert!(j["repos"][0]["edges"].as_array().unwrap().iter().any(|e| e["kind"] == "calls" && e["source_qn"] == "src/a.py::f"));
    let rep = check(ws.path()).unwrap();
    assert!(rep.ok(), "{:?}", rep.problems);
    assert_eq!(rep.runs_done, 2);
}

#[test]
fn build_refuses_zero_files_and_marks_run_failed() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("README.txt", "nothing to parse\n")]);
    add_repo(ws.path(), c.path(), Some("empty"), false).unwrap();
    let err = build_repo(ws.path(), "empty", "reference", &BuildOptions::default()).unwrap_err().to_string();
    assert!(err.contains("0 files"), "{err}");
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    let repo = store.repo_by_name("empty").unwrap().unwrap();
    assert_eq!(store.latest_run(repo.id.unwrap()).unwrap().unwrap().status, comparegraph::domain::RunStatus::Failed);
    assert!(export_json(ws.path(), Some("empty")).is_err());
}

#[test]
fn check_marks_locations_stale_after_content_change() {
    // Plan 2 writes claims; here we insert one location row by hand to prove the stale rule on the store.
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    return 1\n")]);
    add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    let repo = store.repo_by_name("alpha").unwrap().unwrap();
    let run = store.latest_done_run(repo.id.unwrap()).unwrap().unwrap();
    let f = store.nodes_of_run(run.id.unwrap()).unwrap().into_iter().find(|n| n.qualified_name == "src/a.py::f").unwrap();
    store.conn().execute("INSERT INTO family(name, definition_path) VALUES ('t','t.md')", []).unwrap();
    store.conn().execute("INSERT INTO feature(family_id, grp, slug, definition, counts_as_has, tier) VALUES (1,'g','x','d','c','basics')", []).unwrap();
    store.conn().execute("INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, created_at) VALUES (?1,1,?2,'has','derived','test','2026-10-12T00:00:00Z')", rusqlite::params![repo.id.unwrap().0, run.id.unwrap().0]).unwrap();
    store.conn().execute("INSERT INTO location(claim_id, node_id, path, line_start, line_end, content_hash, resolved_at) VALUES (1, ?1, ?2, ?3, ?4, ?5, '2026-10-12T00:00:00Z')",
        rusqlite::params![f.id.unwrap().0, f.path, f.line_start, f.line_end, f.content_hash]).unwrap();
    drop(store);
    assert_eq!(check(ws.path()).unwrap().stale_locations, 0);
    std::fs::write(c.path().join("src/a.py"), "def f():\n    return 2\n").unwrap();
    let rep = check(ws.path()).unwrap();
    assert_eq!(rep.stale_locations, 1);
    assert!(!rep.ok());
}
```
Add `rusqlite = { version = "0.32", features = ["bundled"] }` is already a dependency; integration tests can use it via `comparegraph::store::Store::conn()` — keep `rusqlite` also under `[dev-dependencies]`? No: the test uses `rusqlite::params!`, so add `rusqlite` to `[dev-dependencies]` as well (same version).

- [ ] **Step 2: Run, expect failure** — `cargo test -p comparegraph --test application` → FAIL: module not found.

- [ ] **Step 3: Implement application/mod.rs, add.rs, build.rs, check.rs, export.rs**

`mod.rs`:
```rust
pub mod add; pub mod build; pub mod check; pub mod export;
use std::path::PathBuf;
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)] Store(#[from] crate::store::StoreError),
    #[error(transparent)] Provider(#[from] crate::providers::ProviderError),
    #[error("{0}")] NotFound(String),
    #[error("{0}")] Empty(String),
    #[error("{0}: {1}")] Io(PathBuf, std::io::Error),
}
pub type Result<T> = std::result::Result<T, AppError>;
pub(crate) fn git(clone: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(clone).args(args).output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
```
`add.rs`:
```rust
use super::{git, AppError, Result};
use crate::store::{RepoRow, Store};
use std::path::Path;

pub fn licence_of(clone: &Path) -> Option<String> {
    let dir = std::fs::read_dir(clone).ok()?;
    let mut names: Vec<_> = dir.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.to_ascii_uppercase().starts_with("LICEN")).collect();
    names.sort();
    let text = std::fs::read_to_string(clone.join(names.first()?)).ok()?;
    let head: String = text.lines().take(5).collect::<Vec<_>>().join(" ");
    for (needle, id) in [("Apache License", "Apache-2.0"), ("MIT License", "MIT"), ("BSD", "BSD"), ("GNU AFFERO", "AGPL-3.0"), ("GNU GENERAL PUBLIC", "GPL-3.0"), ("Mozilla Public", "MPL-2.0"), ("Unlicense", "Unlicense")] {
        if head.to_ascii_uppercase().contains(&needle.to_ascii_uppercase()) { return Some(id.to_string()); }
    }
    None
}
pub fn lane_for(licence: Option<&str>) -> &'static str { match licence { Some("MIT") | Some("Apache-2.0") | Some("BSD") | Some("Unlicense") => "take", _ => "study" } }

pub fn add_repo(root: &Path, clone: &Path, name: Option<&str>, as_root: bool) -> Result<RepoRow> {
    let clone = clone.canonicalize().map_err(|e| AppError::Io(clone.to_path_buf(), e))?;
    let name = name.map(String::from).unwrap_or_else(|| clone.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into()));
    let store = Store::open(root)?;
    if store.repo_by_name(&name)?.is_some() { return Err(AppError::NotFound(format!("repo '{name}' already exists in {}", store.path().display()))); }
    let mut row = RepoRow::new(&name, &clone.to_string_lossy());
    row.head = git(&clone, &["rev-parse", "--short=7", "HEAD"]);
    row.remote = git(&clone, &["remote", "get-url", "origin"]);
    row.licence = licence_of(&clone);
    row.lane = lane_for(row.licence.as_deref()).to_string();
    if as_root { row.role = "root".into(); }
    let id = store.insert_repo(&row).map_err(|e| match e { crate::store::StoreError::Sqlite(ref s) if s.to_string().contains("idx_repo_one_root") => AppError::NotFound("this workspace already has a root; a workspace has at most one (spec §2b)".into()), other => other.into() })?;
    row.id = Some(id);
    Ok(row)
}
```
`build.rs`:
```rust
use super::{AppError, Result};
use crate::domain::{Edge, Limitation, NodeId, Run, RunId, RunStatus};
use crate::providers::{by_name, BuildOptions};
use crate::store::{now_iso, Store};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildReport { pub run: RunId, pub nodes: usize, pub edges: usize, pub files_parsed: usize, pub limitations: usize, pub store_hash: String, pub seconds: f64 }

pub fn build_repo(root: &Path, repo_name: &str, provider_name: &str, opts: &BuildOptions) -> Result<BuildReport> {
    let store = Store::open(root)?;
    let repo = store.repo_by_name(repo_name)?.ok_or_else(|| AppError::NotFound(format!("no repo '{repo_name}'; run `comparegraph add` first")))?;
    let provider = by_name(provider_name).ok_or_else(|| AppError::NotFound(format!("no provider '{provider_name}'")))?;
    let status = provider.detect();
    if !status.installed { return Err(AppError::Provider(crate::providers::ProviderError::Unavailable(provider_name.into(), status.detail))); }
    let repo_id = repo.id.unwrap();
    let t = Instant::now();
    let run = store.insert_run(&Run { id: None, repo: repo_id, provider: provider.name().into(), provider_version: provider.version(), started_at: now_iso(), seconds: 0.0, status: RunStatus::Running, limitations: vec![], store_hash: None })?;
    let out = match provider.build(Path::new(&repo.path), opts) {
        Ok(o) => o,
        Err(e) => { store.finish_run(run, RunStatus::Failed, t.elapsed().as_secs_f64(), &[Limitation::new("provider_error", e.to_string())], None)?; return Err(e.into()); }
    };
    if out.files_parsed == 0 {
        let msg = format!("0 files parsed under {} ({} files seen, {} limitations)", repo.path, out.files_seen, out.limitations.len());
        store.finish_run(run, RunStatus::Failed, t.elapsed().as_secs_f64(), &[Limitation::new("empty_build", msg.clone())], None)?;
        return Err(AppError::Empty(msg));
    }
    let mut nodes = out.nodes;
    store.insert_nodes(repo_id, run, &mut nodes)?;
    let ids: HashMap<&str, NodeId> = nodes.iter().map(|n| (n.qualified_name.as_str(), n.id.unwrap())).collect();
    let edges: Vec<Edge> = out.edges_by_qn.iter().filter_map(|(k, s, t2, c)| Some(Edge { kind: *k, source: *ids.get(s.as_str())?, target: *ids.get(t2.as_str())?, confidence: *c })).collect();
    store.insert_edges(repo_id, run, &edges)?;
    let hash = store.store_hash(run)?;
    let secs = t.elapsed().as_secs_f64();
    store.finish_run(run, RunStatus::Done, secs, &out.limitations, Some(&hash))?;
    Ok(BuildReport { run, nodes: nodes.len(), edges: edges.len(), files_parsed: out.files_parsed, limitations: out.limitations.len(), store_hash: hash, seconds: secs })
}
```
`check.rs`:
```rust
use super::Result;
use crate::domain::content_hash;
use crate::store::{Store, SCHEMA_VERSION};
use rusqlite::params;
use std::path::Path;

#[derive(Debug, Default, serde::Serialize)]
pub struct CheckReport { pub repos: usize, pub runs_done: usize, pub stale_locations: usize, pub drifted_runs: usize, pub schema_ok: bool, pub problems: Vec<String> }
impl CheckReport { pub fn ok(&self) -> bool { self.problems.is_empty() } }

pub fn check(root: &Path) -> Result<CheckReport> {
    let store = Store::open(root)?;
    let mut rep = CheckReport { schema_ok: store.schema_version()? == SCHEMA_VERSION, ..Default::default() };
    if !rep.schema_ok { rep.problems.push(format!("schema version is not {SCHEMA_VERSION}")); }
    let repos = store.repos()?;
    rep.repos = repos.len();
    for r in &repos {
        let Some(run) = store.latest_done_run(r.id.unwrap())? else { continue };
        rep.runs_done += store.conn().query_row("SELECT COUNT(*) FROM run WHERE repo_id=?1 AND status='done'", params![r.id.unwrap().0], |x| x.get::<_, usize>(0))?;
        // determinism: the recorded hash must equal a recomputation over the stored rows
        let recomputed = store.store_hash(run.id.unwrap())?;
        if run.store_hash.as_deref() != Some(recomputed.as_str()) { rep.drifted_runs += 1; rep.problems.push(format!("{}: run {} store_hash drifted", r.name, run.id.unwrap())); }
        // locations: path exists, lines within file, content hash matches
        let mut st = store.conn().prepare("SELECT l.id, l.path, l.line_start, l.line_end, l.content_hash FROM location l JOIN claim c ON c.id = l.claim_id WHERE c.repo_id = ?1")?;
        let rows: Vec<(i64, String, u32, u32, String)> = st.query_map(params![r.id.unwrap().0], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?, x.get(4)?)))?.collect::<std::result::Result<_, _>>()?;
        for (id, path, ls, le, h) in rows {
            let full = Path::new(&r.path).join(&path);
            let stale = match std::fs::read_to_string(&full) {
                Ok(src) => { let lines: Vec<&str> = src.lines().collect(); if (le as usize) > lines.len() || ls == 0 || ls > le { true } else { content_hash(&lines[ls as usize - 1..le as usize]) != h } }
                Err(_) => true,
            };
            if stale {
                rep.stale_locations += 1;
                store.conn().execute("UPDATE location SET stale = 1 WHERE id = ?1", params![id])?; // the one permitted flag flip (spec §7.4)
                rep.problems.push(format!("{}: location {} {}:{}-{} is stale", r.name, id, path, ls, le));
            }
        }
    }
    Ok(rep)
}
```
`export.rs`:
```rust
use super::{AppError, Result};
use crate::store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

pub fn export_json(root: &Path, repo: Option<&str>) -> Result<Value> {
    let store = Store::open(root)?;
    let repos: Vec<_> = store.repos()?.into_iter().filter(|r| repo.map(|n| n == r.name).unwrap_or(true)).collect();
    if let Some(n) = repo { if repos.is_empty() { return Err(AppError::NotFound(format!("no repo '{n}'"))); } }
    let mut out = Vec::new();
    let mut total_nodes = 0;
    for r in repos {
        let Some(run) = store.latest_done_run(r.id.unwrap())? else { out.push(json!({ "repo": r, "run": null, "nodes": [], "edges": [] })); continue };
        let nodes = store.nodes_of_run(run.id.unwrap())?;
        let qn: HashMap<_, _> = nodes.iter().map(|n| (n.id.unwrap(), n.qualified_name.clone())).collect();
        let edges: Vec<Value> = store.edges_of_run(run.id.unwrap())?.iter().map(|e| json!({ "kind": e.kind.as_str(), "source_qn": qn[&e.source], "target_qn": qn[&e.target], "confidence": e.confidence })).collect();
        total_nodes += nodes.len();
        out.push(json!({ "repo": r, "run": run, "nodes": nodes, "edges": edges }));
    }
    if total_nodes == 0 { return Err(AppError::Empty("export has 0 nodes: build first".into())); }
    Ok(json!({ "schema": 1, "repos": out }))
}
```
- [ ] **Step 4: Run tests, clippy, commit**

Run: `cargo test -p comparegraph` → all green (unit + the 4 integration tests). `cargo clippy -p comparegraph --tests -- -D warnings` → clean.
```bash
git add crates/comparegraph/src/application crates/comparegraph/src/lib.rs crates/comparegraph/tests/application.rs crates/comparegraph/Cargo.toml
git commit -m "feat(application): add, build (a run per build, deterministic hash), check (stale locations, drift), export json"
```

---

### Task 6: CLI adapter

**Files:**
- Create: `crates/comparegraph/src/adapters/mod.rs`, `crates/comparegraph/src/adapters/cli.rs`, `crates/comparegraph/tests/cli.rs`
- Modify: `crates/comparegraph/src/main.rs`, `crates/comparegraph/src/lib.rs` (add `pub mod adapters;`)

**Interfaces:**
- Produces: binary `comparegraph` with `--root <dir>` (default `.`), subcommands `add <path> [--name n]`, `build <repo> [--provider reference]`, `check`, `export json [--repo r] [--out file]`, `doctor`, `conform <provider> [--fixtures dir]` (conform lands in Task 7; wire the subcommand to `application::conform` then). Exit codes: 0 ok, 1 refusal with reason on stderr, 2 usage. Every command prints a one-line JSON summary on stdout (`--pretty` for multi-line).

- [ ] **Step 1: Write the end-to-end test**

`tests/cli.rs`:
```rust
use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as Sys;

fn clone() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/a.ts"), "export function f() { return g(); }\nexport function g() { return 1; }\n").unwrap();
    std::fs::write(d.path().join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] { Sys::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap(); }
    d
}
fn cc(ws: &std::path::Path) -> Command { let mut c = Command::cargo_bin("comparegraph").unwrap(); c.arg("--root").arg(ws); c }

#[test]
fn add_build_check_export_through_the_binary() {
    let ws = tempfile::tempdir().unwrap(); let c = clone();
    cc(ws.path()).args(["add"]).arg(c.path()).args(["--name", "a", "--as-root"]).assert().success().stdout(predicate::str::contains("\"lane\":\"take\"")).stdout(predicate::str::contains("\"role\":\"root\""));
    cc(ws.path()).args(["build", "a"]).assert().success().stdout(predicate::str::contains("\"nodes\":3"));
    cc(ws.path()).args(["check"]).assert().success().stdout(predicate::str::contains("\"problems\":[]"));
    let out = ws.path().join("g.json");
    cc(ws.path()).args(["export", "json", "--repo", "a", "--out"]).arg(&out).assert().success();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["repos"][0]["nodes"].as_array().unwrap().len(), 3);
}
#[test]
fn refusals_exit_1_with_reason() {
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["build", "nope"]).assert().code(1).stderr(predicate::str::contains("no repo 'nope'"));
    cc(ws.path()).args(["export", "json"]).assert().code(1).stderr(predicate::str::contains("0 nodes"));
}
#[test]
fn doctor_reports_store_and_providers() {
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["doctor"]).assert().success().stdout(predicate::str::contains("\"reference\"")).stdout(predicate::str::contains("\"schema_version\":1"));
}
```
- [ ] **Step 2: Run, expect failure** — `cargo test -p comparegraph --test cli` → FAIL (no subcommands).

- [ ] **Step 3: Implement cli.rs and main.rs**

`adapters/mod.rs`: `pub mod cli;`

`cli.rs`:
```rust
use crate::application::{add::add_repo, build::build_repo, check::check, export::export_json, AppError};
use crate::providers::{by_name, BuildOptions};
use crate::store::Store;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "comparegraph", version, about = "Many clones, one taxonomy, evidence on every cell")]
pub struct Cli {
    #[arg(long, default_value = ".")] pub root: PathBuf,
    #[arg(long)] pub pretty: bool,
    #[command(subcommand)] pub cmd: Cmd,
}
#[derive(Subcommand)]
pub enum Cmd {
    /// Register a clone (read, never run)
    Add { path: PathBuf, #[arg(long)] name: Option<String>, /// register this clone as the workspace's root (spec §2b product/baseline modes)
    #[arg(long)] as_root: bool },
    /// Parse a registered clone into the graph (one run)
    Build { repo: String, #[arg(long, default_value = "reference")] provider: String, #[arg(long, default_value_t = 1_048_576)] max_file_bytes: u64, #[arg(long, default_value_t = 50_000)] max_files: usize },
    /// Verify: schema, determinism, locations
    Check,
    /// Export the graph
    Export { #[command(subcommand)] what: ExportWhat },
    /// Report the store and every provider
    Doctor,
    /// Run the golden fixtures against a provider
    Conform { provider: String, #[arg(long, default_value = "fixtures")] fixtures: PathBuf },
}
#[derive(Subcommand)]
pub enum ExportWhat { Json { #[arg(long)] repo: Option<String>, #[arg(long)] out: Option<PathBuf> } }

fn emit(v: &serde_json::Value, pretty: bool) { if pretty { println!("{}", serde_json::to_string_pretty(v).unwrap()) } else { println!("{v}") } }

pub fn run() -> i32 {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(v) => { emit(&v, cli.pretty); 0 }
        Err(e) => { eprintln!("comparegraph: {e}"); 1 }
    }
}

fn dispatch(cli: &Cli) -> Result<serde_json::Value, AppError> {
    Ok(match &cli.cmd {
        Cmd::Add { path, name, as_root } => serde_json::to_value(add_repo(&cli.root, path, name.as_deref(), *as_root)?).unwrap(),
        Cmd::Build { repo, provider, max_file_bytes, max_files } => serde_json::to_value(build_repo(&cli.root, repo, provider, &BuildOptions { max_file_bytes: *max_file_bytes, max_files: *max_files })?).unwrap(),
        Cmd::Check => { let r = check(&cli.root)?; let v = serde_json::to_value(&r).unwrap(); if !r.ok() { emit(&v, cli.pretty); return Err(AppError::Empty(format!("{} problem(s)", r.problems.len()))); } v }
        Cmd::Export { what: ExportWhat::Json { repo, out } } => {
            let v = export_json(&cli.root, repo.as_deref())?;
            if let Some(p) = out { std::fs::write(p, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| AppError::Io(p.clone(), e))?; serde_json::json!({ "written": p, "repos": v["repos"].as_array().map(|a| a.len()).unwrap_or(0) }) } else { v }
        }
        Cmd::Doctor => {
            let store = Store::open(&cli.root)?;
            let providers: Vec<_> = ["reference"].iter().map(|n| { let p = by_name(n).unwrap(); serde_json::json!({ "name": n, "status": p.detect(), "capabilities": p.capabilities() }) }).collect();
            serde_json::json!({ "store": store.path(), "schema_version": store.schema_version()?, "repos": store.repos()?.len(), "providers": providers })
        }
        Cmd::Conform { provider, fixtures } => serde_json::to_value(crate::application::conform::conform(provider, fixtures)?).unwrap(),
    })
}
```
Until Task 7 exists, stub `application::conform::conform` as `pub fn conform(_: &str, _: &std::path::Path) -> super::Result<serde_json::Value> { Err(super::AppError::NotFound("conform lands in Task 7".into())) }` so the crate compiles; Task 7 replaces it.

`main.rs`:
```rust
fn main() { std::process::exit(comparegraph::adapters::cli::run()); }
```
- [ ] **Step 4: Run, clippy, commit**

Run: `cargo test -p comparegraph --test cli` → 3 passed; `cargo clippy -p comparegraph --tests -- -D warnings` → clean.
```bash
git add crates/comparegraph/src/adapters crates/comparegraph/src/main.rs crates/comparegraph/src/lib.rs crates/comparegraph/src/application/mod.rs crates/comparegraph/src/application/conform.rs crates/comparegraph/tests/cli.rs
git commit -m "feat(cli): add, build, check, export json, doctor, conform (stub) with JSON output and exit codes"
```

---

### Task 7: Conformance fixtures and `conform`

**Files:**
- Create: `fixtures/typescript/src/{index.ts, util.ts}`, `fixtures/typescript/expected.json`, `fixtures/python/pkg/{__init__.py, core.py, cli.py}`, `fixtures/python/expected.json`, `crates/comparegraph/src/application/conform.rs` (replace the stub)
- Test: `crates/comparegraph/tests/conform.rs`

**Interfaces:**
- Produces: `application::conform::conform(provider: &str, fixtures_dir: &Path) -> Result<ConformReport { provider, fixtures: Vec<FixtureResult { name, nodes: PR, edges: PR, passed: bool }>, passed: bool }>` with `PR { expected: usize, found: usize, matched: usize, precision: f64, recall: f64 }`; floors: nodes recall ≥ 0.95 and precision ≥ 0.95, edges recall ≥ 0.85 and precision ≥ 0.85.
- `expected.json` format: `{ "nodes": ["src/index.ts::main", "src/util.ts::add", …] (qualified names of non-file nodes), "edges": ["calls src/index.ts::main src/util.ts::add", "imports src/index.ts src/util.ts", …] }`.

- [ ] **Step 1: Write the fixtures by hand and verify them by eye**

`fixtures/typescript/src/util.ts`:
```ts
export function add(a: number, b: number): number { return a + b; }
export function mul(a: number, b: number): number { return a * b; }
export class Counter {
  private n = 0;
  inc(): number { this.n = add(this.n, 1); return this.n; }
}
```
`fixtures/typescript/src/index.ts`:
```ts
import { add, Counter } from "./util";
export function main(): number {
  const c = new Counter();
  c.inc();
  return add(1, 2);
}
```
`fixtures/typescript/expected.json`:
```json
{ "nodes": ["src/util.ts::add", "src/util.ts::mul", "src/util.ts::Counter", "src/util.ts::inc", "src/index.ts::main"],
  "edges": ["contains src/util.ts src/util.ts::add", "contains src/util.ts src/util.ts::mul", "contains src/util.ts src/util.ts::Counter", "contains src/util.ts src/util.ts::inc",
            "contains src/index.ts src/index.ts::main", "imports src/index.ts src/util.ts", "calls src/index.ts::main src/util.ts::add", "calls src/util.ts::inc src/util.ts::add"] }
```
`fixtures/python/pkg/__init__.py`: empty file. `fixtures/python/pkg/core.py`:
```python
def add(a, b):
    return a + b

class Store:
    def __init__(self):
        self.items = []

    def push(self, x):
        self.items.append(x)
        return len(self.items)
```
`fixtures/python/pkg/cli.py`:
```python
from .core import add, Store

def main():
    s = Store()
    s.push(add(1, 2))
    return s
```
`fixtures/python/expected.json`:
```json
{ "nodes": ["pkg/core.py::add", "pkg/core.py::Store", "pkg/core.py::__init__", "pkg/core.py::push", "pkg/cli.py::main"],
  "edges": ["contains pkg/core.py pkg/core.py::add", "contains pkg/core.py pkg/core.py::Store", "contains pkg/core.py pkg/core.py::__init__", "contains pkg/core.py pkg/core.py::push",
            "contains pkg/cli.py pkg/cli.py::main", "imports pkg/cli.py pkg/core.py", "calls pkg/cli.py::main pkg/core.py::add"] }
```
If the taken Python extractor names methods `Store.push` rather than `push`, write the fixture to what the extractor produces ONLY if that naming is consistent and documented in a comment in `expected.json`'s sibling `NOTES.md`; never lower a floor to pass.

- [ ] **Step 2: Write the test**

`tests/conform.rs`:
```rust
use comparegraph::application::conform::conform;
#[test]
fn reference_provider_passes_the_two_fixtures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let rep = conform("reference", &root).unwrap();
    for f in &rep.fixtures { eprintln!("{} nodes p={:.2} r={:.2} edges p={:.2} r={:.2}", f.name, f.nodes.precision, f.nodes.recall, f.edges.precision, f.edges.recall); }
    assert!(rep.passed, "{:?}", rep);
}
```
- [ ] **Step 3: Run, expect failure** (stub) — `cargo test -p comparegraph --test conform` → FAIL "conform lands in Task 7".

- [ ] **Step 4: Implement conform.rs**

```rust
use super::{AppError, Result};
use crate::domain::NodeKind;
use crate::providers::{by_name, BuildOptions};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PR { pub expected: usize, pub found: usize, pub matched: usize, pub precision: f64, pub recall: f64 }
#[derive(Debug, Clone, serde::Serialize)]
pub struct FixtureResult { pub name: String, pub nodes: PR, pub edges: PR, pub passed: bool }
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConformReport { pub provider: String, pub fixtures: Vec<FixtureResult>, pub passed: bool }

fn pr(expected: &HashSet<String>, found: &HashSet<String>) -> PR {
    let matched = expected.intersection(found).count();
    let precision = if found.is_empty() { 0.0 } else { matched as f64 / found.len() as f64 };
    let recall = if expected.is_empty() { 0.0 } else { matched as f64 / expected.len() as f64 };
    PR { expected: expected.len(), found: found.len(), matched, precision, recall }
}

pub fn conform(provider_name: &str, fixtures_dir: &Path) -> Result<ConformReport> {
    let provider = by_name(provider_name).ok_or_else(|| AppError::NotFound(format!("no provider '{provider_name}'")))?;
    let mut dirs: Vec<_> = std::fs::read_dir(fixtures_dir).map_err(|e| AppError::Io(fixtures_dir.to_path_buf(), e))?.flatten().filter(|e| e.path().join("expected.json").exists()).collect();
    dirs.sort_by_key(|e| e.file_name());
    if dirs.is_empty() { return Err(AppError::Empty(format!("no fixtures with expected.json under {}", fixtures_dir.display()))); }
    let mut fixtures = Vec::new();
    for d in dirs {
        let name = d.file_name().to_string_lossy().to_string();
        let expected: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("expected.json")).map_err(|e| AppError::Io(d.path(), e))?).map_err(|e| AppError::Empty(format!("{name}/expected.json: {e}")))?;
        let exp_nodes: HashSet<String> = expected["nodes"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let exp_edges: HashSet<String> = expected["edges"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let out = provider.build(&d.path(), &BuildOptions::default())?;
        let found_nodes: HashSet<String> = out.nodes.iter().filter(|n| n.kind != NodeKind::File && n.kind != NodeKind::Test).map(|n| n.qualified_name.clone()).collect();
        let found_edges: HashSet<String> = out.edges_by_qn.iter().map(|(k, s, t, _)| format!("{} {} {}", k.as_str(), s, t)).collect();
        let (n, e) = (pr(&exp_nodes, &found_nodes), pr(&exp_edges, &found_edges));
        let passed = n.precision >= 0.95 && n.recall >= 0.95 && e.precision >= 0.85 && e.recall >= 0.85;
        fixtures.push(FixtureResult { name, nodes: n, edges: e, passed });
    }
    let passed = fixtures.iter().all(|f| f.passed);
    Ok(ConformReport { provider: provider_name.into(), fixtures, passed })
}
```
- [ ] **Step 5: Run; fix the FIXTURE EXPECTATIONS only where the extractor's naming is legitimately different (document in `fixtures/<lang>/NOTES.md`); never the floors**

Run: `cargo test -p comparegraph --test conform -- --nocapture` → prints the four numbers per fixture, then PASS. Also through the binary: `cargo run -p comparegraph -- conform reference --fixtures fixtures --pretty`.

- [ ] **Step 6: Clippy, commit**
```bash
git add fixtures crates/comparegraph/src/application/conform.rs crates/comparegraph/tests/conform.rs
git commit -m "feat(conform): golden fixtures for TypeScript and Python; precision/recall floors per kind"
```

---

### Task 8: Proof on a real clone, README, and the plan's exit

**Files:**
- Create: `README.md`, `docs/reports/2026-10-XX-plan-1-proof.md`
- Test: `crates/comparegraph/tests/real_clone.rs` (`#[ignore]`, run by hand)

- [ ] **Step 1: Write the ignored real-clone test**

```rust
// Run by hand: COMPAREGRAPH_CLONE=/abs/path/to/a/clone cargo test -p comparegraph --test real_clone -- --ignored --nocapture
use comparegraph::application::{add::add_repo, build::build_repo, check::check, export::export_json};
use comparegraph::providers::BuildOptions;
#[test] #[ignore]
fn builds_a_real_clone_deterministically() {
    let Some(clone) = std::env::var_os("COMPAREGRAPH_CLONE") else { eprintln!("set COMPAREGRAPH_CLONE"); return };
    let ws = tempfile::tempdir().unwrap();
    add_repo(ws.path(), std::path::Path::new(&clone), Some("real"), false).unwrap();
    let a = build_repo(ws.path(), "real", "reference", &BuildOptions::default()).unwrap();
    let b = build_repo(ws.path(), "real", "reference", &BuildOptions::default()).unwrap();
    eprintln!("files_parsed={} nodes={} edges={} limitations={} seconds={:.1}", a.files_parsed, a.nodes, a.edges, a.limitations, a.seconds);
    assert_eq!(a.store_hash, b.store_hash);
    assert!(a.nodes > 100);
    assert!(check(ws.path()).unwrap().ok());
    let j = export_json(ws.path(), Some("real")).unwrap();
    assert_eq!(j["repos"][0]["nodes"].as_array().unwrap().len(), a.nodes);
}
```
- [ ] **Step 2: Run it on three study clones and record the numbers**

Run for three real clones outside this repository (one Rust, two Python):
`COMPAREGRAPH_CLONE=/abs/path/to/<clone> cargo test -p comparegraph --test real_clone -- --ignored --nocapture`
Write `docs/reports/<today>-plan-1-proof.md` with front matter (`title`, `date`, `valid_until` (+90 days), `lane: plan-1`, `verdict`, `status: decided`) and a table: clone, head, files parsed, nodes, edges, limitations by code, seconds, hash equal on rerun (yes/no), check ok (yes/no).

- [ ] **Step 3: README**

`README.md` (≤ 60 lines): what CompareGraph is (one paragraph from spec §1), install from source (`cargo install --path crates/comparegraph`), the five commands with one example each, the schema v1 pointer (spec §4), "taken from others" pointer (THIRD_PARTY_NOTICES), licence Apache-2.0.

- [ ] **Step 4: Final gate and commit**

Run: `cargo test -p comparegraph` (all, ignored excluded) → green; `cargo clippy -p comparegraph --tests -- -D warnings` → clean; `cargo build --release -p comparegraph` → binary at `target/release/comparegraph`; `./target/release/comparegraph --root /tmp/cc-demo doctor --pretty` prints the store and the provider.
```bash
git add README.md docs/reports crates/comparegraph/tests/real_clone.rs
git commit -m "docs: README and the plan-1 proof on three real clones"
```

---

## Self-review (done at writing time)
- **Spec coverage, Plan 1 scope:** §3 rings ✔ (Tasks 1–6), §4 schema v1 ✔ (Task 3 — every table created now so Plans 2–4 add no migration), §5 evidence types ✔ (Task 2; claims written in Plan 2), §6 provider trait + reference ✔ (Task 4; crg adapter → Plan 4), §7.1 conformance ✔ (Task 7), §7.3 determinism ✔ (Tasks 3, 5), §7.4 citations ✔ (Task 5 check), §7.5 doctor ✔ (Task 6, providers + store; agent clients → Plan 5), §7.6 no silent empties ✔ (Tasks 5, 6, 7), §9 verbs add/build/check/export json/conform/doctor ✔; family/relate/map/claim/search/export vault → Plan 2; communities → Plan 3; MCP + catalog → Plan 4; method/ + npm shell + setup → Plan 5.
- **Placeholders:** none; every step has its code. The one deliberate deferral (Task 6's `conform` stub) is replaced in Task 7 of the same plan.
- **Type consistency:** `BuildOutput.edges_by_qn: Vec<(EdgeKind, String, String, f64)>` is produced by Task 4 and consumed by Tasks 5 and 7; `Store::latest_done_run` used by Tasks 5 and 6 is defined in Task 3; `now_iso` in Task 3 used by Task 5; `AppError::Empty` used by Tasks 5, 6, 7.
- **Review Focus:** each of the five has its test in the owning task (Task 4 ×3, Task 3 ×1, Task 5 ×2).
