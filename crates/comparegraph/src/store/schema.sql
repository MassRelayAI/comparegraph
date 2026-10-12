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
CREATE TABLE IF NOT EXISTS family (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, definition_path TEXT NOT NULL, description TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS feature (
  id INTEGER PRIMARY KEY, family_id INTEGER NOT NULL REFERENCES family(id), grp TEXT NOT NULL, slug TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
  definition TEXT NOT NULL, counts_as_has TEXT NOT NULL, tier TEXT NOT NULL, weight INTEGER NOT NULL DEFAULT 1, UNIQUE(family_id, slug));
CREATE TABLE IF NOT EXISTS relatedness (
  repo_id INTEGER NOT NULL REFERENCES repo(id), family_id INTEGER NOT NULL REFERENCES family(id), run_id INTEGER NOT NULL REFERENCES run(id),
  score REAL NOT NULL, band TEXT NOT NULL, basis_json TEXT NOT NULL DEFAULT '[]', pack_hash TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (repo_id, family_id, run_id, pack_hash));
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
CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(name, qualified_name, path, tokenize='unicode61 remove_diacritics 2 tokenchars ''_''');

-- Append-only, enforced by the database (spec §7.4): `run`, `feature`, `family` and `location.stale` stay writable.
CREATE TRIGGER IF NOT EXISTS claim_no_update BEFORE UPDATE ON claim BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS claim_no_delete BEFORE DELETE ON claim BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS decision_no_update BEFORE UPDATE ON decision BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS decision_no_delete BEFORE DELETE ON decision BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS relatedness_no_update BEFORE UPDATE ON relatedness BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS relatedness_no_delete BEFORE DELETE ON relatedness BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS location_no_delete BEFORE DELETE ON location BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS location_no_update BEFORE UPDATE OF path, line_start, line_end, content_hash, claim_id, node_id, resolved_at ON location BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS node_no_update BEFORE UPDATE ON node BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS node_no_delete BEFORE DELETE ON node BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS edge_no_update BEFORE UPDATE ON edge BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS edge_no_delete BEFORE DELETE ON edge BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS community_no_update BEFORE UPDATE ON community BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS community_no_delete BEFORE DELETE ON community BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS community_member_no_update BEFORE UPDATE ON community_member BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER IF NOT EXISTS community_member_no_delete BEFORE DELETE ON community_member BEGIN SELECT RAISE(ABORT, 'append-only'); END;
-- per-file parse cache (incremental builds): prunable, so no append-only triggers.
CREATE TABLE IF NOT EXISTS parse_cache (
  file_hash TEXT NOT NULL, language TEXT NOT NULL, provider_version TEXT NOT NULL, payload BLOB NOT NULL,
  PRIMARY KEY (file_hash, language, provider_version));
