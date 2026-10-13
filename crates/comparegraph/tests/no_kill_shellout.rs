//! No source file may shell out to `kill`, `pkill` or `killall`: `/usr/bin/kill -KILL -<pid>` (procps) parses a 7-digit
//! negative pid as `-1` and signals every process of the user (docs/reports/2026-10-13-user-wide-kill-from-kill-tree.md).
//! Signals go through `libc::kill` on a pid the code computed, never through a shell argument.
use std::path::Path;

fn walk(dir: &Path, hits: &mut Vec<String>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, hits);
        } else if p.extension().is_some_and(|x| x == "rs") {
            let s = std::fs::read_to_string(&p).unwrap();
            for (i, l) in s.lines().enumerate() {
                if ["Command::new(\"kill\")", "Command::new(\"pkill\")", "Command::new(\"killall\")", "\"kill -"].iter().any(|n| l.contains(n)) {
                    hits.push(format!("{}:{}: {}", p.display(), i + 1, l.trim()));
                }
            }
        }
    }
}

#[test]
fn no_source_shells_out_to_kill() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk(&src, &mut hits);
    assert!(hits.is_empty(), "kill shell-outs: {hits:#?}");
}
