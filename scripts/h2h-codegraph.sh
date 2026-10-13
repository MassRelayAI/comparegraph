#!/usr/bin/env bash
# h2h-codegraph.sh — score codegraph-mcp (Apache-2.0, on PATH) on the committed gold call labels, next to our own engine.
#
# usage: h2h-codegraph.sh <scratch-dir> <comparegraph-binary> <name>=<project-copy-dir> [...]
#   Each project copy must be the tree at the head pinned in fixtures/gold/README.md (read-only here; copy it first).
#   codegraph-mcp runs with HOME=<scratch>/home/<name> (fresh per project) so its store and socket stay in the scratch dir.
#   For each project: our engine is built timed (add + build, fresh workspace) and scored with gold-score.sh; codegraph indexes
#   with --graph-only and answers one codegraph_get_callees call per labelled site over MCP stdio (scripts/lib/h2h_codegraph.py).
# Output: one JSON object per project on stdout (the gold-score.sh shape plus index time, node/edge counts, per-site disagreement
# with ours) and a table on stderr. Files: <scratch>/out/<name>.{ours,cg}.json. Load average is printed at start and end.
set -euo pipefail
[ $# -ge 3 ] || { sed -n 2,9p "$0" >&2; exit 2; }
SCR=$(readlink -f "$1"); BIN=$(readlink -f "$2"); shift 2
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$(dirname "$HERE")
mkdir -p "$SCR/out" "$SCR/home"
echo "load $(cut -d' ' -f1-3 /proc/loadavg) at start" >&2
for spec in "$@"; do
  NAME=${spec%%=*}; SRC=$(readlink -f "${spec#*=}"); GOLD="$ROOT/fixtures/gold/$NAME/calls.jsonl"
  WS="$SCR/ws-$NAME"; rm -rf "$WS"; mkdir -p "$WS"
  nice -n 10 "$BIN" --root "$WS" add "$SRC" --name "$NAME" >/dev/null
  T0=$(date +%s.%N)
  nice -n 10 "$BIN" --root "$WS" --allow-unverified build "$NAME" --provider reference >/dev/null
  T1=$(date +%s.%N)
  OURS_S=$(echo "$T1 - $T0" | bc)
  bash "$HERE/gold-score.sh" "$SRC" "$GOLD" --binary "$BIN" --workspace "$WS" > "$SCR/out/$NAME.ours.json" 2>/dev/null
  CGH="$SCR/home/$NAME"; rm -rf "$CGH"
  python3 -I "$HERE/lib/h2h_codegraph.py" "$NAME" "$SRC" "$GOLD" "$CGH" --ours "$SCR/out/$NAME.ours.json" > "$SCR/out/$NAME.cg.json"
  python3 -I -c "
import json,sys
o=json.load(open(sys.argv[1])); c=json.load(open(sys.argv[2])); ours_s=float(sys.argv[3])
c['ours_build_s']=round(ours_s,2); c['index_ratio_ours_to_codegraph']=round(ours_s/c['scan_to_ready_s'],2) if c['scan_to_ready_s'] else None; c['index_ratio_incl_model_load']=round(ours_s/c['index_s'],2) if c['index_s'] else None
json.dump(c,open(sys.argv[2],'w')); print(json.dumps(c))
print('%-18s ours P %s R %s | codegraph P %s R %s | build ours %.1fs codegraph scan-to-ready %ss, incl. model load %ss (ratio %s, incl. load %s) | cg-only %d ours-only %d both %d' % (c['project'],o['precision'],o['recall'],c['precision'],c['recall'],ours_s,c['scan_to_ready_s'],c['index_s'],c['index_ratio_ours_to_codegraph'],c['index_ratio_incl_model_load'],len(c['codegraph_only_correct']),len(c['ours_only_correct']),c['both_correct']), file=sys.stderr)
" "$SCR/out/$NAME.ours.json" "$SCR/out/$NAME.cg.json" "$OURS_S"
done
echo "load $(cut -d' ' -f1-3 /proc/loadavg) at end" >&2
