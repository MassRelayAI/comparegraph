use comparegraph::application::{add::add_repo, build::build_repo, check::check, export::export_json};
use comparegraph::providers::BuildOptions;
use std::process::Command;

fn clone_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (p, c) in files {
        let f = d.path().join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, c).unwrap();
    }
    std::fs::write(
        d.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 Someone\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "x",
        ],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(d.path())
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
    d
}

#[test]
fn add_reads_head_and_licence_lane() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    pass\n")]);
    let r = add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    assert_eq!(r.name, "alpha");
    assert_eq!(r.lane, "take");
    assert_eq!(r.licence.as_deref(), Some("MIT"));
    assert_eq!(r.head.map(|h| h.len()).unwrap_or(0), 7);
    let dup = add_repo(ws.path(), c.path(), Some("alpha"), false)
        .unwrap_err()
        .to_string();
    assert!(dup.contains("alpha") && dup.contains("exists"), "{dup}");
    let r1 = add_repo(ws.path(), c.path(), Some("root1"), true).unwrap();
    assert_eq!(r1.role, "root");
    let two = add_repo(ws.path(), c.path(), Some("root2"), true)
        .unwrap_err()
        .to_string();
    assert!(two.contains("at most one"), "{two}");
}

#[test]
fn build_then_export_and_check_are_green_and_deterministic() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[(
        "src/a.py",
        "def f():\n    return g()\n\ndef g():\n    return 1\n",
    )]);
    add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    let r1 = build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    let r2 = build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    assert_eq!(r1.store_hash, r2.store_hash);
    assert_eq!(r1.nodes, 3); // file + f + g
    assert!(r1.edges >= 3); // 2 contains + 1 calls
    let j = export_json(ws.path(), Some("alpha")).unwrap();
    assert_eq!(j["schema"], 1);
    assert_eq!(j["repos"][0]["nodes"].as_array().unwrap().len(), 3);
    assert!(j["repos"][0]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "calls" && e["source_qn"] == "src/a.py::f"));
    let rep = check(ws.path()).unwrap();
    assert!(rep.ok(), "{:?}", rep.problems);
    assert_eq!(rep.runs_done, 1); // the second build was a no-op and reused the first run
    assert!(r2.reused);
}

#[test]
fn build_refuses_zero_files_and_marks_run_failed() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("README.txt", "nothing to parse\n")]);
    add_repo(ws.path(), c.path(), Some("empty"), false).unwrap();
    let err = build_repo(ws.path(), "empty", "reference", &BuildOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("0 files"), "{err}");
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    let repo = store.repo_by_name("empty").unwrap().unwrap();
    assert_eq!(
        store.latest_run(repo.id.unwrap()).unwrap().unwrap().status,
        comparegraph::domain::RunStatus::Failed
    );
    assert!(export_json(ws.path(), Some("empty")).is_err());
}

#[test]
fn check_marks_locations_stale_after_content_change() {
    // Plan 2 writes claims; here we insert one location row by hand to prove the stale rule on the store.
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    return 1\n")]);
    add_repo(ws.path(), c.path(), Some("alpha"), false).unwrap();
    build_repo(ws.path(), "alpha", "reference", &BuildOptions::default()).unwrap();
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    let repo = store.repo_by_name("alpha").unwrap().unwrap();
    let run = store.latest_done_run(repo.id.unwrap()).unwrap().unwrap();
    let f = store
        .nodes_of_run(run.id.unwrap())
        .unwrap()
        .into_iter()
        .find(|n| n.qualified_name == "src/a.py::f")
        .unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO family(name, definition_path) VALUES ('t','t.md')",
            [],
        )
        .unwrap();
    store.conn().execute("INSERT INTO feature(family_id, grp, slug, definition, counts_as_has, tier) VALUES (1,'g','x','d','c','basics')", []).unwrap();
    store.conn().execute("INSERT INTO claim(repo_id, feature_id, run_id, verdict, confidence, author, created_at) VALUES (?1,1,?2,'has','derived','test','2026-10-12T00:00:00Z')", rusqlite::params![repo.id.unwrap().0, run.id.unwrap().0]).unwrap();
    store.conn().execute("INSERT INTO location(claim_id, node_id, path, line_start, line_end, content_hash, resolved_at) VALUES (1, ?1, ?2, ?3, ?4, ?5, '2026-10-12T00:00:00Z')",
        rusqlite::params![f.id.unwrap().0, f.path, f.line_start, f.line_end, f.content_hash]).unwrap();
    drop(store);
    assert_eq!(check(ws.path()).unwrap().stale_locations, 0);
    std::fs::write(c.path().join("src/a.py"), "def f():\n    return 2\n").unwrap();
    let rep = check(ws.path()).unwrap();
    assert_eq!(rep.stale_locations, 1);
    assert!(!rep.ok());
}

#[test]
fn store_error_after_the_provider_marks_the_run_failed() {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[("src/a.py", "def f():\n    pass\n")]);
    add_repo(ws.path(), c.path(), Some("a"), false).unwrap();
    // deterministic fault: node inserts abort once the run row exists
    let store = comparegraph::store::Store::open(ws.path()).unwrap();
    store
        .conn()
        .execute_batch(
            "CREATE TRIGGER boom BEFORE INSERT ON node BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    let err = build_repo(ws.path(), "a", "reference", &BuildOptions::default())
        .unwrap_err()
        .to_string();
    assert!(err.contains("boom"), "{err}");
    let repo = store.repo_by_name("a").unwrap().unwrap();
    let run = store.latest_run(repo.id.unwrap()).unwrap().unwrap();
    assert_eq!(run.status, comparegraph::domain::RunStatus::Failed);
    assert!(
        run.limitations
            .iter()
            .any(|l| l.code == "store_error" && l.detail.contains("boom")),
        "{:?}",
        run.limitations
    );
}

#[test]
fn communities_cover_every_file_and_are_deterministic() {
    use comparegraph::domain::NodeKind;
    use comparegraph::store::Store;
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[
        ("a.py", "def one():\n    pass\n"),
        ("b.py", "def two():\n    pass\n"),
        ("c.py", "def three():\n    pass\n"),
    ]);
    add_repo(ws.path(), c.path(), Some("r"), false).unwrap();
    let r1 = build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let r2 = build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    assert_eq!(r1.communities, 3);
    assert_eq!(r2.communities, 3);
    let store = Store::open(ws.path()).unwrap();
    let summ = |run| {
        let cs = store.communities_of_run(run).unwrap();
        cs.iter()
            .map(|c| (c.label.clone(), c.members.len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(summ(r1.run), summ(r2.run));
    let files = store
        .nodes_of_run(r1.run)
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == NodeKind::File)
        .count();
    let cs = store.communities_of_run(r1.run).unwrap();
    assert_eq!(files, 3);
    assert!(cs.iter().all(|c| c.cohesion.is_none()));
    assert_eq!(
        cs.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
        ["a.py", "b.py", "c.py"]
    );
}

#[test]
fn linked_files_form_one_community() {
    use comparegraph::store::Store;
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[
        (
            "pkg/a.py",
            "from . import b\n\ndef f():\n    return b.g()\n",
        ),
        ("pkg/b.py", "def g():\n    return 1\n"),
        ("lone.py", "def z():\n    pass\n"),
    ]);
    add_repo(ws.path(), c.path(), Some("r"), false).unwrap();
    let r = build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let store = Store::open(ws.path()).unwrap();
    let cs = store.communities_of_run(r.run).unwrap();
    let labels: Vec<_> = cs.iter().map(|c| c.label.as_str()).collect();
    eprintln!("{labels:?}");
    assert_eq!(r.communities, 2, "{labels:?}");
    assert!(labels.contains(&"lone.py"));
}

mod search_tests {
    use super::*;
    use comparegraph::application::search::{search, SearchQuery};
    use comparegraph::domain::NodeKind;

    fn q(t: &str) -> SearchQuery {
        SearchQuery {
            text: t.into(),
            repo: None,
            kind: None,
            limit: 50,
            offset: 0,
        }
    }
    fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        let ws = tempfile::tempdir().unwrap();
        let a = clone_with(&[(
            "src/a.py",
            "def parse_config():\n    pass\n\nclass ParserThing:\n    pass\n",
        )]);
        let b = clone_with(&[("lib/b.py", "def parse_other():\n    pass\n")]);
        add_repo(ws.path(), a.path(), Some("alpha"), false).unwrap();
        add_repo(ws.path(), b.path(), Some("beta"), false).unwrap();
        for r in ["alpha", "beta"] {
            build_repo(
                ws.path(),
                r,
                "reference",
                &BuildOptions {
                    max_file_bytes: 1_048_576,
                    max_files: 1000,
                    ..BuildOptions::default()
                },
            )
            .unwrap();
        }
        (ws, a, b)
    }

    #[test]
    fn finds_by_prefix_and_filters() {
        let (ws, _a, _b) = setup();
        let p = search(ws.path(), &q("parse_con")).unwrap();
        assert_eq!(p.items.len(), 1, "{:?}", p.items);
        assert_eq!(p.items[0].repo, "alpha");
        assert_eq!(p.items[0].label, "candidate");
        assert!(p.items[0].community.is_some());
        let all = search(ws.path(), &q("parse")).unwrap();
        assert!(all.total >= 2);
        let mut f = q("parse");
        f.repo = Some("beta".into());
        let only = search(ws.path(), &f).unwrap();
        assert!(only.items.iter().all(|h| h.repo == "beta") && only.total == 1);
        let mut k = q("parse");
        k.kind = Some(NodeKind::Class);
        assert!(search(ws.path(), &k)
            .unwrap()
            .items
            .iter()
            .all(|h| h.kind == "class"));
    }

    #[test]
    fn paginates() {
        let (ws, _a, _b) = setup();
        let mut s = q("parse");
        s.limit = 1;
        let p1 = search(ws.path(), &s).unwrap();
        assert_eq!(p1.next.as_deref(), Some("1"));
        s.offset = 1;
        let p2 = search(ws.path(), &s).unwrap();
        assert_eq!(p2.items.len(), 1);
        assert_eq!(p1.total, p2.total);
        assert_ne!(p1.items[0].qualified_name, p2.items[0].qualified_name);
    }

    #[test]
    fn fts_syntax_does_not_error_and_empty_does() {
        let (ws, _a, _b) = setup();
        for t in ["foo\"bar", "a*", "-x", "x:y", "(a)", "AND", "NEAR(a b)"] {
            search(ws.path(), &q(t)).unwrap_or_else(|e| panic!("{t}: {e}"));
        }
        assert!(search(ws.path(), &q("*-:")).is_err());
    }

    #[test]
    fn old_runs_are_not_returned() {
        let ws = tempfile::tempdir().unwrap();
        let a = clone_with(&[("a.py", "def oldname():\n    pass\n")]);
        add_repo(ws.path(), a.path(), Some("alpha"), false).unwrap();
        let o = BuildOptions {
            max_file_bytes: 1_048_576,
            max_files: 1000,
            ..BuildOptions::default()
        };
        build_repo(ws.path(), "alpha", "reference", &o).unwrap();
        std::fs::write(a.path().join("a.py"), "def newname():\n    pass\n").unwrap();
        build_repo(ws.path(), "alpha", "reference", &o).unwrap();
        assert_eq!(search(ws.path(), &q("oldname")).unwrap().total, 0);
        assert_eq!(search(ws.path(), &q("newname")).unwrap().total, 1);
    }
}

fn shown_setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let c = clone_with(&[(
        "m.py",
        "def helper():\n    return 1\n\n\ndef main():\n    return helper()\n",
    )]);
    add_repo(ws.path(), c.path(), Some("r"), false).unwrap();
    build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    (ws, c)
}
fn qn_of(ws: &std::path::Path, name: &str) -> String {
    use comparegraph::store::Store;
    let s = Store::open(ws).unwrap();
    let run = s
        .latest_done_run(s.repo_by_name("r").unwrap().unwrap().id.unwrap())
        .unwrap()
        .unwrap();
    s.nodes_of_run(run.id.unwrap())
        .unwrap()
        .into_iter()
        .find(|n| n.name == name && n.kind == comparegraph::domain::NodeKind::Function)
        .unwrap()
        .qualified_name
}

#[test]
fn show_compact_and_full() {
    use comparegraph::application::show::show;
    let (ws, _c) = shown_setup();
    let qn = qn_of(ws.path(), "helper");
    let s = show(ws.path(), "r", &qn, false).unwrap();
    assert!(s.lines.is_none());
    assert_eq!(s.lane, "take");
    assert!(s.licence.is_some());
    assert!(!s.stale);
    assert_eq!(s.confidence, "observed");
    assert!(s.community.is_some());
    assert_eq!(s.callers, vec![qn_of(ws.path(), "main")]);
    let f = show(ws.path(), "r", &qn, true).unwrap();
    assert_eq!(f.lines.unwrap(), vec!["def helper():", "    return 1"]);
    let m = show(ws.path(), "r", &qn_of(ws.path(), "main"), false).unwrap();
    assert_eq!(m.callees, vec![qn]);
}

#[test]
fn show_is_stale_when_the_file_changed() {
    use comparegraph::application::show::show;
    let (ws, c) = shown_setup();
    let qn = qn_of(ws.path(), "helper");
    std::fs::write(
        c.path().join("m.py"),
        "def helper():\n    return 2\n\n\ndef main():\n    return helper()\n",
    )
    .unwrap();
    let s = show(ws.path(), "r", &qn, true).unwrap();
    assert!(s.stale);
    assert_eq!(s.confidence, "unknown");
    assert_eq!(s.lines.unwrap(), vec!["def helper():", "    return 2"]);
    std::fs::remove_file(c.path().join("m.py")).unwrap();
    let g = show(ws.path(), "r", &qn, true).unwrap();
    assert!(g.stale && g.lines.is_none());
}

#[test]
fn show_unknown_symbol_is_not_found() {
    use comparegraph::application::show::show;
    let (ws, _c) = shown_setup();
    let e = show(ws.path(), "r", "nope::x", false).unwrap_err();
    assert_eq!(e.code(), "not_found");
    assert_eq!(
        show(ws.path(), "zz", "x", false).unwrap_err().code(),
        "not_found"
    );
}

mod vault_tests {
    use super::*;
    use comparegraph::application::vault::export_vault;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        let ws = tempfile::tempdir().unwrap();
        let a = clone_with(&[
            (
                "src/auth/login.py",
                "import src.auth.util\n\ndef login():\n    check()\n\ndef check():\n    pass\n",
            ),
            ("src/auth/util.py", "def helper():\n    pass\n"),
            ("other/lone.py", "class Lone:\n    pass\n"),
        ]);
        add_repo(ws.path(), a.path(), Some("alpha"), false).unwrap();
        build_repo(
            ws.path(),
            "alpha",
            "reference",
            &BuildOptions {
                max_file_bytes: 1_048_576,
                max_files: 1000,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        (ws, a)
    }
    fn tree(dir: &Path) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        fn walk(base: &Path, d: &Path, m: &mut BTreeMap<String, String>) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(base, &p, m);
                } else {
                    let b = std::fs::read(&p).unwrap();
                    m.insert(
                        p.strip_prefix(base).unwrap().to_string_lossy().into(),
                        hex::encode(Sha256::digest(&b)),
                    );
                }
            }
        }
        walk(dir, dir, &mut m);
        m
    }

    #[test]
    fn layout_and_front_matter() {
        let (ws, _c) = setup();
        let out = tempfile::tempdir().unwrap();
        let o = out.path().join("v");
        let rep = export_vault(ws.path(), &o).unwrap();
        let t = tree(&o);
        for f in ["index.md", "llms.txt", "AGENTS.md", "repos/alpha.md"] {
            assert!(t.contains_key(f), "{f} in {:?}", t.keys());
        }
        assert!(t.keys().any(|k| k.starts_with("communities/alpha/")));
        assert!(t.keys().any(|k| k.starts_with("symbols/alpha/")));
        assert_eq!(rep.files, t.len());
        let idx = std::fs::read_to_string(o.join("index.md")).unwrap();
        assert!(idx.contains("generated_by: comparegraph ") && idx.contains("[[repos/alpha]]"));
        for k in t.keys().filter(|k| k.ends_with(".md") && *k != "AGENTS.md") {
            let s = std::fs::read_to_string(o.join(k)).unwrap();
            let mut ls = s.lines();
            assert_eq!(ls.next(), Some("---"), "{k}");
            let mut closed = false;
            for l in ls.by_ref() {
                if l == "---" {
                    closed = true;
                    break;
                }
                assert!(l.contains(": "), "{k}: {l}");
            }
            assert!(closed, "{k}");
        }
        let repo = std::fs::read_to_string(o.join("repos/alpha.md")).unwrap();
        assert!(repo.contains("type: \"repo\"") || repo.contains("type: repo"));
        assert!(repo.contains("[[communities/alpha/"));
        // every wikilink resolves to a file in the vault
        for k in t.keys().filter(|k| k.ends_with(".md")) {
            let s = std::fs::read_to_string(o.join(k)).unwrap();
            for part in s.split("[[").skip(1) {
                let target = part.split("]]").next().unwrap();
                assert!(o.join(format!("{target}.md")).exists(), "{k} -> {target}");
            }
        }
        // no stray temp dir
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 1);
    }

    #[test]
    fn two_exports_are_byte_identical() {
        let (ws, _c) = setup();
        let out = tempfile::tempdir().unwrap();
        export_vault(ws.path(), &out.path().join("a")).unwrap();
        export_vault(ws.path(), &out.path().join("b")).unwrap();
        assert_eq!(tree(&out.path().join("a")), tree(&out.path().join("b")));
    }

    #[test]
    fn stale_notes_do_not_survive_and_foreign_dirs_are_refused() {
        let (ws, _c) = setup();
        let out = tempfile::tempdir().unwrap();
        let o = out.path().join("v");
        export_vault(ws.path(), &o).unwrap();
        std::fs::write(o.join("symbols/old-note.md"), "stale").unwrap();
        export_vault(ws.path(), &o).unwrap();
        assert!(!o.join("symbols/old-note.md").exists());
        // a dir that is not a comparegraph vault is left untouched
        let f = out.path().join("mine");
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(f.join("keep.txt"), "x").unwrap();
        let e = export_vault(ws.path(), &f).unwrap_err().to_string();
        assert!(e.contains("mine"), "{e}");
        assert!(f.join("keep.txt").exists());
        // an index.md without the marker is also foreign
        std::fs::write(f.join("index.md"), "---\ntype: index\n---\n").unwrap();
        assert!(export_vault(ws.path(), &f).is_err() && f.join("keep.txt").exists());
        let leftovers: Vec<_> = std::fs::read_dir(out.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            leftovers.iter().all(|n| !n.contains(".tmp-")),
            "{leftovers:?}"
        );
    }

    #[test]
    fn refuses_empty_store() {
        let ws = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        assert!(export_vault(ws.path(), &out.path().join("v")).is_err());
        assert!(!out.path().join("v").exists());
    }

    fn with_history() -> (tempfile::TempDir, tempfile::TempDir) {
        use comparegraph::application::{
            claim::{claim, ClaimInput},
            decide::decide,
            family::add_family,
        };
        let (ws, c) = setup();
        let pack = tempfile::tempdir().unwrap();
        std::fs::write(pack.path().join("rows.md"), "---\nfamily: fam\ndescription: d\n---\n| id | group | feature | definition | counts_as_has | tier | weight |\n|---|---|---|---|---|---|---|\n| auth.login | auth | Login | Users log in | a login fn | basics | 2 |\n").unwrap();
        add_family(ws.path(), pack.path()).unwrap();
        let at = vec!["src/auth/login.py:3-4".to_string()];
        for v in ["partial", "has"] {
            claim(
                ws.path(),
                &ClaimInput {
                    repo: "alpha",
                    feature: "auth.login",
                    family: None,
                    verdict: v,
                    author: "me",
                    at: &at,
                    searched: None,
                    note: None,
                    skip_equal: false,
                },
            )
            .unwrap();
        }
        decide(ws.path(), "alpha", "adopt", "good fit", "me", None, None).unwrap();
        (ws, c)
    }

    #[test]
    fn every_note_has_a_description_and_history_notes_list_the_trail() {
        let (ws, _c) = with_history();
        let out = tempfile::tempdir().unwrap();
        let o = out.path().join("v");
        export_vault(ws.path(), &o).unwrap();
        let t = tree(&o);
        for k in t.keys().filter(|k| k.ends_with(".md") && *k != "AGENTS.md") {
            let s = std::fs::read_to_string(o.join(k)).unwrap();
            let fm: Vec<&str> = s.lines().skip(1).take_while(|l| *l != "---").collect();
            let d = fm
                .iter()
                .find(|l| l.starts_with("description: \""))
                .unwrap_or_else(|| panic!("{k}: no description"));
            assert!(d.len() - "description: ".len() <= 162 + 8, "{k}: {d}");
        }
        let h = std::fs::read_to_string(o.join("history/alpha.md")).unwrap();
        assert!(
            h.contains("type: history") && h.contains("2 claim rows, 1 decision rows"),
            "{h}"
        );
        let (a, b) = (h.find("- #1 ").unwrap(), h.find("- #2 ").unwrap());
        assert!(a < b);
        let l1 = h.lines().find(|l| l.starts_with("- #1 ")).unwrap();
        assert!(
            l1.contains("superseded by #2") && l1.contains("[[features/fam/"),
            "{l1}"
        );
        assert!(h
            .lines()
            .find(|l| l.starts_with("- #2 "))
            .unwrap()
            .contains("current"));
        assert!(
            h.contains("src/auth/login.py:3-4") && h.contains("adopt") && h.contains("good fit")
        );
        let repo = std::fs::read_to_string(o.join("repos/alpha.md")).unwrap();
        assert!(repo.contains("[[history/alpha]]") && repo.contains("decision adopt"));
        assert!(!t.contains_key("history/none.md"));
        let ag = std::fs::read_to_string(o.join("AGENTS.md")).unwrap();
        assert!(ag.contains("relate \u{2192} map \u{2192} claim"));
        for k in t.keys().filter(|k| k.ends_with(".md")) {
            let s = std::fs::read_to_string(o.join(k)).unwrap();
            for part in s.split("[[").skip(1) {
                let target = part.split("]]").next().unwrap();
                assert!(o.join(format!("{target}.md")).exists(), "{k} -> {target}");
            }
        }
        export_vault(ws.path(), &out.path().join("w")).unwrap();
        assert_eq!(t, tree(&out.path().join("w")));
    }

    #[test]
    fn slugs() {
        use comparegraph::application::vault::slug;
        assert_eq!(slug("src/Auth::Login.ts"), "src-auth-login-ts");
        assert_eq!(slug("--a  b--"), "a-b");
        assert_eq!(slug(&"a".repeat(200)).len(), 80);
    }
}

#[test]
fn show_records_truncated_callers_with_the_total_and_honours_max_callers() {
    use comparegraph::application::show::{show_with, DEFAULT_MAX_CALLERS, MAX_CALLERS_LIMIT};
    let ws = tempfile::tempdir().unwrap();
    let mut src = String::from("def target():\n    return 1\n\n\n");
    for i in 0..60 {
        src.push_str(&format!("def caller_{i:02}():\n    return target()\n\n\n"));
    }
    let c = clone_with(&[("m.py", src.as_str())]);
    add_repo(ws.path(), c.path(), Some("r"), false).unwrap();
    build_repo(ws.path(), "r", "reference", &BuildOptions::default()).unwrap();
    let qn = qn_of(ws.path(), "target");
    let s = show_with(ws.path(), "r", &qn, false, DEFAULT_MAX_CALLERS).unwrap();
    assert_eq!(s.callers.len(), 50);
    assert_eq!(s.callers_total, 60);
    let l = s.limitations.iter().find(|l| l.code == "truncated").expect("truncated limitation");
    assert!(l.detail.contains("callers_total=60"), "{}", l.detail);
    let all = show_with(ws.path(), "r", &qn, false, 100).unwrap();
    assert_eq!(all.callers.len(), 60);
    assert!(all.limitations.is_empty());
    assert_eq!(show_with(ws.path(), "r", &qn, false, 0).unwrap_err().code(), show_with(ws.path(), "r", &qn, false, MAX_CALLERS_LIMIT + 1).unwrap_err().code());
    // a plain `show` keeps the default cap
    assert_eq!(comparegraph::application::show::show(ws.path(), "r", &qn, false).unwrap().callers.len(), 50);
}
