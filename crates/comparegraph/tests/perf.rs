//! Scale guard for the graph queries: a wide generated project, queries at default bounds must stay fast and bounded.
//! The 400k-node figure is measured by hand (see the Plan 6 B3 commit); this keeps the shape from regressing.
use std::process::Command;
use std::time::Instant;

fn cc(root: &std::path::Path) -> assert_cmd::Command {
    let mut c = assert_cmd::Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(root);
    c
}

#[test]
fn queries_at_default_bounds_on_a_wide_project_are_fast_and_capped() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("LICENSE"), "MIT License\n\nCopyright (c) 2026 S\n").unwrap();
    std::fs::create_dir_all(d.path().join("src")).unwrap();
    // 40 modules x 60 functions; every function calls the shared hub, and a chain links neighbours
    let mut hub = String::new();
    hub.push_str("def hub():\n    pass\n");
    std::fs::write(d.path().join("src/hub.py"), hub).unwrap();
    for m in 0..40 {
        let mut s = String::from("from src.hub import hub\n\n");
        for i in 0..60 {
            s.push_str(&format!("def f{m}_{i}():\n    hub()\n"));
            if i + 1 < 60 {
                s.push_str(&format!("    f{m}_{}()\n", i + 1));
            }
            s.push('\n');
        }
        std::fs::write(d.path().join(format!("src/m{m}.py")), s).unwrap();
    }
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"],
    ] {
        assert!(Command::new("git").arg("-C").arg(d.path()).args(&args).status().unwrap().success());
    }
    let ws = tempfile::tempdir().unwrap();
    cc(ws.path()).args(["add", d.path().to_str().unwrap(), "--name", "w"]).assert().success();
    cc(ws.path()).args(["build", "w"]).assert().success();

    for args in [
        vec!["impact", "w", "src/hub.py::hub"],
        vec!["traverse", "w", "src/hub.py::hub", "--direction", "callers"],
        vec!["tests-for", "w", "src/hub.py::hub"],
        vec!["entry-points", "w", "--limit", "20"],
        vec!["dead-code", "w", "--limit", "20"],
    ] {
        let t = Instant::now();
        let out = cc(ws.path()).args(&args).assert().success().get_output().clone();
        let secs = t.elapsed().as_secs_f64();
        assert!(secs < 15.0, "{args:?} took {secs:.1}s");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        if args[0] == "impact" {
            assert_eq!(v["data"]["total"], 500, "capped at the default max-nodes");
            assert_eq!(v["data"]["truncated"], true);
        }
        if args[0] == "entry-points" {
            assert_eq!(v["data"]["items"].as_array().unwrap().len(), 20);
            assert!(v["page"]["total"].as_u64().unwrap() > 20);
        }
    }
}
