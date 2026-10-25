//! `--root-map FROM=TO`: a store copied to another machine keeps its absolute clone paths; the map re-roots them on read.
use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use std::process::Command as Sys;

fn cc(ws: &Path) -> Command {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.arg("--root").arg(ws).env_remove("COMPAREGRAPH_ROOT_MAP");
    c
}
fn json(c: &mut Command) -> Value {
    let out = c.output().unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)))
}
/// A built workspace whose clone then moves to another folder, as a copied store's clone does.
fn moved() -> (tempfile::TempDir, tempfile::TempDir, String, String) {
    let ws = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let from = base.path().join("old");
    std::fs::create_dir_all(from.join("src")).unwrap();
    std::fs::write(from.join("src/a.ts"), "export function f() { return g(); }\nexport function g() { return 1; }\n").unwrap();
    std::fs::write(from.join("LICENSE"), "Apache License\nVersion 2.0\n").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "."], vec!["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "x"]] {
        Sys::new("git").arg("-C").arg(&from).args(&args).status().unwrap();
    }
    cc(ws.path()).args(["add"]).arg(&from).args(["--name", "a", "--as-root"]).assert().success();
    cc(ws.path()).args(["build", "a"]).assert().success();
    let to = base.path().join("new");
    std::fs::rename(&from, &to).unwrap();
    let (f, t) = (from.to_string_lossy().into_owned(), to.to_string_lossy().into_owned());
    (ws, base, f, t)
}

#[test]
fn doctor_names_repos_whose_stored_path_is_missing_here() {
    let (ws, _base, from, _to) = moved();
    let d = json(cc(ws.path()).arg("doctor"));
    let un = d["data"]["unmapped_roots"].as_array().unwrap_or_else(|| panic!("no unmapped_roots: {d}"));
    assert_eq!(un.len(), 1, "{d}");
    assert_eq!(un[0]["name"], "a");
    assert_eq!(un[0]["path"], Value::String(from));
}

#[test]
fn root_map_re_roots_the_stored_paths_on_read() {
    let (ws, _base, from, to) = moved();
    let map = format!("{from}={to}");
    let d = json(cc(ws.path()).args(["--root-map", &map, "doctor"]));
    assert_eq!(d["data"]["unmapped_roots"].as_array().unwrap().len(), 0, "{d}");
    // the clone's lines are read through the mapped path: the symbol is current with the map, stale without it
    let show = |extra: &[&str]| json(cc(ws.path()).args(extra).args(["show", "a", "src/a.ts::f"]))["data"]["stale"].clone();
    assert_eq!(show(&["--root-map", &map]), Value::Bool(false));
    assert_eq!(show(&[]), Value::Bool(true));
}

#[test]
fn root_map_applies_at_a_path_boundary_only_and_the_env_form_works() {
    let (ws, _base, from, to) = moved();
    // `<from>x` is not a prefix of `<from>` at a path boundary: nothing is re-rooted
    let d = json(cc(ws.path()).args(["--root-map", &format!("{from}x={to}"), "doctor"]));
    assert_eq!(d["data"]["unmapped_roots"].as_array().unwrap().len(), 1, "{d}");
    let d = json(cc(ws.path()).env("COMPAREGRAPH_ROOT_MAP", format!("{from}={to}")).arg("doctor"));
    assert_eq!(d["data"]["unmapped_roots"].as_array().unwrap().len(), 0, "{d}");
}
