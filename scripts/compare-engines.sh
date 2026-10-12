#!/usr/bin/env bash
# compare-engines.sh — measured head-to-head of CompareGraph with code-review-graph (PyPI "code-review-graph").
#
# usage: compare-engines.sh <scratch-dir> <comparegraph-binary> <project-dir> [<project-dir> ...]
#   env: CRG_VERSION (default 2.3.9)  REPS (cold builds per engine, default 3)  SEED (default 7)  SAMPLE (default 20)
#
# Each <project-dir> is COPIED into <scratch-dir>/work/<basename> (code-review-graph writes its store inside the repo it
# indexes, so it never runs on the original); both engines build the same copy. Needs python3 with venv + pip, bc, nice.
# Output: <scratch-dir>/out/<project>.json (all numbers + the call-edge sample) and a summary on stdout.
#
# KIND MAPPING (read from code-review-graph 2.3.9: graph.py schema; parser.py NodeInfo/EdgeInfo kinds; applied in analyze.py).
# Nodes are identified by (path relative to the project root, symbol name); file nodes by (path, "").
#   node  File                          -> file (our file and test-kind file nodes)
#   node  Function with parent_name     -> method          node  Function without parent_name -> function
#   node  Test (is_test function)       -> function/method (we keep test code as function/method; test FILES are kind test)
#   node  Class -> class     Type -> type
#   edge  CALLS -> calls     IMPORTS_FROM -> imports (file to file)     CONTAINS -> contains
#   edge  INHERITS -> extends     IMPLEMENTS -> implements (also compared as one "inheritance" set: the engines label
#         interface-vs-class differently per language)
#   edge  TESTED_BY (prod -> test) -> tests, REVERSED to our direction (test -> prod)
#   not mapped (we have no counterpart): REFERENCES CONSUMES DEPENDS_ON DEPENDS_ON_CONFIG HANDLES INJECTS PRODUCES PUBLISHES
#         TRIGGERS TEMPORAL_STUB; our routes_to has no counterpart either.
# An edge is "resolved" when its target (code-review-graph: target_qualified equals a node's qualified_name) is an indexed
# node; code-review-graph also stores unresolved bare-name call targets ("set"), compared by target name only.
set -euo pipefail
[ $# -ge 3 ] || { sed -n 2,5p "$0"; exit 2; }
SCR=$(readlink -f "$1"); CG=$(readlink -f "$2"); shift 2
CRG_VERSION=${CRG_VERSION:-2.3.9}; REPS=${REPS:-3}; SEED=${SEED:-7}; SAMPLE=${SAMPLE:-20}
mkdir -p "$SCR/work" "$SCR/out" "$SCR/tools"
if [ ! -x "$SCR/venv/bin/code-review-graph" ]; then
  python3 -m venv "$SCR/venv"; nice -n 10 "$SCR/venv/bin/pip" install -q "code-review-graph==$CRG_VERSION"
fi
CRG="$SCR/venv/bin/code-review-graph"
echo "code-review-graph $("$SCR/venv/bin/pip" list 2>/dev/null | awk '$1=="code-review-graph"{print $2}')"
now() { date +%s.%N; }
median() { sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'; }
clean() { for d in "$@"; do [ -d "$d" ] && find "$d" -mindepth 1 -delete && rmdir "$d"; done; return 0; }

cat > "$SCR/tools/analyze.py" <<'PY'
import json, os, random, sqlite3, sys
proj, root, crg_db, cg_db, name, seed, nsample, outp = sys.argv[1:9]
root = os.path.realpath(root)
def rel(p):
    return os.path.relpath(p, root) if os.path.isabs(p) else p
crg = sqlite3.connect(crg_db); cg = sqlite3.connect(cg_db)
def crg_kind(k, parent):
    if k == "File": return "file"
    if k == "Class": return "class"
    if k == "Type": return "type"
    return "method" if parent else "function"          # Function, Test
cn = {}                                                  # qualified_name -> (kind, key)
for q, k, nm, fp, parent in crg.execute("select qualified_name,kind,name,file_path,parent_name from nodes"):
    kind = crg_kind(k, parent)
    cn[q] = (kind, (rel(fp), "" if kind == "file" else nm))
rid = cg.execute("select r.id from run r join repo p on p.id=r.repo_id where p.name=? and r.status='done' order by r.id desc limit 1", (name,)).fetchone()[0]
on = {}
for i, k, nm, p in cg.execute("select id,kind,name,path from node where run_id=?", (rid,)):
    kind = "file" if k in ("file", "test") and nm == os.path.basename(p) else k
    on[i] = (kind, (p, "" if kind == "file" else nm))
def jac(a, b):
    u = len(a | b)
    return round(len(a & b) / u, 4) if u else None
res = {"project": proj, "nodes": {}, "edges": {}}
SYM = ("function", "method", "class", "type")
for kind in ("file", "function", "method", "class", "type"):
    a = {v[1] for v in cn.values() if v[0] == kind}; b = {v[1] for v in on.values() if v[0] == kind}
    res["nodes"][kind] = {"crg": sum(1 for v in cn.values() if v[0] == kind), "cg": sum(1 for v in on.values() if v[0] == kind),
                          "both": len(a & b), "jaccard": jac(a, b)}
a = {v[1] for v in cn.values() if v[0] in SYM}; b = {v[1] for v in on.values() if v[0] in SYM}
res["nodes"]["symbols(function|method|class|type)"] = {"crg_set": len(a), "cg_set": len(b), "both": len(a & b), "jaccard": jac(a, b),
                                                        "crg_only": len(a - b), "cg_only": len(b - a)}
KM = {"CALLS": "calls", "IMPORTS_FROM": "imports", "CONTAINS": "contains", "INHERITS": "extends", "IMPLEMENTS": "implements", "TESTED_BY": "tests"}
cr = {k: {"all": 0, "res": set(), "name": set()} for k in KM.values()}
calls_detail = {}
by_name = {}
for q, (kd, key) in cn.items():
    if kd in ("class", "type"): by_name.setdefault(key[1], []).append(q)
for kind, s, t, fp, line in crg.execute("select kind,source_qualified,target_qualified,file_path,line from edges"):
    k = KM.get(kind)
    if not k: continue
    if k == "tests": s, t = t, s                        # TESTED_BY prod->test becomes test->prod
    if s not in cn: continue
    src = cn[s][1]
    cr[k]["all"] += 1
    if t not in cn and k in ("extends", "implements"):          # bare base-class name: bind it when exactly one class/type has that name
        c = by_name.get(t.split(".")[-1], [])
        if len(c) == 1: t = c[0]
    if t in cn:
        key = (src, cn[t][1]); cr[k]["res"].add(key); cr[k]["name"].add((src, cn[t][1][1]))
        if k == "calls": calls_detail.setdefault(key, (rel(fp), line))
    else:
        cr[k]["name"].add((src, t.split("::")[-1].split(".")[-1]))
og = {k: {"all": 0, "res": set(), "name": set(), "lo": 0} for k in KM.values()}
ospan = {i: (ls, le) for i, ls, le in cg.execute("select id,line_start,line_end from node where run_id=?", (rid,))}
ocall = {}
for k, s, t, conf in cg.execute("select kind,source_node_id,target_node_id,confidence from edge where run_id=?", (rid,)):
    if k not in og or s not in on or t not in on: continue
    if k == "calls": ocall.setdefault((on[s][1], on[t][1]), (s, t, conf))
    og[k]["all"] += 1; og[k]["res"].add((on[s][1], on[t][1])); og[k]["name"].add((on[s][1], on[t][1][1]))
    if conf < 1.0: og[k]["lo"] += 1
for k in KM.values():
    A, B = cr[k]["res"], og[k]["res"]
    res["edges"][k] = {"crg_total": cr[k]["all"], "crg_resolved_distinct": len(A), "cg_total": og[k]["all"], "cg_distinct": len(B),
                       "cg_below_1.0": og[k]["lo"], "both": len(A & B), "jaccard_resolved": jac(A, B),
                       "crg_only": len(A - B), "cg_only": len(B - A), "jaccard_by_target_name": jac(cr[k]["name"], og[k]["name"])}
A = {k[1] for k in cr["contains"]["res"]}; B = {k[1] for k in og["contains"]["res"]}
res["edges"]["contains(by contained symbol: does each symbol have a container)"] = {"crg": len(A), "cg": len(B), "both": len(A & B), "jaccard": jac(A, B)}
A = cr["extends"]["res"] | cr["implements"]["res"]; B = og["extends"]["res"] | og["implements"]["res"]
res["edges"]["inheritance(extends|implements)"] = {"crg_resolved_distinct": len(A), "cg_distinct": len(B), "both": len(A & B), "jaccard_resolved": jac(A, B)}
missing = sorted(cr["calls"]["res"] - og["calls"]["res"])
rnd = random.Random(int(seed)); pick = rnd.sample(missing, min(int(nsample), len(missing)))
samp = []
for (src, tgt) in pick:
    f, line = calls_detail[(src, tgt)]
    text = ""
    try:
        with open(os.path.join(root, f), errors="replace") as fh:
            ls = fh.read().splitlines()
        text = "\n".join(f"{n}: {ls[n-1]}" for n in range(max(1, line - 3), min(len(ls), line + 3) + 1))
    except OSError:
        pass
    same = any(s2 == src and t2[1] == tgt[1] for (s2, t2) in og["calls"]["res"])
    samp.append({"source": list(src), "target": list(tgt), "at": f"{f}:{line}", "cg_has_same_name_edge_from_source": same, "context": text})
omiss = sorted(og["calls"]["res"] - cr["calls"]["res"])
osamp = []
for key in rnd.sample(omiss, min(int(nsample), len(omiss))):
    (src, tgt) = key; sid, tid, conf = ocall[key]
    text = ""
    try:
        with open(os.path.join(root, src[0]), errors="replace") as fh:
            ls = fh.read().splitlines()
        a, b = ospan[sid]
        hits = [f"{n}: {ls[n-1].strip()}" for n in range(max(1, a), min(len(ls), b) + 1) if tgt[1] in ls[n-1]]
        text = "\n".join(hits[:4]) if hits else "NO LINE IN THE CALLER MENTIONS THE TARGET NAME"
    except OSError:
        pass
    osamp.append({"source": list(src), "target": list(tgt), "confidence": conf, "caller_span": list(ospan[sid]), "lines_mentioning_target": text})
res["cg_only_call_sample"] = {"population": len(omiss), "items": osamp}
res["call_sample"] = {"population_crg_resolved_calls_we_lack": len(missing), "seed": int(seed), "items": samp}
json.dump(res, open(outp, "w"), indent=1)
PY

printf '%-22s %9s %10s %9s %10s %5s\n' project "crg(s)" "crg-raw(s)" "cg(s)" "cg-warm(s)" reps
for src in "$@"; do
  name=$(basename "$src"); W="$SCR/work/$name"
  clean "$W"; cp -a "$src" "$W"
  ct=(); rt=(); gt=()
  for i in $(seq "$REPS"); do
    clean "$W/.code-review-graph" "$SCR/ws-$name"
    s=$(now); nice -n 10 "$CRG" build --repo "$W" -q >"$SCR/out/$name.crg.log" 2>&1; e=$(now); ct+=("$(echo "$e-$s" | bc)")
    clean "$W/.code-review-graph"
    s=$(now); nice -n 10 "$CRG" build --repo "$W" -q --skip-postprocess >/dev/null 2>&1; e=$(now); rt+=("$(echo "$e-$s" | bc)")
    clean "$W/.code-review-graph"
    nice -n 10 "$CRG" build --repo "$W" -q >/dev/null 2>&1                      # leave the full store for the analysis
    "$CG" --root "$SCR/ws-$name" add "$W" --name "$name" >/dev/null
    s=$(now); nice -n 10 "$CG" --root "$SCR/ws-$name" build "$name" >"$SCR/out/$name.cg.log" 2>&1; e=$(now); gt+=("$(echo "$e-$s" | bc)")
  done
  s=$(now); nice -n 10 "$CG" --root "$SCR/ws-$name" build "$name" >/dev/null 2>&1; e=$(now); warm=$(echo "$e-$s" | bc)
  c=$(printf '%s\n' "${ct[@]}" | median); r=$(printf '%s\n' "${rt[@]}" | median); g=$(printf '%s\n' "${gt[@]}" | median)
  printf '%-22s %9.2f %10.2f %9.2f %10.2f %5s\n' "$name" "$c" "$r" "$g" "$warm" "$REPS"
  python3 -I "$SCR/tools/analyze.py" "$name" "$W" "$W/.code-review-graph/graph.db" "$SCR/ws-$name/.comparegraph/comparegraph.db" "$name" "$SEED" "$SAMPLE" "$SCR/out/$name.json"
  python3 -I - "$SCR/out/$name.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k, v in d["nodes"].items(): print("  node", k, v)
for k, v in d["edges"].items(): print("  edge", k, v)
print("  sample population", d["call_sample"]["population_crg_resolved_calls_we_lack"])
PY
done
