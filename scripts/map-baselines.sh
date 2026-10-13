#!/usr/bin/env bash
# map-baselines.sh — feature-mapping baselines (plain FTS, codegraph symbol search) next to our `map`, one scorer, one run.
#
# usage: map-baselines.sh <scratch-dir> <comparegraph-binary> <name>=<clone-copy-dir> [...]
#   Clone copies are read-only here. Per clone: a workspace <scratch>/ws-<name> (built if absent), `map-score.sh` for our row,
#   then scripts/lib/map_baselines.py for FTS-OR, FTS-AND (the `search` verb), codegraph symbol search and pattern search on the same
#   terms (the `terms` field of `map`), scored with scripts/lib/goldmap.py against fixtures/gold-map/<name>/features.jsonl.
#   codegraph-mcp runs with HOME=<scratch>/home/<name>-<pid> (fresh per run); the real ~/.codegraph is never touched
#   (its mtime is printed at start and end as the proof).
# Output: <scratch>/out/<name>.json per clone, the table on stdout (Markdown), load average at start and end on stderr.
set -euo pipefail
[ $# -ge 3 ] || { sed -n 2,10p "$0" >&2; exit 2; }
SCR=$(readlink -f "$1"); BIN=$(readlink -f "$2"); shift 2
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$HERE")
FAMILY=code-graph-tools; PACK="$ROOT/families/code-graph-tools/rows.md"
mkdir -p "$SCR/out" "$SCR/home"
echo "load $(cut -d' ' -f1-3 /proc/loadavg) at start; HEAD $(git -C "$ROOT" rev-parse --short HEAD)" >&2
echo "real ~/.codegraph at start: $(stat -c '%y' "$HOME/.codegraph" 2>/dev/null || echo absent)" >&2
for spec in "$@"; do
  NAME=${spec%%=*}; SRC=$(readlink -f "${spec#*=}"); GOLD="$ROOT/fixtures/gold-map/$NAME/features.jsonl"
  bash "$HERE/map-score.sh" "$SRC" "$GOLD" --family "$FAMILY" --pack "$PACK" --binary "$BIN" --workspace "$SCR/ws-$NAME" > "$SCR/out/$NAME.ours.json" 2>/dev/null
  python3 -I "$HERE/lib/map_baselines.py" "$NAME" "$SRC" "$GOLD" "$SCR/ws-$NAME" "$FAMILY" "$BIN" "$SCR/home/$NAME-$$" "$SCR/out/$NAME.json" 2>&1 | tail -1 >&2
done
python3 -I - "$HERE/lib" "$SCR/out" "$@" <<'PY'
import json, sys
sys.path.insert(0, sys.argv[1])
import goldmap as G
out = sys.argv[2]
names = [s.split("=")[0] for s in sys.argv[3:]]
M = [("fts_or", "FTS-OR"), ("fts_and", "FTS-AND (search verb)"), ("cg_symbol", "codegraph symbol_search"), ("cg_pattern", "codegraph search_by_pattern"), ("ours", "ours (map)")]
res = {n: json.load(open("%s/%s.json" % (out, n))) for n in names}
for n in names:   # the scorer inside map_baselines.py must give map-score.sh's numbers for ours
    o = json.load(open("%s/%s.ours.json" % (out, n)))["map"]
    assert (o["cands"], o["cands_ok"], o["golds"], o["golds_hit"]) == tuple(res[n]["methods"]["ours"][k] for k in ("cands", "cands_ok", "golds", "golds_hit")), n
def f(x):
    return "-" if x is None else "%.3f" % x
print("| clone | n | " + " | ".join(l + " P@5 / R@5" for _, l in M) + " |")
print("|---|---|" + "---|" * len(M))
for n in names:
    r = res[n]["methods"]
    print("| %s | %d | " % (n, r["ours"]["n"]) + " | ".join("%s / %s" % (f(r[m]["p5"]), f(r[m]["r5"])) for m, _ in M) + " |")
pool = {m: G.pool([res[n]["methods"][m] for n in names]) for m, _ in M}
print("| pooled | %d | " % pool["ours"]["n"] + " | ".join("%s / %s" % (f(pool[m]["p5"]), f(pool[m]["r5"])) for m, _ in M) + " |")
print()
print("Raw counts pooled (cands_ok/cands, golds_hit/golds): " + "; ".join("%s %d/%d, %d/%d" % (l, pool[m]["cands_ok"], pool[m]["cands"], pool[m]["golds_hit"], pool[m]["golds"]) for m, l in M))
print()
for m, l in M[:-1]:
    beats = []
    for n in names:
        for fid, d in res[n]["per_feature"].items():
            if d["ours"]["n"] == 0:
                continue
            b, o = d[m], d["ours"]
            if (b["golds_hit"], b["cands_ok"]) > (o["golds_hit"], o["cands_ok"]):
                beats.append("%s:%s (%d/%d vs %d/%d)" % (n, fid, b["golds_hit"], b["golds"], o["golds_hit"], o["golds"]))
    print("%s beats ours on %d features: %s" % (l, len(beats), "; ".join(beats) or "none"))
print()
print("FTS-OR top-5 identical to ours on " + ", ".join("%s %d/%d" % (n, res[n]["same_as_ours"]["fts_or_identical_features"], res[n]["same_as_ours"]["features"]) for n in names) + " features")
PY
echo "real ~/.codegraph at end: $(stat -c '%y' "$HOME/.codegraph" 2>/dev/null || echo absent)" >&2
echo "load $(cut -d' ' -f1-3 /proc/loadavg) at end" >&2
