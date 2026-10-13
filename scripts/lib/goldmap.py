"""The mapping gold scorer (Plan 7 lane 1). Used by scripts/map-score.sh and, for its self-test, by crates/comparegraph/tests/map_gold.rs.

Rows: {id, clone, feature_id, reader1, reader2, final: {verdict, locations} | null}; verdict in has|partial|none|unknown; a location is
`path::symbol` or `path:line`. Rows with `final` null or verdict `unknown` are left out everywhere.

map: over final rows with verdict has/partial and >=1 location. A candidate (a `symbols` hit of `map`, top 5) matches a gold location when
its path equals the gold path and, for `path::symbol`, the symbol name equals the candidate's name, or for `path:line`, the candidate's
span holds the line. A gold location with no node of that path+name (or no node of that path holding the line) in the engine's graph is
`unresolved`: out of the denominators, never a miss of the engine. precision@5 = candidates matching some resolved gold location of their
feature / candidates; recall@5 = resolved gold locations matched by some candidate / resolved gold locations (micro, summed for the pool).
relate: expected `related` when the final gold has >=1 `has` row for the clone, else `unrelated`; ok when band related|adjacent agrees.
absent: over checks that say absent and rows whose final is has/partial/none; precision = absent calls whose gold is `none` / absent calls;
recall = gold `none` pairs called absent / gold `none` pairs. A pair never checked counts as not called absent.
"""
import json
import re
import sys


def read_jsonl(path):
    with open(path) as f:
        return [json.loads(l) for l in f if l.strip()]


def bare(qualified):
    """The symbol name of a qualified name: the last `.` or `::` segment."""
    return re.split(r"\.|::", qualified)[-1]


def load_map(data):
    """`map` output (the envelope or its data) -> {feature_id: [candidate {path, name, line_start, line_end, kind}]} in rank order."""
    if isinstance(data, str):
        data = json.loads(data)
    d = data.get("data", data)
    out = {}
    for f in d.get("features", []):
        out[f["id"]] = [
            {"path": s["path"], "name": s.get("name") or bare(s["qualified_name"]), "kind": s.get("kind"),
             "line_start": s.get("line_start"), "line_end": s.get("line_end")}
            for s in f.get("symbols", [])
        ]
    return out


def parse_loc(loc):
    """`path::symbol` -> (path, symbol, None); `path:line` -> (path, None, line)."""
    if "::" in loc:
        p, s = loc.split("::", 1)
        return p.replace("\\", "/"), s, None
    m = re.match(r"^(.*):(\d+)$", loc)
    if m:
        return m.group(1).replace("\\", "/"), None, int(m.group(2))
    return loc.replace("\\", "/"), None, None


def final_of(r):
    f = r.get("final")
    if not f or f.get("verdict") in (None, "unknown"):
        return None
    return f


def resolves(graph, loc):
    """graph: {"names": {(path, name)}, "spans": {path: [(start, end)]}}; graph None = everything resolves."""
    if graph is None:
        return True
    p, s, ln = parse_loc(loc)
    if s is not None:
        return (p, s) in graph["names"]
    if ln is not None:
        return any(a <= ln <= b for a, b in graph["spans"].get(p, []))
    return False


def hit(cand, loc):
    p, s, ln = parse_loc(loc)
    if cand["path"].replace("\\", "/") != p:
        return False
    if s is not None:
        return cand["name"] == s
    if ln is not None:
        a, b = cand.get("line_start"), cand.get("line_end")
        return a is not None and b is not None and a <= ln <= b
    return False


def _ratio(a, b):
    return round(a / b, 4) if b else None


def score_map(rows, mapped, graph=None, k=5):
    """-> counts {cands, cands_ok, golds, golds_hit, n, unresolved} plus p5, r5."""
    c = {"cands": 0, "cands_ok": 0, "golds": 0, "golds_hit": 0, "n": 0, "unresolved": 0}
    for r in rows:
        f = final_of(r)
        if not f or f["verdict"] not in ("has", "partial") or not f.get("locations"):
            continue
        locs = f["locations"][:3]
        good = [l for l in locs if resolves(graph, l)]
        c["unresolved"] += len(locs) - len(good)
        if not good:
            continue
        cands = mapped.get(r["feature_id"], [])[:k]
        c["n"] += 1
        c["cands"] += len(cands)
        c["cands_ok"] += sum(1 for x in cands if any(hit(x, l) for l in good))
        c["golds"] += len(good)
        c["golds_hit"] += sum(1 for l in good if any(hit(x, l) for x in cands))
    return _finish(c)


def _finish(c):
    c["p5"] = _ratio(c["cands_ok"], c["cands"])
    c["r5"] = _ratio(c["golds_hit"], c["golds"])
    return c


def pool(results):
    """Sum the counts of per-clone results (score_map or score_absent) and recompute the ratios."""
    keys = [k for k in results[0] if k not in ("p5", "r5", "precision", "recall")] if results else []
    c = {k: sum(r[k] for r in results) for k in keys}
    if "cands" in c:
        return _finish(c)
    return _finish_absent(c)


def score_relate(rows, band):
    expected = "related" if any((final_of(r) or {}).get("verdict") == "has" for r in rows) else "unrelated"
    got = "related" if band in ("related", "adjacent") else "unrelated"
    return {"band": band, "expected": expected, "ok": got == expected}


def score_absent(rows, checks):
    """checks: {feature_id: True if the check says absent}."""
    c = {"called": 0, "called_none": 0, "none": 0, "none_called": 0, "n": 0}
    for r in rows:
        f = final_of(r)
        if not f or f["verdict"] not in ("has", "partial", "none"):
            continue
        c["n"] += 1
        called = bool(checks.get(r["feature_id"]))
        if f["verdict"] == "none":
            c["none"] += 1
            c["none_called"] += called
        if called:
            c["called"] += 1
            c["called_none"] += f["verdict"] == "none"
    return _finish_absent(c)


def _finish_absent(c):
    c["precision"] = _ratio(c["called_none"], c["called"])
    c["recall"] = _ratio(c["none_called"], c["none"])
    return c


def load_graph(cg, run_id):
    """From an open sqlite connection on comparegraph.db: the graph of one run, for the unresolved test."""
    names, spans = set(), {}
    for p, nm, ls, le in cg.execute("select path,name,line_start,line_end from node where run_id=?", (run_id,)):
        names.add((p, nm))
        if ls is not None and le is not None:
            spans.setdefault(p, []).append((ls, le))
    return {"names": names, "spans": spans}


def selftest():
    def row(i, feat, verdict, locs=()):
        return {"id": i, "clone": "c", "feature_id": feat, "final": {"verdict": verdict, "locations": list(locs)}}

    def cand(p, n, a=1, b=9):
        return {"path": p, "name": n, "line_start": a, "line_end": b}
    graph = {"names": {("a.py", "f"), ("a.py", "g"), ("b.py", "h")}, "spans": {"a.py": [(1, 20)], "b.py": [(5, 9)]}}
    rows = [
        row("1", "x", "has", ["a.py::f"]),
        row("2", "y", "partial", ["b.py:7", "a.py::g"]),
        row("3", "z", "has", ["gone.py::q"]),            # unresolved only: row excluded
        row("4", "w", "none"),
        row("5", "v", "unknown"),
        {"id": "6", "clone": "c", "feature_id": "u", "final": None},
    ]
    mapped = {"x": [cand("a.py", "f"), cand("a.py", "zz")], "y": [cand("b.py", "h", 5, 9), cand("a.py", "other")], "z": [cand("a.py", "f")]}
    s = score_map(rows, mapped, graph)
    assert (s["n"], s["unresolved"]) == (2, 1), s
    assert (s["cands"], s["cands_ok"], s["golds"], s["golds_hit"]) == (4, 2, 3, 2), s   # name rule, line-in-span rule
    assert (s["p5"], s["r5"]) == (0.5, 0.6667), s
    assert score_map(rows, mapped)["unresolved"] == 0                                     # no graph: nothing excluded
    assert score_map([row("1", "x", "has", ["a.py::f"])], {"x": [cand("a.py", "n")] * 5 + [cand("a.py", "f")]})["r5"] == 0.0   # rank 6 is out
    assert not hit(cand("a.py", "f", 1, 9), "a.py:10") and hit(cand("a.py", "f", 1, 9), "a.py:9")
    assert not hit(cand("b.py", "f"), "a.py::f")
    # relate
    assert score_relate(rows, "adjacent") == {"band": "adjacent", "expected": "related", "ok": True}
    assert score_relate(rows, "unrelated")["ok"] is False
    assert score_relate([row("4", "w", "none")], "related")["ok"] is False
    assert score_relate([row("4", "w", "none")], "unrelated")["ok"] is True
    # absent: gold has/partial/none for x,y,w ; unknown and null excluded
    a = score_absent(rows, {"w": True, "x": True, "v": True, "u": True})
    assert (a["n"], a["called"], a["called_none"], a["none"], a["none_called"]) == (4, 2, 1, 1, 1), a
    assert (a["precision"], a["recall"]) == (0.5, 1.0), a
    b = score_absent([row("7", "q", "none"), row("8", "r", "none")], {"q": True})
    assert (b["precision"], b["recall"]) == (1.0, 0.5), b
    assert score_absent(rows, {})["precision"] is None
    # pooled vs per-clone: micro sums
    p = pool([s, score_map([row("1", "x", "has", ["a.py::f"])], {"x": [cand("a.py", "f")]}, graph)])
    assert (p["cands"], p["cands_ok"], p["golds"], p["golds_hit"], p["n"]) == (5, 3, 4, 3, 3), p
    assert (p["p5"], p["r5"]) == (0.6, 0.75), p
    pa = pool([a, b])
    assert (pa["called"], pa["called_none"], pa["none"], pa["none_called"]) == (3, 2, 3, 2), pa
    # load_map: envelope, bare names
    m = load_map({"data": {"features": [{"id": "x", "symbols": [{"path": "a.py", "qualified_name": "K.f", "kind": "method", "line_start": 1, "line_end": 2}]}]}})
    assert m["x"][0]["name"] == "f", m
    print("goldmap selftest ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--selftest"]:
        selftest()
    else:
        print("usage: goldmap.py --selftest", file=sys.stderr)
        sys.exit(2)
