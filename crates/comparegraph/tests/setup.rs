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
    assert_eq!(rows.iter().filter(|r| r["scope"] == "user").count(), 6);
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
