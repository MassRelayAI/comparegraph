#!/usr/bin/env bash
# gold-score.sh — score CompareGraph's call edges against the committed gold labels (fixtures/gold/<project>/calls.jsonl).
#
# usage: gold-score.sh <project-copy-dir> <gold-jsonl> [--engine ours] [--binary <comparegraph>] [--workspace <dir>]
#   --binary     the comparegraph binary (default: comparegraph on PATH)
#   --workspace  reuse this workspace (built if the project has no finished run in it) instead of a scratch one
#   --engine     `ours` (the reference provider, default) or `crg` (code-review-graph through providers/crg, scored through our
#                schema by the same scorer; needs code-review-graph on PATH or COMPAREGRAPH_CRG_BIN)
# The project copy is read-only here: the engine reads it, nothing is written inside it.
# Output: one JSON object on stdout {project, engine, sites, repo_target_sites, answered, correct, precision, recall, misses, false},
# one table row on stderr. The matching rule lives in scripts/lib/gold.py, shared with compare-engines.sh.
set -euo pipefail
ENGINE=ours; BIN=comparegraph; WS=""
POS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --engine) ENGINE=$2; shift 2;;
    --binary) BIN=$2; shift 2;;
    --workspace) WS=$2; shift 2;;
    --*) echo "unknown option $1" >&2; exit 2;;
    *) POS+=("$1"); shift;;
  esac
done
[ "${#POS[@]}" -eq 2 ] || { sed -n 2,9p "$0" >&2; exit 2; }
case "$ENGINE" in ours) PROVIDER=reference;; crg) PROVIDER=crg;; *) echo "engine $ENGINE is not implemented (ours|crg)" >&2; exit 2;; esac
SRC=$(readlink -f "${POS[0]}"); GOLD=$(readlink -f "${POS[1]}")
LIB=$(cd "$(dirname "$0")/lib" && pwd)
NAME=$(basename "$SRC")
if [ -z "$WS" ]; then WS=$(mktemp -d); trap 'rm -rf "$WS"' EXIT; fi
mkdir -p "$WS"
DB="$WS/.comparegraph/comparegraph.db"
# a reused workspace must hold this project at the SAME path; a namesake from another checkout would be scored as if it were this one
if [ -f "$DB" ]; then
  REG=$(python3 -I -c "
import sqlite3,sys
r=sqlite3.connect(sys.argv[1]).execute('select path from repo where name=?',(sys.argv[2],)).fetchone(); print(r[0] if r else '')" "$DB" "$NAME")
  if [ -n "$REG" ] && [ "$REG" != "$SRC" ]; then echo "gold-score: workspace repo '$NAME' is registered at $REG, not $SRC; use a fresh --workspace" >&2; exit 2; fi
fi
if ! [ -f "$DB" ] || ! python3 -I -c "
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]); sys.exit(0 if c.execute(\"select 1 from run r join repo p on p.id=r.repo_id where p.name=? and r.provider=? and r.status='done'\",(sys.argv[2],sys.argv[3])).fetchone() else 1)" "$DB" "$NAME" "$PROVIDER" 2>/dev/null; then
  if [ "$ENGINE" = crg ]; then
    printf '[providers.crg]\ncommand = "%s"\n' "$(cd "$(dirname "$0")/.." && pwd)/providers/crg/comparegraph-provider-crg" > "$WS/comparegraph.toml"
  fi
  # the repo may already be registered by the other engine's run at the same path (checked above); any other add error is fatal
  if ! nice -n 10 "$BIN" --root "$WS" add "$SRC" --name "$NAME" >/dev/null 2>"$WS/.add.err"; then
    grep -q "already exists" "$WS/.add.err" || { cat "$WS/.add.err" >&2; exit 2; }
  fi
  nice -n 10 "$BIN" --root "$WS" --allow-unverified build "$NAME" --provider "$PROVIDER" >/dev/null
fi
python3 -I - "$LIB" "$DB" "$NAME" "$GOLD" "$ENGINE" "$PROVIDER" <<'PY'
import json, sqlite3, sys
lib, db, name, gold, engine, provider = sys.argv[1:7]
sys.path.insert(0, lib)
import gold as G
cg = sqlite3.connect(db)
rid = cg.execute("select r.id from run r join repo p on p.id=r.repo_id where p.name=? and r.provider=? and r.status='done' order by r.id desc limit 1", (name, provider)).fetchone()[0]
nodes, spans, calls = G.load_ours(cg, rid)
rows = G.read_jsonl(gold)
answers = {r["id"]: G.ours_answer(nodes, spans, calls, r["path"], r["line"], r["callee"])[1] for r in rows}
s = G.score(rows, answers)
out = {"project": name, "engine": engine, "provider": provider, **s}
print(json.dumps(out))
p, r = s["precision"], s["recall"]
print("%-20s %-5s sites %4d repo %4d answered %4d correct %4d  P %s  R %s" % (name, engine, s["sites"], s["repo_target_sites"], s["answered"], s["correct"], p, r), file=sys.stderr)
PY
