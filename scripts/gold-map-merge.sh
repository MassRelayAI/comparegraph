#!/usr/bin/env bash
# gold-map-merge.sh — merge two readers' labels into fixtures/gold-map/<clone>/features.jsonl (Plan 7 lane 1).
# usage: gold-map-merge.sh <reader1-dir> <reader2-dir>   (each holds <clone>.jsonl rows {id, verdict, locations, searched, note})
# Normalises locations, applies the final rule of fixtures/gold-map/README.md, prints per-clone and pooled agreement and Cohen's kappa.
# Disagreements stay in the file with final null; nothing is adjudicated.
set -euo pipefail
[ $# -eq 2 ] || { sed -n 2,5p "$0" >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
exec python3 -I - "$ROOT" "$1" "$2" <<'PY'
import json, os, re, sys
root, d1, d2 = sys.argv[1:4]
VERD = ["has", "partial", "none", "unknown"]

def norm(loc):
    loc = loc.strip().replace("\\", "/")
    while loc.startswith("./"):
        loc = loc[2:]
    if "::" in loc:
        p, s = loc.split("::", 1)
        s = re.split(r"\.|::", s.strip())[-1]
        s = re.sub(r"\(.*$", "", s)
        return f"{p}::{s}" if s else None
    m = re.match(r"^(.*):(\d+)(?:-\d+)?$", loc)
    return f"{m.group(1)}:{m.group(2)}" if m else None

def parse(l):
    if "::" in l:
        p, s = l.split("::", 1); return p, s, None
    p, n = l.rsplit(":", 1); return p, None, int(n)

def shared(a, b):
    out = []
    for x in a:
        px, sx, lx = parse(x)
        for y in b:
            py, sy, ly = parse(y)
            if px == py and ((sx and sx == sy) or (lx is not None and ly is not None and abs(lx - ly) <= 10)):
                if x not in out: out.append(x)
    return out

def read(p):
    return [json.loads(l) for l in open(p) if l.strip()]

def kappa(pairs):
    n = len(pairs)
    if not n: return None
    po = sum(a == b for a, b in pairs) / n
    pe = sum((sum(a == c for a, _ in pairs) / n) * (sum(b == c for _, b in pairs) / n) for c in VERD)
    return 1.0 if pe == 1 else round((po - pe) / (1 - pe), 3)

pairs_all = [json.loads(l) for l in open(f"{root}/fixtures/gold-map/pairs.jsonl") if l.strip()]
clones = []
for p in pairs_all:
    if p["clone"] not in clones: clones.append(p["clone"])
pool = []
print("%-18s %4s %6s %6s  %s  %s" % ("clone", "rows", "agree", "null", "final verdicts", "kappa"))
for c in clones:
    pr = [p for p in pairs_all if p["clone"] == c]
    r1 = {r["id"]: r for r in read(f"{d1}/{c}.jsonl")}
    r2 = {r["id"]: r for r in read(f"{d2}/{c}.jsonl")}
    rows, vp, counts, agreed = [], [], {v: 0 for v in VERD}, 0
    for p in pr:
        a, b = r1[p["id"]], r2[p["id"]]
        for r in (a, b):
            r["locations"] = [n for n in (norm(x) for x in r.get("locations", [])) if n]
        final = None
        if a["verdict"] == b["verdict"]:
            if a["verdict"] in ("has", "partial"):
                s = shared(a["locations"], b["locations"])[:3]
                if s: final = {"verdict": a["verdict"], "locations": s}
            else:
                final = {"verdict": a["verdict"], "locations": []}
        if final:
            agreed += 1; counts[final["verdict"]] += 1
        vp.append((a["verdict"], b["verdict"]))
        rows.append({"id": p["id"], "clone": c, "feature_id": p["feature_id"],
                     "reader1": {k: a.get(k) for k in ("verdict", "locations", "searched", "note")},
                     "reader2": {k: b.get(k) for k in ("verdict", "locations", "searched", "note")}, "final": final})
    os.makedirs(f"{root}/fixtures/gold-map/{c}", exist_ok=True)
    with open(f"{root}/fixtures/gold-map/{c}/features.jsonl", "w") as f:
        for r in rows: f.write(json.dumps(r, ensure_ascii=False) + "\n")
    pool += vp
    print("%-18s %4d %6d %6d  %s  %s" % (c, len(rows), agreed, len(rows) - agreed, " ".join(f"{v}={counts[v]}" for v in VERD), kappa(vp)))
print("%-18s %4d  kappa(verdict, 4 classes) pooled %s" % ("POOLED", len(pool), kappa(pool)))
PY
