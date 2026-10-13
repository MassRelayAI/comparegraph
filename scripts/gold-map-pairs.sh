#!/usr/bin/env bash
# Regenerate fixtures/gold-map/pairs.jsonl from families/code-graph-tools/rows.md. Deterministic: clone order x pack row order.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
python3 -I - "$root" <<'PY'
import json, re, sys
root = sys.argv[1]
clones = ["graphmind", "srclight", "code-review-graph", "CodeGraph", "codanna", "ripwire"]
rows = []
for line in open(f"{root}/families/code-graph-tools/rows.md", encoding="utf-8"):
    if not line.startswith("|") or line.startswith("| id ") or line.startswith("|---"):
        continue
    c = [x.strip() for x in line.strip().strip("|").split("|")]
    m = re.match(r"^(.*) \[kinds: ([^\]]*)\]$", c[3])
    rows.append((c[0], c[2], m.group(1), c[4], [k.strip() for k in m.group(2).split(",")]))
with open(f"{root}/fixtures/gold-map/pairs.jsonl", "w", encoding="utf-8", newline="\n") as f:
    for clone in clones:
        for n, (fid, title, d, cah, kinds) in enumerate(rows, 1):
            f.write(json.dumps({"id": f"map-{clone.lower()}-{n:03d}", "clone": clone, "feature_id": fid, "title": title,
                                "definition": d, "counts_as_has": cah, "kinds": kinds}, ensure_ascii=False) + "\n")
print(f"{len(clones) * len(rows)} pairs")
PY
