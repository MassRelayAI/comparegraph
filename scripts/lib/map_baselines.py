"""Feature-mapping baselines against the mapping gold (Plan 7 lane 1, task H2H), scored with scripts/lib/goldmap.py.

usage: map_baselines.py <name> <clone-copy> <features.jsonl> <workspace> <family> <binary> <codegraph HOME dir> <out.json>

Inputs are identical for every method: the `terms` field of `comparegraph map` for each feature (the workspace holds the finished build
and the family; scripts/map-baselines.sh made them). Every method returns its top 5 candidates {path, name, line_start, line_end} and
goldmap.score_map scores them against the same gold with the same graph (so the same unresolved locations are left out).
Methods:
  ours        the `map` output already in <workspace>/.map.json (written by map-score.sh)
  fts_or      the FTS5 index of the workspace, quoted-prefix terms OR'd, bm25 best first: the query `map` itself starts from, run directly in sqlite
              (the `search` verb cannot OR: it ANDs its tokens)
  fts_and     `comparegraph search "<terms>" --limit 5`, the verb as shipped (tokens ANDed)
  cg_symbol   codegraph_symbol_search, query = terms joined by spaces, limit 5
  cg_pattern  codegraph_search_by_pattern, pattern = terms joined by `|`, limit 5 (regex over names, bodies, signatures; unranked)
codegraph-mcp runs with HOME=<dir> (fresh), --graph-only, over MCP stdio; its paths are absolute and made relative to the clone.
"""
import json, os, re, sqlite3, subprocess, sys, threading

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import goldmap as G


def run_json(cmd):
    r = subprocess.run(["nice", "-n", "10"] + cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit("failed: %s\n%s" % (" ".join(cmd), r.stderr))
    return json.loads(r.stdout)


def cand_from_hit(h):
    return {"path": h["path"], "name": h.get("name") or G.bare(h["qualified_name"]), "kind": h.get("kind"),
            "line_start": h.get("line_start"), "line_end": h.get("line_end")}


def fts_or(db, repo, terms, k):
    if not terms:
        return []
    q = " OR ".join('"%s"*' % t for t in terms)
    c = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
    sql = ("SELECT n.qualified_name, n.kind, n.path, n.name, n.line_start, n.line_end, -bm25(node_fts) AS score FROM node_fts "
           "JOIN node n ON n.id = node_fts.rowid JOIN repo r ON r.id = n.repo_id "
           "WHERE node_fts MATCH ?1 AND n.run_id = (SELECT MAX(id) FROM run WHERE repo_id = n.repo_id AND status = 'done') AND r.name = ?2 "
           "ORDER BY score DESC, n.qualified_name ASC, n.id ASC LIMIT ?3")
    return [{"path": p, "name": G.bare(qn), "kind": kd, "line_start": a, "line_end": b}
            for qn, kd, p, nm, a, b, _ in c.execute(sql, (q, repo, k))]


def fts_and(binary, ws, repo, terms, k):
    if not terms:
        return []
    d = run_json([binary, "--root", ws, "search", " ".join(terms), "--repo", repo, "--limit", str(k)])
    return [cand_from_hit(h) for h in d["data"]["items"]][:k]


class Codegraph:
    def __init__(self, proj, home):
        os.makedirs(home, exist_ok=True)
        self.proj = os.path.realpath(proj)
        env = dict(os.environ, HOME=os.path.realpath(home), NO_COLOR="1")
        self.log = []
        self.p = subprocess.Popen(["nice", "-n", "10", "codegraph-mcp", "--mcp", "--graph-only", "--workspace", self.proj],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.done = threading.Event()
        threading.Thread(target=self._pump, daemon=True).start()
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "h2h-map", "version": "1"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()
        # lazy index: the first tools/call starts it and returns when it is done
        self.rpc("tools/call", {"name": "codegraph_get_callees", "arguments": {"nodeId": "0"}})
        if not self.done.wait(1500):
            self.p.terminate()
            raise SystemExit("codegraph index did not finish")

    def _pump(self):
        for l in self.p.stderr:
            self.log.append(l)
            if "Indexed " in l and " files (" in l:
                self.done.set()
        self.done.set()

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            l = self.p.stdout.readline()
            if not l:
                raise SystemExit("codegraph-mcp closed its stdout")
            r = json.loads(l)
            if r.get("id") == self.n:
                return r

    def call(self, tool, args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        try:
            return json.loads(r["result"]["content"][0]["text"])
        except Exception:
            return {}

    def rel(self, f):
        return os.path.relpath(f, self.proj) if os.path.isabs(f) else f

    def symbol(self, terms, k):
        if not terms:
            return []
        b = self.call("codegraph_symbol_search", {"query": " ".join(terms), "limit": k})
        out = []
        for x in b.get("results", [])[:k]:
            s, loc = x["symbol"], x["symbol"]["location"]
            out.append({"path": self.rel(loc["file"]), "name": s["name"], "kind": str(s.get("kind", "")).lower(),
                        "line_start": loc.get("line"), "line_end": loc.get("end_line")})
        return out

    def pattern(self, terms, k):
        if not terms:
            return []
        b = self.call("codegraph_search_by_pattern", {"pattern": "|".join(re.escape(t) for t in terms), "limit": k})
        return [{"path": self.rel(m["path"]), "name": m["name"], "kind": m.get("kind"), "line_start": m.get("line_start"), "line_end": m.get("line_end")}
                for m in b.get("matches", [])[:k]]

    def close(self):
        try:
            self.p.stdin.close()
            self.p.terminate()
        except Exception:
            pass


def main():
    name, proj, gold, ws, family, binary, home, outp = sys.argv[1:9]
    k = 5
    ws = os.path.realpath(ws)
    db = ws + "/.comparegraph/comparegraph.db"
    rows = G.read_jsonl(gold)
    cg = sqlite3.connect(db)
    rid = cg.execute("select r.id from run r join repo p on p.id=r.repo_id where p.name=? and r.provider='reference' and r.status='done' order by r.id desc limit 1", (name,)).fetchone()[0]
    graph = G.load_graph(cg, rid)
    mp = run_json([binary, "--root", ws, "map", name, family, "--limit", str(k)])["data"]["features"]
    terms = {f["id"]: f["terms"] for f in mp}
    ours = G.load_map(open(ws + "/.map.json").read())
    feats = sorted({r["feature_id"] for r in rows})
    cgs = Codegraph(proj, home)
    methods = {"ours": ours, "fts_or": {}, "fts_and": {}, "cg_symbol": {}, "cg_pattern": {}}
    try:
        for f in feats:
            t = terms.get(f, [])
            methods["fts_or"][f] = fts_or(db, name, t, k)
            methods["fts_and"][f] = fts_and(binary, ws, name, t, k)
            methods["cg_symbol"][f] = cgs.symbol(t, k)
            methods["cg_pattern"][f] = cgs.pattern(t, k)
    finally:
        cgs.close()
    out = {"clone": name, "terms_features": len(terms), "methods": {}, "per_feature": {}, "same_as_ours": {}}
    for m, mapped in methods.items():
        out["methods"][m] = G.score_map(rows, mapped, graph, k)
    for f in feats:
        frows = [r for r in rows if r["feature_id"] == f]
        out["per_feature"][f] = {m: G.score_map(frows, methods[m], graph, k) for m in methods}
    key = lambda cs: [(c["path"], c["line_start"], c["line_end"]) for c in cs[:k]]
    same = [f for f in feats if key(ours.get(f, [])) == key(methods["fts_or"][f])]
    out["same_as_ours"] = {"fts_or_identical_features": len(same), "features": len(feats)}
    json.dump(out, open(outp, "w"))
    print(outp, file=sys.stderr)


main()
