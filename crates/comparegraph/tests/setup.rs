//! `setup` and the `doctor` registrations, always under a temp HOME / XDG_CONFIG_HOME / project dir.
use assert_cmd::Command;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const BIN: &str = "/opt/cg/bin/comparegraph";
struct Env {
    home: tempfile::TempDir,
    xdg: tempfile::TempDir,
    proj: tempfile::TempDir,
    ws: tempfile::TempDir,
}
impl Env {
    fn new() -> Env {
        Env {
            home: tempfile::tempdir().unwrap(),
            xdg: tempfile::tempdir().unwrap(),
            proj: tempfile::tempdir().unwrap(),
            ws: tempfile::tempdir().unwrap(),
        }
    }
    fn run(&self, bin: &str, args: &[&str]) -> Value {
        self.run_with(Some(bin), args)
    }
    fn run_with(&self, bin: Option<&str>, args: &[&str]) -> Value {
        let mut c = Command::cargo_bin("comparegraph").unwrap();
        c.env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.xdg.path())
            .env("COMPAREGRAPH_NOW", "1700000000")
            .env_remove("COMPAREGRAPH_BIN");
        if let Some(b) = bin {
            c.env("COMPAREGRAPH_BIN", b);
        }
        c
            .current_dir(self.proj.path())
            .arg("--root")
            .arg(self.ws.path())
            .args(args);
        let out = c.output().unwrap();
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&out.stdout)))
    }
    fn setup(&self, args: &[&str]) -> Value {
        let mut a = vec!["setup"];
        a.extend_from_slice(args);
        self.run(BIN, &a)
    }
    fn ws_str(&self) -> String {
        std::fs::canonicalize(self.ws.path()).unwrap().to_string_lossy().into_owned()
    }
    fn user(&self, rel: &str) -> PathBuf {
        self.home.path().join(rel)
    }
    fn files(&self) -> Vec<String> {
        let mut v = Vec::new();
        for d in [self.home.path(), self.xdg.path(), self.proj.path()] {
            walk(d, &mut v);
        }
        v.sort();
        v
    }
}
fn walk(d: &Path, out: &mut Vec<String>) {
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out)
        } else {
            out.push(p.to_string_lossy().into_owned())
        }
    }
}
fn row<'a>(v: &'a Value, client: &str) -> &'a Value {
    v["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == client).unwrap_or_else(|| panic!("no row for {client}: {v}"))
}
fn rowk<'a>(v: &'a Value, client: &str, kind: &str) -> &'a Value {
    v["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == client && r["kind"] == kind).unwrap_or_else(|| panic!("no {kind} row for {client}: {v}"))
}
fn kinds(v: &Value, kind: &str) -> Vec<String> {
    v["data"]["clients"].as_array().unwrap().iter().filter(|r| r["kind"] == kind).map(|r| r["client"].as_str().unwrap().to_string()).collect()
}
fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn baks(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains(".bak-")).collect()
}

#[test]
fn dry_run_is_the_default_and_writes_nothing() {
    let e = Env::new();
    for d in [".claude", ".codex", ".cursor", ".gemini", ".hermes"] {
        std::fs::create_dir_all(e.user(d)).unwrap();
    }
    std::fs::write(e.user(".claude.json"), "{\"numStartups\": 3}\n").unwrap();
    let before = e.files();
    let before_claude = std::fs::read_to_string(e.user(".claude.json")).unwrap();
    let v = e.setup(&[]);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["dry_run"], true);
    assert_eq!(row(&v, "claude")["state"], "missing");
    assert_eq!(row(&v, "claude")["action"], "add");
    assert_eq!(row(&v, "claude")["written"], false);
    assert_eq!(row(&v, "codex")["action"], "create");
    assert_eq!(e.files(), before, "a dry run must not create or change a file");
    assert_eq!(std::fs::read_to_string(e.user(".claude.json")).unwrap(), before_claude);
}

#[test]
fn all_skips_clients_that_are_not_installed() {
    let e = Env::new();
    std::fs::create_dir_all(e.user(".gemini")).unwrap();
    let v = e.setup(&["--apply"]);
    assert_eq!(row(&v, "gemini")["written"], true);
    assert_eq!(row(&v, "codex")["action"], "skip");
    assert_eq!(row(&v, "codex")["written"], false);
    assert!(!e.user(".codex").exists());
    assert!(!e.user(".claude.json").exists());
}

#[test]
fn apply_writes_each_clients_documented_shape() {
    let e = Env::new();
    let ws = e.ws_str();
    let args = json!(["--root", ws, "mcp"]);
    for c in ["claude", "codex", "gemini", "opencode", "hermes"] {
        let v = e.setup(&["--client", c, "--apply"]);
        assert_eq!(row(&v, c)["written"], true, "{c}: {v}");
    }
    let v = e.setup(&["--client", "cursor", "--scope", "project", "--apply"]);
    assert_eq!(row(&v, "cursor")["written"], true, "{v}");
    assert_eq!(read_json(&e.user(".claude.json"))["mcpServers"]["comparegraph"], json!({"command": BIN, "args": args}));
    assert_eq!(read_json(&e.proj.path().join(".cursor/mcp.json"))["mcpServers"]["comparegraph"], json!({"command": BIN, "args": args}));
    assert_eq!(read_json(&e.user(".gemini/settings.json"))["mcpServers"]["comparegraph"], json!({"command": BIN, "args": args}));
    let oc = read_json(&e.xdg.path().join("opencode/opencode.json"));
    assert_eq!(oc["mcp"]["comparegraph"], json!({"type": "local", "command": [BIN, "--root", ws, "mcp"]}));
    let toml = std::fs::read_to_string(e.user(".codex/config.toml")).unwrap();
    assert_eq!(toml, format!("[mcp_servers.comparegraph]\ncommand = \"{BIN}\"\nargs = [\"--root\",\"{ws}\",\"mcp\"]\n"));
    let yaml = std::fs::read_to_string(e.user(".hermes/config.yaml")).unwrap();
    assert_eq!(yaml, format!("mcp_servers:\n  comparegraph:\n    command: \"{BIN}\"\n    args: [\"--root\",\"{ws}\",\"mcp\"]\n"));
    // 2-space indent
    assert!(std::fs::read_to_string(e.proj.path().join(".cursor/mcp.json")).unwrap().contains("\n  \"mcpServers\": {\n    \"comparegraph\""));
}

#[test]
fn apply_is_idempotent_and_backs_up_what_it_changes() {
    let e = Env::new();
    let original = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\"other\": {\"command\": \"x\"}}\n}\n";
    std::fs::write(e.user(".claude.json"), original).unwrap();
    let v = e.setup(&["--client", "claude", "--apply"]);
    assert_eq!(row(&v, "claude")["action"], "add");
    let j = read_json(&e.user(".claude.json"));
    assert_eq!(j["theme"], "dark");
    assert_eq!(j["mcpServers"]["other"], json!({"command": "x"}));
    assert_eq!(j["mcpServers"]["comparegraph"]["command"], BIN);
    let bak = baks(e.home.path());
    assert_eq!(bak.len(), 1, "{bak:?}");
    assert!(bak[0].starts_with(".claude.json.bak-"));
    assert_eq!(std::fs::read_to_string(e.home.path().join(&bak[0])).unwrap(), original);
    let after_first = std::fs::read_to_string(e.user(".claude.json")).unwrap();
    let v2 = e.setup(&["--client", "claude", "--apply"]);
    assert_eq!(row(&v2, "claude")["state"], "aligned");
    assert_eq!(row(&v2, "claude")["action"], "none");
    assert_eq!(row(&v2, "claude")["written"], false);
    assert_eq!(std::fs::read_to_string(e.user(".claude.json")).unwrap(), after_first);
    assert_eq!(baks(e.home.path()).len(), 1, "a second apply writes and backs up nothing");
}

#[test]
fn a_conflicting_entry_is_untouched_unless_forced() {
    let e = Env::new();
    let other = "{\"mcpServers\":{\"comparegraph\":{\"command\":\"/elsewhere/cg\",\"args\":[\"--root\",\"/other\",\"mcp\"]}}}\n";
    std::fs::write(e.user(".gemini/settings.json").parent().map(|p| { std::fs::create_dir_all(p).unwrap(); p.join("settings.json") }).unwrap(), other).unwrap();
    let v = e.setup(&["--client", "gemini", "--apply"]);
    assert_eq!(row(&v, "gemini")["state"], "conflict");
    assert_eq!(row(&v, "gemini")["action"], "skip");
    assert_eq!(row(&v, "gemini")["written"], false);
    assert_eq!(std::fs::read_to_string(e.user(".gemini/settings.json")).unwrap(), other);
    assert!(baks(&e.user(".gemini")).is_empty());
    assert_eq!(v["evidence"]["limitations"][0]["code"], "conflict");
    // doctor says conflict too
    let d = e.run(BIN, &["doctor"]);
    let cur = d["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == "gemini" && r["scope"] == "user").unwrap();
    assert_eq!(cur["state"], "conflict");
    let f = e.setup(&["--client", "gemini", "--apply", "--force"]);
    assert_eq!(row(&f, "gemini")["action"], "replace");
    assert_eq!(read_json(&e.user(".gemini/settings.json"))["mcpServers"]["comparegraph"]["command"], BIN);
    assert_eq!(baks(&e.user(".gemini")).len(), 1);
}

#[test]
fn a_different_binary_path_is_stale_and_setup_refreshes_it() {
    let e = Env::new();
    e.setup(&["--client", "gemini", "--apply"]);
    let d = e.run("/new/place/comparegraph", &["doctor"]);
    let g = d["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == "gemini" && r["scope"] == "user").unwrap();
    assert_eq!(g["state"], "stale");
    let v = e.run("/new/place/comparegraph", &["setup", "--client", "gemini", "--apply"]);
    assert_eq!(row(&v, "gemini")["action"], "skip", "stale is a conflict: plain apply leaves it");
    assert_eq!(read_json(&e.user(".gemini/settings.json"))["mcpServers"]["comparegraph"]["command"], BIN);
    let v = e.run("/new/place/comparegraph", &["setup", "--client", "gemini", "--apply", "--force"]);
    assert_eq!(row(&v, "gemini")["action"], "replace");
    assert_eq!(read_json(&e.user(".gemini/settings.json"))["mcpServers"]["comparegraph"]["command"], "/new/place/comparegraph");
    let d = e.run("/new/place/comparegraph", &["doctor"]);
    let g = d["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == "gemini" && r["scope"] == "user").unwrap();
    assert_eq!(g["state"], "aligned");
}

#[test]
fn doctor_reports_missing_aligned_and_never_writes() {
    let e = Env::new();
    let before = e.files();
    let d = e.run(BIN, &["doctor"]);
    assert_eq!(d["ok"], true);
    let rows = d["data"]["clients"].as_array().unwrap();
    assert_eq!(rows.iter().filter(|r| r["scope"] == "user").count(), 10);
    assert!(rows.iter().filter(|r| r["client"] != "cursor").all(|r| r["state"] == "missing"));
    assert!(rows.iter().any(|r| r["client"] == "cursor" && r["scope"] == "user" && r["state"] == "unverified"));
    assert_eq!(e.files(), before);
    e.setup(&["--client", "opencode", "--apply"]);
    let d = e.run(BIN, &["doctor"]);
    let oc = d["data"]["clients"].as_array().unwrap().iter().find(|r| r["client"] == "opencode").unwrap();
    assert_eq!(oc["state"], "aligned");
}

#[test]
fn toml_and_yaml_are_appended_never_rewritten() {
    let e = Env::new();
    let ws = e.ws_str();
    let toml = "# my codex\nmodel = \"gpt\"\n\n[mcp_servers.other]\ncommand = \"o\"\n";
    std::fs::create_dir_all(e.user(".codex")).unwrap();
    std::fs::write(e.user(".codex/config.toml"), toml).unwrap();
    e.setup(&["--client", "codex", "--apply"]);
    let after = std::fs::read_to_string(e.user(".codex/config.toml")).unwrap();
    assert!(after.starts_with(toml), "existing text is a byte-for-byte prefix:\n{after}");
    assert!(after.contains(&format!("[mcp_servers.comparegraph]\ncommand = \"{BIN}\"")));
    assert_eq!(row(&e.setup(&["--client", "codex"]), "codex")["state"], "aligned");

    let yaml = "model: x\nmcp_servers:\n  fs:\n    command: npx\n    args: [\"-y\"]\nother: 1\n";
    std::fs::create_dir_all(e.user(".hermes")).unwrap();
    std::fs::write(e.user(".hermes/config.yaml"), yaml).unwrap();
    e.setup(&["--client", "hermes", "--apply"]);
    let after = std::fs::read_to_string(e.user(".hermes/config.yaml")).unwrap();
    assert_eq!(after, format!("model: x\nmcp_servers:\n  comparegraph:\n    command: \"{BIN}\"\n    args: [\"--root\",\"{ws}\",\"mcp\"]\n  fs:\n    command: npx\n    args: [\"-y\"]\nother: 1\n"));
    assert_eq!(row(&e.setup(&["--client", "hermes"]), "hermes")["state"], "aligned");
    // a conflicting text entry is left alone, and --force replaces just that table
    let conflicting = "mcp_servers:\n  comparegraph:\n    command: \"/x\"\n    args: [\"mcp\"]\n  fs:\n    command: npx\n";
    std::fs::write(e.user(".hermes/config.yaml"), conflicting).unwrap();
    let v = e.setup(&["--client", "hermes", "--apply"]);
    assert_eq!(row(&v, "hermes")["state"], "conflict");
    assert_eq!(std::fs::read_to_string(e.user(".hermes/config.yaml")).unwrap(), conflicting);
    e.setup(&["--client", "hermes", "--apply", "--force"]);
    let after = std::fs::read_to_string(e.user(".hermes/config.yaml")).unwrap();
    assert!(after.contains("  fs:\n    command: npx\n") && after.contains(&format!("command: \"{BIN}\"")) && !after.contains("\"/x\""), "{after}");
}

#[test]
fn an_unreadable_json_config_is_never_overwritten_even_with_force() {
    let e = Env::new();
    std::fs::create_dir_all(e.user(".gemini")).unwrap();
    std::fs::write(e.user(".gemini/settings.json"), "{ not json").unwrap();
    let v = e.setup(&["--client", "gemini", "--apply", "--force"]);
    assert_eq!(row(&v, "gemini")["state"], "conflict");
    assert_eq!(row(&v, "gemini")["written"], false);
    assert_eq!(std::fs::read_to_string(e.user(".gemini/settings.json")).unwrap(), "{ not json");
}

#[test]
fn project_scope_writes_into_the_current_directory() {
    let e = Env::new();
    let v = e.setup(&["--client", "claude", "--scope", "project", "--apply"]);
    assert_eq!(row(&v, "claude")["written"], true);
    assert_eq!(read_json(&e.proj.path().join(".mcp.json"))["mcpServers"]["comparegraph"]["command"], BIN);
    assert!(!e.user(".claude.json").exists());
    let bad = e.setup(&["--client", "hermes", "--scope", "project"]);
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["error"]["code"], "invalid_arguments");
    let bad = e.setup(&["--client", "nope"]);
    assert_eq!(bad["error"]["code"], "invalid_arguments");
}

#[test]
fn cursor_user_scope_is_unverified_and_skipped() {
    let e = Env::new();
    std::fs::create_dir_all(e.user(".cursor")).unwrap();
    let v = e.setup(&["--client", "cursor", "--apply"]);
    assert_eq!(row(&v, "cursor")["state"], "unverified");
    assert_eq!(row(&v, "cursor")["action"], "skip");
    assert_eq!(row(&v, "cursor")["written"], false);
    assert!(!e.user(".cursor/mcp.json").exists());
    let v = e.setup(&["--apply"]);
    assert_eq!(row(&v, "cursor")["state"], "unverified");
    let v = e.setup(&["--client", "cursor", "--scope", "project", "--apply"]);
    assert_eq!(row(&v, "cursor")["written"], true);
}

#[test]
fn dev_builds_are_never_registered() {
    let e = Env::new();
    let v = e.run_with(None, &["setup", "--client", "gemini", "--apply"]);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(v["error"]["message"].as_str().unwrap().contains("build directory"));
    assert!(!e.user(".gemini").exists());
    let v = e.run_with(None, &["setup", "--client", "gemini", "--bin", "/opt/x/comparegraph"]);
    assert_eq!(v["ok"], true);
    let d = e.run_with(None, &["doctor"]);
    assert!(d["evidence"]["limitations"].as_array().unwrap().iter().any(|l| l["code"] == "dev_build"));
    let d = e.run_with(None, &["doctor", "--bin", "/opt/x/comparegraph"]);
    assert!(d["evidence"]["limitations"].as_array().unwrap().iter().all(|l| l["code"] != "dev_build"));
}

#[test]
fn json_round_trip_keeps_key_order_and_number_text_except_exponent_sign() {
    let e = Env::new();
    std::fs::create_dir_all(e.user(".gemini")).unwrap();
    std::fs::write(e.user(".gemini/settings.json"), "{\n  \"zeta\": 100000000000000000000001,\n  \"alpha\": 1.50,\n  \"mcpServers\": {\"zz\": {}, \"aa\": {}}\n}\n").unwrap();
    e.setup(&["--client", "gemini", "--apply"]);
    let t = std::fs::read_to_string(e.user(".gemini/settings.json")).unwrap();
    assert!(t.contains("\"zeta\": 100000000000000000000001") && t.contains("\"alpha\": 1.50"), "{t}");
    assert!(t.find("zeta").unwrap() < t.find("alpha").unwrap() && t.find("alpha").unwrap() < t.find("mcpServers").unwrap());
    assert!(t.find("\"zz\"").unwrap() < t.find("\"aa\"").unwrap() && t.find("\"aa\"").unwrap() < t.find("comparegraph").unwrap());
}

#[test]
fn a_symlinked_config_stays_a_symlink_and_its_target_is_updated() {
    let e = Env::new();
    let real = e.proj.path().join("dotfiles/claude.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "{\"a\": 1}\n").unwrap();
    std::os::unix::fs::symlink(&real, e.user(".claude.json")).unwrap();
    e.setup(&["--client", "claude", "--apply"]);
    assert!(std::fs::symlink_metadata(e.user(".claude.json")).unwrap().file_type().is_symlink());
    assert_eq!(read_json(&real)["mcpServers"]["comparegraph"]["command"], BIN);
    let bak = baks(real.parent().unwrap());
    assert_eq!(bak.len(), 1, "{bak:?}");
    assert_eq!(std::fs::read_to_string(real.parent().unwrap().join(&bak[0])).unwrap(), "{\"a\": 1}\n");
    assert!(baks(e.home.path()).is_empty());
}

#[test]
fn two_backups_in_the_same_second_do_not_collide() {
    let e = Env::new(); // the clock is fixed at 1700000000
    let orig = "{\"mcpServers\":{\"comparegraph\":{\"command\":\"/a\",\"args\":[]}}}\n";
    std::fs::create_dir_all(e.user(".gemini")).unwrap();
    std::fs::write(e.user(".gemini/settings.json"), orig).unwrap();
    e.setup(&["--client", "gemini", "--apply", "--force"]);
    let second = std::fs::read_to_string(e.user(".gemini/settings.json")).unwrap();
    e.run("/b/cg", &["setup", "--client", "gemini", "--apply", "--force"]);
    let mut b = baks(&e.user(".gemini"));
    b.sort();
    assert_eq!(b, vec!["settings.json.bak-1700000000", "settings.json.bak-1700000000-2"]);
    assert_eq!(std::fs::read_to_string(e.user(".gemini/settings.json.bak-1700000000")).unwrap(), orig);
    assert_eq!(std::fs::read_to_string(e.user(".gemini/settings.json.bak-1700000000-2")).unwrap(), second);
}

#[test]
fn a_relative_bin_is_made_absolute() {
    let e = Env::new();
    std::fs::create_dir_all(e.proj.path().join("rel")).unwrap();
    std::fs::write(e.proj.path().join("rel/cg"), "").unwrap();
    let v = e.run_with(None, &["setup", "--client", "gemini", "--bin", "./rel/cg", "--apply"]);
    let want = std::fs::canonicalize(e.proj.path().join("rel/cg")).unwrap();
    assert_eq!(v["data"]["bin"], want.to_string_lossy().as_ref());
    let v = e.run_with(None, &["setup", "--client", "gemini", "--bin", "./nope/cg"]);
    let got = v["data"]["bin"].as_str().unwrap();
    assert!(Path::new(got).is_absolute() && got.ends_with("nope/cg"), "{got}");
}

#[test]
fn crlf_and_a_missing_final_newline_are_preserved() {
    let e = Env::new();
    let ws = e.ws_str();
    std::fs::create_dir_all(e.user(".codex")).unwrap();
    std::fs::write(e.user(".codex/config.toml"), "model = \"x\"\r\n[mcp_servers.o]\r\ncommand = \"o\"").unwrap();
    e.setup(&["--client", "codex", "--apply"]);
    let t = std::fs::read_to_string(e.user(".codex/config.toml")).unwrap();
    assert_eq!(t, format!("model = \"x\"\r\n[mcp_servers.o]\r\ncommand = \"o\"\r\n\r\n[mcp_servers.comparegraph]\r\ncommand = \"{BIN}\"\r\nargs = [\"--root\",\"{ws}\",\"mcp\"]"));
    assert_eq!(row(&e.setup(&["--client", "codex"]), "codex")["state"], "aligned");
}

#[test]
fn quoted_yaml_keys_and_hash_inside_quoted_values() {
    let e = Env::new();
    let ws = e.ws_str();
    std::fs::create_dir_all(e.user(".hermes")).unwrap();
    std::fs::write(e.user(".hermes/config.yaml"), format!("mcp_servers:\n  \"comparegraph\":\n    command: \"{BIN}\"\n    args: [\"--root\",\"{ws}\",\"mcp\"]\n")).unwrap();
    assert_eq!(row(&e.setup(&["--client", "hermes"]), "hermes")["state"], "aligned");
    std::fs::create_dir_all(e.user(".codex")).unwrap();
    std::fs::write(e.user(".codex/config.toml"), format!("[mcp_servers.comparegraph]\ncommand = \"{BIN}\"\nargs = [\"--root\", \"{ws}\", \"mcp\"] # note # more\n")).unwrap();
    assert_eq!(row(&e.setup(&["--client", "codex"]), "codex")["state"], "aligned");
    let hashed = format!("{ws}/a #b");
    let _ = hashed; // a quoted value with " #" is parsed as a string first
    std::fs::write(e.user(".codex/config.toml"), "[mcp_servers.comparegraph]\ncommand = \"/p #q/cg\"\nargs = [\"x\"]\n").unwrap();
    let v = e.setup(&["--client", "codex"]);
    assert_eq!(row(&v, "codex")["state"], "conflict");
}

#[test]
fn all_with_project_scope_and_the_opencode_jsonc_conflict() {
    let e = Env::new();
    std::fs::create_dir_all(e.proj.path().join(".codex")).unwrap();
    let v = e.setup(&["--scope", "project", "--apply"]);
    assert_eq!(row(&v, "codex")["written"], true);
    assert!(e.proj.path().join(".codex/config.toml").is_file());
    assert_eq!(row(&v, "claude")["action"], "skip");
    assert!(!e.proj.path().join(".mcp.json").exists());
    assert!(v["data"]["clients"].as_array().unwrap().iter().all(|r| r["client"] != "hermes"));
    // opencode.jsonc
    std::fs::create_dir_all(e.xdg.path().join("opencode")).unwrap();
    std::fs::write(e.xdg.path().join("opencode/opencode.jsonc"), "{ // c\n}\n").unwrap();
    let v = e.setup(&["--client", "opencode", "--apply", "--force"]);
    assert_eq!(row(&v, "opencode")["state"], "conflict");
    assert_eq!(row(&v, "opencode")["written"], false);
    assert!(!e.xdg.path().join("opencode/opencode.json").exists());
}

// ---------- rules block ----------

const OPEN: &str = "<!-- comparegraph -->";
const CLOSE: &str = "<!-- /comparegraph -->";

fn block_of(text: &str) -> &str {
    let a = text.find(OPEN).unwrap();
    let b = text.find(CLOSE).unwrap() + CLOSE.len();
    &text[a..b]
}

#[test]
fn rules_are_created_in_a_new_file_and_are_aligned_on_a_second_run() {
    let e = Env::new();
    let v = e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    let r = rowk(&v, "claude", "rules");
    assert_eq!((r["action"].as_str(), r["state"].as_str(), r["written"].as_bool(), r["scope"].as_str()), (Some("create"), Some("missing"), Some(true), Some("project")));
    let t = std::fs::read_to_string(e.proj.path().join("CLAUDE.md")).unwrap();
    assert!(t.starts_with("<!-- comparegraph -->\n## CompareGraph\n") && t.ends_with("<!-- /comparegraph -->\n"), "{t}");
    assert!(t.contains("`comparegraph_map`"));
    assert!(t.contains("Never run code inside a clone."));
    let v2 = e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    let r = rowk(&v2, "claude", "rules");
    assert_eq!((r["state"].as_str(), r["action"].as_str(), r["written"].as_bool()), (Some("aligned"), Some("none"), Some(false)));
    assert!(baks(e.proj.path()).is_empty(), "an aligned file is neither written nor backed up");
    assert_eq!(std::fs::read_to_string(e.proj.path().join("CLAUDE.md")).unwrap(), t);
}

#[test]
fn rules_are_appended_after_exactly_one_blank_line_and_crlf_is_kept() {
    let e = Env::new();
    let p = e.proj.path().join("CLAUDE.md");
    std::fs::write(&p, "# Mine\nkeep this").unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    assert_eq!(rowk(&v, "claude", "rules")["action"], "add");
    let t = std::fs::read_to_string(&p).unwrap();
    assert!(t.starts_with("# Mine\nkeep this\n\n<!-- comparegraph -->\n"), "{t}");
    assert_eq!(baks(e.proj.path()).len(), 1);
    // already ends with a blank line: still one
    std::fs::write(&p, "# Mine\n\n\n").unwrap();
    e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("# Mine\n\n\n<!-- comparegraph -->\n"));
    // one trailing newline: one blank line is added
    std::fs::write(&p, "# Mine\n").unwrap();
    e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("# Mine\n\n<!-- comparegraph -->\n"));
    // CRLF
    std::fs::write(&p, "# Mine\r\nline\r\n").unwrap();
    e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    let t = std::fs::read_to_string(&p).unwrap();
    assert!(t.starts_with("# Mine\r\nline\r\n\r\n<!-- comparegraph -->\r\n## CompareGraph\r\n"), "{t:?}");
    assert!(!t.replace("\r\n", "").contains('\n'), "every line break is CRLF");
    let v = e.setup(&["--client", "claude", "--no-hook"]);
    assert_eq!(rowk(&v, "claude", "rules")["state"], "aligned", "a CRLF block is aligned");
}

#[test]
fn an_older_shipped_block_is_refreshed_without_force() {
    let e = Env::new();
    let p = e.proj.path().join("CLAUDE.md");
    std::fs::write(&p, "before\n\n<!-- comparegraph -->\n<!-- /comparegraph -->\nafter\n").unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    let r = rowk(&v, "claude", "rules");
    assert_eq!((r["state"].as_str(), r["action"].as_str(), r["written"].as_bool()), (Some("stale"), Some("replace"), Some(true)));
    assert_eq!(r["note"], "an older CompareGraph block; refreshed");
    let t = std::fs::read_to_string(&p).unwrap();
    assert!(t.starts_with("before\n\n<!-- comparegraph -->\n## CompareGraph\n") && t.ends_with("<!-- /comparegraph -->\nafter\n"), "{t}");
    assert_eq!(baks(e.proj.path()).len(), 1);
}

#[test]
fn a_hand_edited_block_is_a_conflict_replaced_only_with_force_and_only_its_span() {
    let e = Env::new();
    let p = e.proj.path().join("CLAUDE.md");
    let edited = "top\n<!-- comparegraph -->\nmy own words\n<!-- /comparegraph -->\nbottom\n";
    std::fs::write(&p, edited).unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--no-hook"]);
    let r = rowk(&v, "claude", "rules");
    assert_eq!((r["state"].as_str(), r["action"].as_str(), r["written"].as_bool()), (Some("conflict"), Some("skip"), Some(false)));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), edited);
    assert!(baks(e.proj.path()).is_empty());
    let v = e.setup(&["--client", "claude", "--apply", "--force", "--no-hook"]);
    assert_eq!(rowk(&v, "claude", "rules")["action"], "replace");
    let t = std::fs::read_to_string(&p).unwrap();
    assert!(t.starts_with("top\n<!-- comparegraph -->\n## CompareGraph\n") && t.ends_with("<!-- /comparegraph -->\nbottom\n"), "{t}");
    assert!(!t.contains("my own words"));
    assert_eq!(block_of(&t).matches(OPEN).count(), 1);
}

#[test]
fn an_open_marker_without_a_close_is_a_conflict_even_with_force() {
    let e = Env::new();
    let p = e.proj.path().join("CLAUDE.md");
    let broken = "x\n<!-- comparegraph -->\nno end\n";
    std::fs::write(&p, broken).unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--force", "--no-hook"]);
    let r = rowk(&v, "claude", "rules");
    assert_eq!((r["state"].as_str(), r["written"].as_bool()), (Some("conflict"), Some(false)));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), broken);
}

#[test]
fn rules_rows_are_one_per_file_with_the_clients_joined_and_follow_detection() {
    let e = Env::new();
    let v = e.setup(&["--client", "all"]);
    assert!(kinds(&v, "rules").is_empty() && kinds(&v, "hook").is_empty(), "nothing detected, nothing planned: {v}");
    for d in [".codex", ".gemini", ".qwen", ".copilot", ".hermes"] {
        std::fs::create_dir_all(e.user(d)).unwrap();
    }
    let v = e.setup(&[]);
    assert_eq!(kinds(&v, "rules"), vec!["codex,hermes,copilot-cli", "gemini,qwen"]);
    assert_eq!(rowk(&v, "codex,hermes,copilot-cli", "rules")["path"], e.proj.path().join("AGENTS.md").to_string_lossy().as_ref());
    assert_eq!(rowk(&v, "gemini,qwen", "rules")["path"], e.proj.path().join("GEMINI.md").to_string_lossy().as_ref());
    assert!(kinds(&v, "hook").is_empty(), "claude is not detected");
    let v = e.setup(&["--client", "copilot", "--scope", "project"]);
    assert_eq!(rowk(&v, "copilot", "rules")["path"], e.proj.path().join(".github/instructions/comparegraph.instructions.md").to_string_lossy().as_ref());
}

#[test]
fn no_rules_plans_no_rules_rows() {
    let e = Env::new();
    let v = e.setup(&["--client", "codex", "--apply", "--no-rules"]);
    assert!(kinds(&v, "rules").is_empty());
    assert!(!e.proj.path().join("AGENTS.md").exists());
    assert!(e.user(".codex/config.toml").is_file());
}

// ---------- the SessionStart hook ----------

const HOOK: &str = "command -v comparegraph >/dev/null 2>&1 || exit 0; comparegraph doctor --brief";

#[test]
fn the_hook_is_created_added_next_to_other_hooks_and_aligned_on_rerun() {
    let e = Env::new();
    let v = e.setup(&["--client", "claude", "--scope", "project", "--apply", "--no-rules"]);
    let r = rowk(&v, "claude", "hook");
    assert_eq!((r["action"].as_str(), r["written"].as_bool()), (Some("create"), Some(true)));
    assert_eq!(r["path"], e.proj.path().join(".claude/settings.json").to_string_lossy().as_ref());
    assert_eq!(read_json(&e.proj.path().join(".claude/settings.json")), json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": HOOK, "timeout": 10}]}]}}));
    let v = e.setup(&["--client", "claude", "--scope", "project", "--apply", "--no-rules"]);
    assert_eq!(rowk(&v, "claude", "hook")["state"], "aligned");
    assert!(baks(&e.proj.path().join(".claude")).is_empty());

    let e = Env::new();
    let other = json!({"model": "x", "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "lint"}]}], "SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "echo hi"}]}]}});
    std::fs::create_dir_all(e.user(".claude")).unwrap();
    std::fs::write(e.user(".claude/settings.json"), serde_json::to_string_pretty(&other).unwrap()).unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--no-rules"]);
    let r = rowk(&v, "claude", "hook");
    assert_eq!((r["action"].as_str(), r["path"].as_str()), (Some("add"), Some(e.user(".claude/settings.json").to_string_lossy().as_ref())));
    let j = read_json(&e.user(".claude/settings.json"));
    assert_eq!(j["model"], "x");
    assert_eq!(j["hooks"]["PreToolUse"], other["hooks"]["PreToolUse"]);
    assert_eq!(j["hooks"]["SessionStart"][0], other["hooks"]["SessionStart"][0]);
    assert_eq!(j["hooks"]["SessionStart"][1], json!({"hooks": [{"type": "command", "command": HOOK, "timeout": 10}]}));
    assert_eq!(j["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    assert_eq!(baks(&e.user(".claude")).len(), 1);
    // the config row of claude user scope stays ~/.claude.json
    assert_eq!(rowk(&v, "claude", "config")["path"], e.user(".claude.json").to_string_lossy().as_ref());
}

#[test]
fn a_different_doctor_hook_is_stale_and_replaced_only_with_force() {
    let e = Env::new();
    let old = json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "comparegraph doctor --brief", "timeout": 3}]}]}});
    std::fs::create_dir_all(e.user(".claude")).unwrap();
    let path = e.user(".claude/settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&old).unwrap()).unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--no-rules"]);
    let r = rowk(&v, "claude", "hook");
    assert_eq!((r["state"].as_str(), r["action"].as_str(), r["written"].as_bool()), (Some("stale"), Some("skip"), Some(false)));
    assert!(r["note"].as_str().unwrap().contains("--force"));
    assert_eq!(read_json(&path), old);
    let v = e.setup(&["--client", "claude", "--apply", "--force", "--no-rules"]);
    assert_eq!(rowk(&v, "claude", "hook")["action"], "replace");
    let j = read_json(&path);
    assert_eq!(j["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert_eq!(j["hooks"]["SessionStart"][0]["hooks"][0]["command"], HOOK);
}

#[test]
fn an_unreadable_settings_file_is_never_written_and_no_hook_skips_the_row() {
    let e = Env::new();
    std::fs::create_dir_all(e.user(".claude")).unwrap();
    std::fs::write(e.user(".claude/settings.json"), "{ nope").unwrap();
    let v = e.setup(&["--client", "claude", "--apply", "--force", "--no-rules"]);
    let r = rowk(&v, "claude", "hook");
    assert_eq!((r["state"].as_str(), r["written"].as_bool()), (Some("conflict"), Some(false)));
    assert_eq!(std::fs::read_to_string(e.user(".claude/settings.json")).unwrap(), "{ nope");
    let v = e.setup(&["--client", "claude", "--apply", "--no-hook", "--no-rules"]);
    assert!(kinds(&v, "hook").is_empty());
    assert_eq!(std::fs::read_to_string(e.user(".claude/settings.json")).unwrap(), "{ nope");
}

#[test]
fn the_plugin_hook_file_carries_the_same_command_setup_writes() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hooks/hooks.json");
    let h = read_json(&p);
    assert_eq!(h["SessionStart"][0]["hooks"][0]["command"], comparegraph::application::setup::HOOK_COMMAND);
    assert_eq!(h["SessionStart"][0]["hooks"][0]["type"], "command");
    assert_eq!(h["SessionStart"][0]["hooks"][0]["timeout"], 10);
    assert_eq!(comparegraph::application::setup::HOOK_COMMAND, HOOK);
    let plugin = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.claude-plugin/plugin.json"));
    assert_eq!(plugin["hooks"], "./hooks/hooks.json");
}

// ---------- doctor --brief ----------

fn brief(e: &Env) -> (String, i32) {
    let mut c = Command::cargo_bin("comparegraph").unwrap();
    c.env("HOME", e.home.path()).env("XDG_CONFIG_HOME", e.xdg.path()).env("COMPAREGRAPH_BIN", BIN).current_dir(e.proj.path());
    let out = c.arg("--root").arg(e.ws.path()).args(["--pretty", "doctor", "--brief"]).output().unwrap();
    (String::from_utf8(out.stdout).unwrap(), out.status.code().unwrap())
}

#[test]
fn doctor_brief_is_silent_without_a_store_and_one_or_two_lines_with_one() {
    let e = Env::new();
    assert_eq!(brief(&e), (String::new(), 0));
    std::fs::create_dir_all(e.user(".gemini")).unwrap();
    assert_eq!(brief(&e), (String::new(), 0), "still no store");
    Command::cargo_bin("comparegraph").unwrap().current_dir(e.proj.path()).arg("--root").arg(e.ws.path()).args(["build", "x"]).assert().code(1); // creates the store
    let (out, code) = brief(&e);
    assert_eq!(code, 0);
    let store = std::fs::canonicalize(e.ws.path()).unwrap().join(".comparegraph/comparegraph.db");
    let lines: Vec<_> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].starts_with("CompareGraph: 0 repos, store ") && lines[0].ends_with("; server missing for gemini"), "{out}");
    assert!(lines[0].contains(store.file_name().unwrap().to_str().unwrap()));
    assert_eq!(lines[1], "Run `comparegraph setup --apply` (state: gemini missing).");
    e.setup(&["--client", "gemini", "--apply", "--no-rules"]);
    let (out, _) = brief(&e);
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(out.trim_end().ends_with("; server aligned for gemini"), "{out}");
    // nothing detected
    let e2 = Env::new();
    Command::cargo_bin("comparegraph").unwrap().current_dir(e2.proj.path()).arg("--root").arg(e2.ws.path()).args(["build", "x"]).assert().code(1);
    let (out, code) = brief(&e2);
    assert_eq!(code, 0);
    assert!(out.trim_end().ends_with("; no agent client detected") && out.lines().count() == 1, "{out}");
}

// ---------- the six newer clients ----------

#[test]
fn apply_writes_the_shape_of_each_newer_client() {
    let e = Env::new();
    let args = json!(["--root", e.ws_str(), "mcp"]);
    for c in ["windsurf", "antigravity", "copilot-cli", "qwen"] {
        let v = e.setup(&["--client", c, "--apply", "--no-rules"]);
        assert_eq!(rowk(&v, c, "config")["written"], true, "{c}: {v}");
    }
    for c in ["kiro", "copilot"] {
        let v = e.setup(&["--client", c, "--scope", "project", "--apply", "--no-rules"]);
        assert_eq!(rowk(&v, c, "config")["written"], true, "{c}: {v}");
    }
    let plain = json!({"command": BIN, "args": args});
    let stdio = json!({"type": "stdio", "command": BIN, "args": args});
    assert_eq!(read_json(&e.user(".codeium/windsurf/mcp_config.json")), json!({"mcpServers": {"comparegraph": plain}}));
    assert_eq!(read_json(&e.user(".gemini/antigravity/mcp_config.json")), json!({"mcpServers": {"comparegraph": plain}}));
    assert_eq!(read_json(&e.user(".copilot/mcp-config.json")), json!({"mcpServers": {"comparegraph": {"type": "local", "command": BIN, "args": args, "tools": ["*"]}}}));
    assert_eq!(read_json(&e.user(".qwen/settings.json")), json!({"mcpServers": {"comparegraph": stdio}}));
    assert_eq!(read_json(&e.proj.path().join(".kiro/settings/mcp.json")), json!({"mcpServers": {"comparegraph": stdio}}));
    assert_eq!(read_json(&e.proj.path().join(".vscode/mcp.json")), json!({"servers": {"comparegraph": stdio}}));
    // aligned on a second look, and another `type` is a conflict
    assert_eq!(rowk(&e.setup(&["--client", "qwen", "--no-rules"]), "qwen", "config")["state"], "aligned");
    assert_eq!(rowk(&e.setup(&["--client", "copilot-cli", "--no-rules"]), "copilot-cli", "config")["state"], "aligned");
    let other = json!({"mcpServers": {"comparegraph": {"type": "sse", "command": BIN, "args": args}}});
    std::fs::write(e.user(".qwen/settings.json"), other.to_string()).unwrap();
    assert_eq!(rowk(&e.setup(&["--client", "qwen", "--no-rules"]), "qwen", "config")["state"], "conflict");
    // no scope it does not have
    for (c, scope) in [("windsurf", "project"), ("kiro", "user"), ("copilot", "user")] {
        assert_eq!(e.setup(&["--client", c, "--scope", scope])["error"]["code"], "invalid_arguments", "{c}");
    }
}
