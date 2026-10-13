#!/usr/bin/env bash
# map-score.sh — score `map`, `relate` and the absent check against the mapping gold (fixtures/gold-map/<clone>/features.jsonl).
#
# usage: map-score.sh <clone-copy-dir> <features.jsonl> --family <name> --pack <rows.md> [--binary <comparegraph>] [--workspace <dir>]
#   --pack       the family pack file (its directory is given to `family add`)
#   --binary     the comparegraph binary (default: comparegraph on PATH)
#   --workspace  reuse this workspace (built if the clone has no finished run in it) instead of a scratch one
# The clone copy is read-only here. `absent-check` is used when the binary has it (lane ABS); otherwise the absent part is
# skipped with a note on stderr and reported as nulls.
# Output: one JSON object on stdout {clone, family, map:{p5,r5,n,unresolved,...}, relate:{band,expected,ok}, absent:{precision,recall,n,...}},
# one table row on stderr. The rules live in scripts/lib/goldmap.py.
set -euo pipefail
BIN=comparegraph; WS=""; FAMILY=""; PACK=""
POS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BIN=$2; shift 2;;
    --workspace) WS=$2; shift 2;;
    --family) FAMILY=$2; shift 2;;
    --pack) PACK=$2; shift 2;;
    --*) echo "unknown option $1" >&2; exit 2;;
    *) POS+=("$1"); shift;;
  esac
done
[ "${#POS[@]}" -eq 2 ] && [ -n "$FAMILY" ] && [ -n "$PACK" ] || { sed -n 2,6p "$0" >&2; exit 2; }
SRC=$(readlink -f "${POS[0]}"); GOLD=$(readlink -f "${POS[1]}"); PACKDIR=$(dirname "$(readlink -f "$PACK")")
LIB=$(cd "$(dirname "$0")/lib" && pwd)
NAME=$(basename "$SRC")
if [ -z "$WS" ]; then WS=$(mktemp -d); trap 'rm -rf "$WS"' EXIT; fi
mkdir -p "$WS"
DB="$WS/.comparegraph/comparegraph.db"
if [ -f "$DB" ]; then
  REG=$(python3 -I -c "
import sqlite3,sys
r=sqlite3.connect(sys.argv[1]).execute('select path from repo where name=?',(sys.argv[2],)).fetchone(); print(r[0] if r else '')" "$DB" "$NAME")
  if [ -n "$REG" ] && [ "$REG" != "$SRC" ]; then echo "map-score: workspace repo '$NAME' is registered at $REG, not $SRC; use a fresh --workspace" >&2; exit 2; fi
fi
if ! [ -f "$DB" ] || ! python3 -I -c "
import sqlite3,sys
c=sqlite3.connect(sys.argv[1]); sys.exit(0 if c.execute(\"select 1 from run r join repo p on p.id=r.repo_id where p.name=? and r.provider='reference' and r.status='done'\",(sys.argv[2],)).fetchone() else 1)" "$DB" "$NAME" 2>/dev/null; then
  if ! nice -n 10 "$BIN" --root "$WS" add "$SRC" --name "$NAME" >/dev/null 2>"$WS/.add.err"; then
    grep -q "already exists" "$WS/.add.err" || { cat "$WS/.add.err" >&2; exit 2; }
  fi
  nice -n 10 "$BIN" --root "$WS" --allow-unverified build "$NAME" --provider reference >/dev/null
fi
nice -n 10 "$BIN" --root "$WS" family add "$PACKDIR" >/dev/null
nice -n 10 "$BIN" --root "$WS" map "$NAME" "$FAMILY" --limit 5 > "$WS/.map.json"
nice -n 10 "$BIN" --root "$WS" relate --repo "$NAME" --family "$FAMILY" > "$WS/.relate.json"
HAS_ABSENT=0
if "$BIN" --help 2>&1 | grep -q "absent-check"; then HAS_ABSENT=1; else echo "map-score: $BIN has no absent-check; absent part skipped" >&2; fi
FEATS=$(python3 -I -c "
import json,sys
print('\n'.join(sorted({json.loads(l)['feature_id'] for l in open(sys.argv[1]) if l.strip()})))" "$GOLD")
: > "$WS/.absent.jsonl"
if [ "$HAS_ABSENT" = 1 ]; then
  while IFS= read -r F; do
    [ -n "$F" ] || continue
    printf '%s\t' "$F" >> "$WS/.absent.jsonl"
    nice -n 10 "$BIN" --root "$WS" absent-check "$NAME" "$FAMILY" "$F" >> "$WS/.absent.jsonl" 2>/dev/null || echo '{"ok":false}' >> "$WS/.absent.jsonl"
  done <<< "$FEATS"
fi
python3 -I - "$LIB" "$DB" "$NAME" "$GOLD" "$FAMILY" "$WS" "$HAS_ABSENT" <<'PY'
import json, sqlite3, sys
lib, db, name, gold, family, ws, has_absent = sys.argv[1:8]
sys.path.insert(0, lib)
import goldmap as G
cg = sqlite3.connect(db)
rid = cg.execute("select r.id from run r join repo p on p.id=r.repo_id where p.name=? and r.provider='reference' and r.status='done' order by r.id desc limit 1", (name,)).fetchone()[0]
graph = G.load_graph(cg, rid)
rows = G.read_jsonl(gold)
mapped = G.load_map(open(ws + "/.map.json").read())
rel = json.load(open(ws + "/.relate.json"))["data"]["items"]
band = next((x["band"] for x in rel if x["repo"] == name and x["family"] == family), "unrelated")
out = {"clone": name, "family": family, "map": G.score_map(rows, mapped, graph), "relate": G.score_relate(rows, band)}
if has_absent == "1":
    checks = {}
    for line in open(ws + "/.absent.jsonl"):
        f, _, js = line.rstrip("\n").partition("\t")
        try:
            d = json.loads(js).get("data") or {}
        except ValueError:
            continue
        checks[f] = bool(d.get("absent")) or d.get("verdict") == "absent"
    out["absent"] = G.score_absent(rows, checks)
else:
    out["absent"] = {"precision": None, "recall": None, "n": 0}
print(json.dumps(out))
m, a = out["map"], out["absent"]
print("%-20s %-18s map n %3d unres %2d P5 %s R5 %s  relate %s (want %s)  absent P %s R %s" % (
    name, family, m["n"], m["unresolved"], m["p5"], m["r5"], out["relate"]["band"], out["relate"]["expected"], a["precision"], a["recall"]), file=sys.stderr)
PY
