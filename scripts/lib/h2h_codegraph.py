"""Score codegraph-mcp's call graph against the gold labels with the same scorer as gold-score.sh.

usage: h2h_codegraph.py <name> <project-copy> <gold.jsonl> <fresh HOME dir> [--ours <gold-score json file>]

Method: one `codegraph-mcp --mcp --graph-only` stdio server per project (JSON-RPC: initialize, then tools/call), started with
HOME pointing at a fresh scratch directory so its store and socket never touch the real one. For each gold site the tool
`codegraph_get_callees` is called with the file URI and the 1-based gold line; the server picks the tightest node of its own graph
that contains the line (a node of any kind, file nodes included) and returns that node's callees. A callee counts as an answer when
its name equals the last segment of the gold callee; the target string is `<path relative to the project>::<name>`. A site where the
server found no containing node and fell back to a nearest node is NOT answered (a neighbour's callees would be another function's).
Output: one JSON object like gold-score.sh plus index time, node and edge counts and, with --ours, the site ids only one engine got right.
"""
import json, os, re, subprocess, sys, threading, time
from datetime import datetime

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gold as G


def main():
    name, proj, goldp, home = sys.argv[1:5]
    ours_p = sys.argv[sys.argv.index("--ours") + 1] if "--ours" in sys.argv else None
    proj = os.path.realpath(proj)
    os.makedirs(home, exist_ok=True)
    env = dict(os.environ, HOME=os.path.realpath(home), NO_COLOR="1")
    log = []
    t0 = time.time()
    p = subprocess.Popen(["nice", "-n", "10", "codegraph-mcp", "--mcp", "--graph-only", "--workspace", proj],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    done = threading.Event()

    def pump():
        for l in p.stderr:
            log.append(l)
            if "Indexed " in l and " files (" in l:
                done.set()
        done.set()
    threading.Thread(target=pump, daemon=True).start()

    def rpc(m):
        p.stdin.write(json.dumps(m) + "\n")
        p.stdin.flush()
        if "id" in m:
            while True:
                l = p.stdout.readline()
                if not l:
                    raise SystemExit("codegraph-mcp closed its stdout")
                r = json.loads(l)
                if r.get("id") == m["id"]:
                    return r
    rpc({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "h2h", "version": "1"}}})
    rpc({"jsonrpc": "2.0", "method": "notifications/initialized"})
    # the server indexes lazily, on the first tools/call after the handshake, and answers it only when the index is done: a probe call
    # starts it (and its wall time is the wall time to a ready graph)
    rpc({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "codegraph_get_callees", "arguments": {"nodeId": "0"}}})
    if not done.wait(1500):
        p.terminate()
        raise SystemExit("index did not finish in 1500 s")
    wall = time.time() - t0
    txt = "".join(log)

    def stamp(pat):
        for l in log:
            l = re.sub(r"\x1b\[[0-9;]*m", "", l)
            if re.search(pat, l):
                m = re.match(r"^(\S+)Z\s", l)
                if m:
                    return datetime.fromisoformat(m.group(1) + "+00:00").timestamp()
        return None
    a, b = stamp(r"Indexing workspace|Scanning directory"), stamp(r"Indexed \d+ files")
    sc = stamp(r"Scanning directory")
    m = re.search(r"Persisted (\d+) nodes, (\d+) edges", txt)
    nodes_n, edges_n = (int(m.group(1)), int(m.group(2))) if m else (None, None)
    mf = re.search(r"Indexed (\d+) files \((\d+) parsed, (\d+) skipped\)", txt)
    rows = G.read_jsonl(goldp)
    answers, fallback, nid = {}, 0, 10
    for r in rows:
        f = r.get("final")
        if not f or f.get("is_call") != "yes" or f.get("target") in (None, "unknown"):
            continue
        nid += 1
        res = rpc({"jsonrpc": "2.0", "id": nid, "method": "tools/call", "params": {"name": "codegraph_get_callees", "arguments": {"uri": "file://" + os.path.join(proj, r["path"]), "line": r["line"]}}})
        try:
            body = json.loads(res["result"]["content"][0]["text"])
        except Exception:
            continue
        if body.get("used_fallback"):
            fallback += 1
            continue
        want = G.last_segment(r["callee"])
        out = set()
        for c in body.get("callees", []):
            s = c.get("symbol", {})
            if s.get("name") == want:
                fp = s.get("location", {}).get("file", "")
                out.add((os.path.relpath(fp, proj) if os.path.isabs(fp) else fp) + "::" + want)
        answers[r["id"]] = sorted(out)
    p.stdin.close()
    p.terminate()
    s = G.score(rows, answers)
    res = {"project": name, "engine": "codegraph", "index_s": round(b - a, 2) if a and b else None, "scan_to_ready_s": round(b - sc, 2) if sc and b else None, "wall_to_ready_s": round(wall, 2),
           "nodes": nodes_n, "edges": edges_n, "files": mf and int(mf.group(1)), "fallback_sites": fallback, **s}
    if ours_p:
        ours = json.load(open(ours_p))
        repo = [r["id"] for r in rows if (r.get("final") or {}).get("is_call") == "yes" and (r["final"].get("target") not in (None, "unknown", "external"))]
        mine = set(repo) - set(s["misses"])
        theirs = set(repo) - set(ours["misses"])
        res["codegraph_only_correct"] = sorted(mine - theirs)
        res["ours_only_correct"] = sorted(theirs - mine)
        res["both_correct"] = len(mine & theirs)
    print(json.dumps(res))
    print("%-20s codegraph sites %4d repo %4d answered %4d correct %4d  P %s  R %s  index %ss nodes %s edges %s fallback %d" % (
        name, s["sites"], s["repo_target_sites"], s["answered"], s["correct"], s["precision"], s["recall"], res["index_s"], nodes_n, edges_n, fallback), file=sys.stderr)


main()
