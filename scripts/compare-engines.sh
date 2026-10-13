#!/usr/bin/env bash
# compare-engines.sh — measured head-to-head of CompareGraph with code-review-graph (PyPI "code-review-graph").
#
# usage: compare-engines.sh [--sample-size N] [--gold N --gold-seed S] <scratch-dir> <comparegraph-binary> <project-dir> [...]
#   env: CRG_VERSION (default 2.3.9)  REPS (cold builds per engine, default 3)  SEED (default 7)  SAMPLE (default 20)
#   --sample-size N   draw N crg-only and N ours-only resolved call edges (seed SEED), overriding SAMPLE, and write them as JSONL
#   --gold N          draw N call SITES from the project's own source files (not from either engine's output) by --gold-seed S
#                     (default 11): random (file, line) pairs whose line holds a call expression per a per-language regex
#   Without --sample-size and --gold nothing but the old <project>.json is written. With either, JSONL rows
#   {id, path, line, callee, context: [3 lines before .. 3 after], engine_target?} go to <scratch-dir>/samples/<project>/
#   crg-only.jsonl, ours-only.jsonl, gold.jsonl; gold-engines.jsonl (what each engine says at each gold site, for the scorer only)
#   stays beside them. Labels never go in these files. Builds alternate per repetition (ours cold, crg cold, crg raw) and the
#   scratch dir gets out/<project>.load.txt with uptime before and after; the unchanged ours rebuild is checked for "reused".
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
SAMPLE_SIZE=""; GOLD=0; GOLD_SEED=11
while [ $# -gt 0 ]; do
  case "$1" in
    --sample-size) SAMPLE_SIZE=$2; shift 2;;
    --gold) GOLD=$2; shift 2;;
    --gold-seed) GOLD_SEED=$2; shift 2;;
    --*) echo "unknown option $1" >&2; exit 2;;
    *) break;;
  esac
done
[ $# -ge 3 ] || { sed -n 2,5p "$0"; exit 2; }
SCR=$(readlink -f "$1"); CG=$(readlink -f "$2"); shift 2
CRG_VERSION=${CRG_VERSION:-2.3.9}; REPS=${REPS:-3}; SEED=${SEED:-7}; SAMPLE=${SAMPLE:-20}
[ -z "$SAMPLE_SIZE" ] || SAMPLE=$SAMPLE_SIZE
EMIT=0; { [ -n "$SAMPLE_SIZE" ] || [ "$GOLD" -gt 0 ]; } && EMIT=1
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
proj, root, crg_db, cg_db, name, seed, nsample, outp, sdir, gold_n, gold_seed, libdir = sys.argv[1:13]
sys.path.insert(0, libdir)
import gold as G
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

def ctx_lines(path, line):
    try:
        with open(os.path.join(root, path), errors="replace") as fh:
            ls = fh.read().splitlines()
    except OSError:
        return []
    return ls[max(0, line - 4):line + 3]
def write_jsonl(path, rows):
    with open(path, "w") as fh:
        for r in rows: fh.write(json.dumps(r) + "\n")
if sdir != "-":
    import re
    os.makedirs(sdir, exist_ok=True)
    rows = []
    for n, it in enumerate(samp, 1):
        f, line = it["at"].rsplit(":", 1); line = int(line)
        rows.append({"id": f"{name}-crg-{n:03d}", "path": f, "line": line, "callee": it["target"][1], "context": ctx_lines(f, line),
                     "engine_target": f"{it['target'][0]}::{it['target'][1]}", "engine_source": f"{it['source'][0]}::{it['source'][1]}"})
    write_jsonl(os.path.join(sdir, "crg-only.jsonl"), rows)
    rows = []
    for n, it in enumerate(osamp, 1):
        src, tgt = it["source"], it["target"]
        try:
            with open(os.path.join(root, src[0]), errors="replace") as fh: ls = fh.read().splitlines()
        except OSError:
            ls = []
        a, b = it["caller_span"]
        hits = [k for k in range(max(1, a), min(len(ls), b) + 1) if tgt[1] in ls[k - 1]]
        # the site shown: a line that calls the name, never a definition or import line and never the caller's own first line;
        # strict match first (the name itself followed by "("), then an aliased one (`_name(`), then any other non-definition mention
        defre = re.compile(r"^\s*(?:(?:async|pub(?:\([^)]*\))?|export|static|public|private|protected|final|abstract)\s+)*(?:def|fn|func|function|class|struct|enum|trait|interface|type|import|from|use)\b")
        okl = [k for k in hits if k != a and not defre.match(ls[k - 1])]
        strict = re.compile(r"(?<![\w$])" + re.escape(tgt[1]) + r"\s*[(]"); loose = re.compile(re.escape(tgt[1]) + r"\s*[(]")
        line = ([k for k in okl if strict.search(ls[k - 1])] or [k for k in okl if loose.search(ls[k - 1])] or okl or [k for k in hits if k != a] or hits or [a])[0]
        rows.append({"id": f"{name}-ours-{n:03d}", "path": src[0], "line": line, "callee": tgt[1], "context": ctx_lines(src[0], line),
                     "engine_target": f"{tgt[0]}::{tgt[1]}", "engine_source": f"{src[0]}::{src[1]}", "mention_lines": hits[:12],
                     "caller_span": [a, b], "confidence": it["confidence"]})
    write_jsonl(os.path.join(sdir, "ours-only.jsonl"), rows)
    if int(gold_n) > 0:
        EXT = {".py", ".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".go"}
        SKIPD = {".git", ".code-review-graph", "node_modules", "vendor", "target", "dist", "build", "__pycache__", ".comparegraph"}
        CALL = re.compile(r"([A-Za-z_$][\w$]*(?:\s*(?:\.|::|\?\.)\s*[A-Za-z_$][\w$]*)*)\s*[(]")
        KW = {"if", "for", "while", "switch", "catch", "return", "def", "fn", "func", "function", "match", "with", "elif", "assert", "except",
              "and", "or", "not", "in", "is", "await", "async", "yield", "lambda", "class", "struct", "enum", "impl", "typeof", "new", "else", "go", "defer", "select", "case", "do", "throw", "raise", "print"}
        DEFL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:export\s+)?(?:default\s+)?(?:def|fn|func|function|class|interface|type|import|from|use|package|mod|struct|enum|impl|trait)\b")
        cands = []
        for dp, dn, fn in os.walk(root):
            dn[:] = sorted(d for d in dn if d not in SKIPD)
            for f in sorted(fn):
                if os.path.splitext(f)[1] not in EXT: continue
                full = os.path.join(dp, f); rp = os.path.relpath(full, root)
                try:
                    with open(full, errors="replace") as fh: ls = fh.read().splitlines()
                except OSError: continue
                for n, text in enumerate(ls, 1):
                    st = text.strip()
                    if not st or st.startswith(("#", "//", "*", "/*", "\x27\x27\x27", "\x22\x22\x22")) or DEFL.match(text): continue
                    for m in CALL.finditer(text):
                        callee = re.sub(r"\s+", "", m.group(1))
                        if re.split(r"\.|::|\?\.", callee)[-1] in KW or callee in KW: continue
                        cands.append((rp, n, callee)); break
        rg = random.Random(int(gold_seed))
        pick = sorted(rg.sample(cands, min(int(gold_n), len(cands))))
        grows = [{"id": f"{name}-gold-{n:03d}", "path": f, "line": l, "callee": c, "context": ctx_lines(f, l)} for n, (f, l, c) in enumerate(pick, 1)]
        write_jsonl(os.path.join(sdir, "gold.jsonl"), grows)
        # what each engine says at each gold site (for the scorer; not for readers): calls from the smallest function/method span
        # containing the line whose target name equals the callee's last segment
        gnodes, ospans, gcalls = G.load_ours(cg, rid)
        cspans = {}
        # crg's module-level CALLS edges have the FILE as source (parser.py: `caller = enclosing function else file_path`) and its File node
        # spans 1..EOF, so the File kind is a span too (largest, hence the fallback): same rule as G.load_ours for ours
        for q, k, fp, ls_, le_ in crg.execute("select qualified_name,kind,file_path,line_start,line_end from nodes where kind in ('Function','Test','File')"):
            if ls_ is not None and le_ is not None: cspans.setdefault(rel(fp), []).append((le_ - ls_, ls_, le_, q))
        cedges = {}
        for s_, t_ in crg.execute("select source_qualified,target_qualified from edges where kind='CALLS'"):
            cedges.setdefault(s_, []).append(t_)
        erows = []
        for g in grows:
            nm = G.last_segment(g["callee"])
            enc_o, ot = G.ours_answer(gnodes, ospans, gcalls, g["path"], g["line"], g["callee"]); o = True if enc_o else None
            c = G.enclosing(cspans, g["path"], g["line"]); ct = []
            if c is not None:
                for t in cedges.get(c, []):
                    if t in cn:
                        if cn[t][1][1] == nm: ct.append(f"{cn[t][1][0]}::{cn[t][1][1]}")
                    elif t.split("::")[-1].split(".")[-1] == nm: ct.append("unresolved::" + nm)
            erows.append({"id": g["id"], "enclosed_ours": o is not None, "enclosed_crg": c is not None, "ours": ot, "crg": sorted(set(ct))})
        write_jsonl(os.path.join(sdir, "gold-engines.jsonl"), erows)
PY

printf '%-22s %9s %10s %9s %10s %5s\n' project "crg(s)" "crg-raw(s)" "cg(s)" "cg-warm(s)" reps
for src in "$@"; do
  name=$(basename "$src"); W="$SCR/work/$name"
  clean "$W"; cp -a "$src" "$W"
  ct=(); rt=(); gt=()
  { echo "before: $(uptime)"; } > "$SCR/out/$name.load.txt"
  for i in $(seq "$REPS"); do
    # alternate: ours cold, crg cold (full), crg cold (raw), so neither engine always runs first or last
    clean "$SCR/ws-$name"
    "$CG" --root "$SCR/ws-$name" add "$W" --name "$name" >/dev/null
    s=$(now); nice -n 10 "$CG" --root "$SCR/ws-$name" build "$name" >"$SCR/out/$name.cg.log" 2>&1; e=$(now); gt+=("$(echo "$e-$s" | bc)")
    clean "$W/.code-review-graph"
    s=$(now); nice -n 10 "$CRG" build --repo "$W" -q >"$SCR/out/$name.crg.log" 2>&1; e=$(now); ct+=("$(echo "$e-$s" | bc)")
    clean "$W/.code-review-graph"
    s=$(now); nice -n 10 "$CRG" build --repo "$W" -q --skip-postprocess >/dev/null 2>&1; e=$(now); rt+=("$(echo "$e-$s" | bc)")
    clean "$W/.code-review-graph"
  done
  nice -n 10 "$CRG" build --repo "$W" -q >/dev/null 2>&1                          # leave the full store for the analysis
  s=$(now); nice -n 10 "$CG" --root "$SCR/ws-$name" build "$name" >"$SCR/out/$name.cg-warm.log" 2>&1; e=$(now); warm=$(echo "$e-$s" | bc)
  grep -q '"reused":true' "$SCR/out/$name.cg-warm.log" && reused=reused || reused=NOT-REUSED
  { echo "after:  $(uptime)"; echo "warm rebuild: $reused"; echo "ours cold s: ${gt[*]}"; echo "crg cold s: ${ct[*]}"; } >> "$SCR/out/$name.load.txt"
  c=$(printf '%s\n' "${ct[@]}" | median); r=$(printf '%s\n' "${rt[@]}" | median); g=$(printf '%s\n' "${gt[@]}" | median)
  printf '%-22s %9.2f %10.2f %9.2f %10.2f %5s %s\n' "$name" "$c" "$r" "$g" "$warm" "$REPS" "$reused"
  python3 -I "$SCR/tools/analyze.py" "$name" "$W" "$W/.code-review-graph/graph.db" "$SCR/ws-$name/.comparegraph/comparegraph.db" "$name" "$SEED" "$SAMPLE" "$SCR/out/$name.json" "$([ "$EMIT" = 1 ] && echo "$SCR/samples/$name" || echo -)" "$GOLD" "$GOLD_SEED" "$(cd "$(dirname "$0")/lib" && pwd)"
  python3 -I - "$SCR/out/$name.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k, v in d["nodes"].items(): print("  node", k, v)
for k, v in d["edges"].items(): print("  edge", k, v)
print("  sample population", d["call_sample"]["population_crg_resolved_calls_we_lack"])
PY
done
