"""The one gold scorer. Used by scripts/gold-score.sh and scripts/compare-engines.sh.

An engine answers a site when the smallest function, method or (when none holds the line) file node of its own graph enclosing the line has a call edge whose
target name equals the callee's last segment (`unresolved::` answers ignored). Precision = answered sites with a known truth whose
answer set contains the true target / answered sites with a known truth (an answer on an `external` site is a false answer).
Recall = repo-target sites whose answer set contains the true target / repo-target sites. Rows with `final` null, `is_call` not
`yes` or target `unknown` are left out.
"""
import json, re

SPLIT = re.compile(r"\.|::|\?\.")


def last_segment(callee):
    return SPLIT.split(callee)[-1]


def enclosing(spans, path, line):
    """spans: {path: [(length, start, end, key)]} -> key of the smallest span holding the line, or None."""
    c = [x for x in spans.get(path, []) if x[1] <= line <= x[2]]
    return min(c)[3] if c else None


def norm_target(t):
    """Path separators normalised, a `#n` suffix dropped."""
    return re.sub(r"#\d+$", "", t.replace("\\", "/"))


def load_ours(cg, rid):
    """From an open sqlite connection on comparegraph.db and a run id:
    (nodes {id: (kind, path, name)}, spans {path: [(len, start, end, id)]} of functions and methods and, as the largest span of their file, the file and test nodes, calls {source id: [target id]})."""
    nodes = {i: (k, p, nm) for i, k, nm, p in cg.execute("select id,kind,name,path from node where run_id=?", (rid,))}
    spans = {}
    for i, ls, le in cg.execute("select id,line_start,line_end from node where run_id=?", (rid,)):
        # functions and methods first; a file node (kind `file` or `test`) spans the whole file, so it is only ever the LARGEST
        # enclosing span: it answers a module-level site that no function holds (Plan 6 A1)
        if nodes[i][0] in ("function", "method", "file", "test") and ls is not None and le is not None:
            spans.setdefault(nodes[i][1], []).append((le - ls, ls, le, i))
    calls = {}
    for s, t in cg.execute("select source_node_id,target_node_id from edge where run_id=? and kind='calls'", (rid,)):
        if s in nodes and t in nodes:
            calls.setdefault(s, []).append(t)
    return nodes, spans, calls


def ours_answer(nodes, spans, calls, path, line, callee):
    """(enclosed, sorted answer set of `kind::name`-free `path::name` strings) for one site."""
    o = enclosing(spans, path, line)
    if o is None:
        return False, []
    nm = last_segment(callee)
    return True, sorted({f"{nodes[t][1]}::{nodes[t][2]}" for t in calls.get(o, []) if nodes[t][2] == nm})


def score(rows, answers):
    """rows: gold rows; answers: {id: [target strings]} (absent or empty = not answered)."""
    sites = repo = answered = correct = rcorrect = 0
    misses, false = [], []
    for r in rows:
        f = r.get("final")
        if not f or f.get("is_call") != "yes":
            continue
        tgt = f.get("target")
        if tgt in (None, "unknown"):
            continue
        sites += 1
        ans = {norm_target(a) for a in answers.get(r["id"], [])}
        if tgt != "external":
            repo += 1
        if ans:
            answered += 1
            if tgt != "external" and norm_target(tgt) in ans:
                correct += 1
            else:
                false.append(r["id"])
        if tgt != "external":
            if ans and norm_target(tgt) in ans:
                rcorrect += 1
            else:
                misses.append(r["id"])
    return {"sites": sites, "repo_target_sites": repo, "answered": answered, "correct": correct,
            "precision": round(correct / answered, 4) if answered else None,
            "recall": round(rcorrect / repo, 4) if repo else None,
            "misses": misses, "false": false}


def read_jsonl(path):
    with open(path) as fh:
        return [json.loads(l) for l in fh if l.strip()]
