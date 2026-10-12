//! `setup` and the `doctor` registrations: write the `comparegraph` MCP server entry into agent clients' configs (spec §10).
//!
//! Plan first: a dry run is the default and writes nothing. `--apply` writes additively, with a backup `<file>.bak-<unix-seconds>`
//! beside any file it changes. JSON configs are parsed and re-serialised (2-space indent, every other key kept; key order is
//! sorted because the crate carries no order-preserving map). TOML and YAML are never parsed-and-rewritten: the entry is appended
//! (or, for a stale command, one line is replaced) as text, and only when the `comparegraph` entry is absent or ours.
//! A `comparegraph` entry that points elsewhere is a `conflict` and stays untouched unless `--force`.
//!
//! Verified config shapes (documentation read 2026-10-13):
//! - Claude Code: `.mcp.json` (project) / `~/.claude.json` (user), `mcpServers.<name> = {command, args}`.
//! - Codex: `~/.codex/config.toml` or `.codex/config.toml`, `[mcp_servers.<name>]` with `command` and `args`.
//! - Cursor: `~/.cursor/mcp.json` or `.cursor/mcp.json`, `mcpServers.<name> = {command, args}`.
//! - Gemini CLI: `~/.gemini/settings.json` or `.gemini/settings.json`, `mcpServers.<name> = {command, args}`.
//! - OpenCode: `~/.config/opencode/opencode.json` (or `$XDG_CONFIG_HOME`) or `opencode.json`, `mcp.<name> = {type:"local", command:[..]}`.
//! - Hermes: `~/.hermes/config.yaml`, `mcp_servers.<name>` with `command` and `args` (no project scope).
use super::{AppError, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SERVER: &str = "comparegraph";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Client {
    Claude,
    Codex,
    Cursor,
    Gemini,
    OpenCode,
    Hermes,
}
pub const ALL: [Client; 6] = [
    Client::Claude,
    Client::Codex,
    Client::Cursor,
    Client::Gemini,
    Client::OpenCode,
    Client::Hermes,
];
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    User,
    Project,
}
impl Client {
    pub fn name(self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Codex => "codex",
            Client::Cursor => "cursor",
            Client::Gemini => "gemini",
            Client::OpenCode => "opencode",
            Client::Hermes => "hermes",
        }
    }
    pub fn parse(s: &str) -> Option<Client> {
        ALL.into_iter().find(|c| c.name() == s)
    }
    /// A client/scope whose shape could not be verified is reported `unverified` and never written.
    /// Cursor's project file `.cursor/mcp.json` is documented; the user-level path is not stated in the docs read.
    pub fn verified(self, scope: Scope) -> bool {
        !(self == Client::Cursor && scope == Scope::User)
    }
}
impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
        }
    }
}

/// Where things are. Tests build one over temp dirs; the CLI builds it from `HOME`, `XDG_CONFIG_HOME` and the cwd.
pub struct Ctx {
    pub home: Option<PathBuf>,
    pub xdg: Option<PathBuf>,
    pub project: PathBuf,
    pub bin: String,
    /// the command came from `--bin` / COMPAREGRAPH_BIN rather than the running executable
    pub explicit_bin: bool,
    /// the running executable sits under a `target/` directory (a build tree)
    pub dev_build: bool,
    pub workspace: String,
}
impl Ctx {
    pub fn from_env(root: &Path, bin: Option<&str>) -> Ctx {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let explicit = bin.map(str::to_string).or_else(|| std::env::var("COMPAREGRAPH_BIN").ok().filter(|b| !b.is_empty()));
        let exe = std::env::current_exe().ok();
        let dev_build = exe.as_ref().is_some_and(|p| p.components().any(|c| c.as_os_str() == "target"));
        let explicit_bin = explicit.is_some();
        let bin = match explicit {
            Some(b) => absolutise(&b, &cwd),
            None => exe.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "comparegraph".into()),
        };
        let ws = std::fs::canonicalize(root).unwrap_or_else(|_| {
            if root.is_absolute() {
                root.to_path_buf()
            } else {
                cwd.join(root)
            }
        });
        Ctx { home, xdg, project: cwd, bin, explicit_bin, dev_build, workspace: ws.to_string_lossy().into_owned() }
    }
    fn args(&self) -> Vec<String> {
        vec!["--root".into(), self.workspace.clone(), "mcp".into()]
    }
    /// The config file for a client and scope, and the directory/file whose existence means "this client is installed".
    pub fn locate(&self, c: Client, s: Scope) -> Option<(PathBuf, PathBuf)> {
        let home = self.home.as_ref();
        match (c, s) {
            (Client::Claude, Scope::User) => home.map(|h| (h.join(".claude.json"), h.join(".claude"))),
            (Client::Claude, Scope::Project) => Some((self.project.join(".mcp.json"), self.project.join(".claude"))),
            (Client::Codex, Scope::User) => home.map(|h| (h.join(".codex/config.toml"), h.join(".codex"))),
            (Client::Codex, Scope::Project) => Some((self.project.join(".codex/config.toml"), self.project.join(".codex"))),
            (Client::Cursor, Scope::User) => home.map(|h| (h.join(".cursor/mcp.json"), h.join(".cursor"))),
            (Client::Cursor, Scope::Project) => Some((self.project.join(".cursor/mcp.json"), self.project.join(".cursor"))),
            (Client::Gemini, Scope::User) => home.map(|h| (h.join(".gemini/settings.json"), h.join(".gemini"))),
            (Client::Gemini, Scope::Project) => Some((self.project.join(".gemini/settings.json"), self.project.join(".gemini"))),
            (Client::OpenCode, Scope::User) => {
                self.xdg.as_ref().map(|x| (x.join("opencode/opencode.json"), x.join("opencode")))
            }
            (Client::OpenCode, Scope::Project) => {
                Some((self.project.join("opencode.json"), self.project.join("opencode.json")))
            }
            (Client::Hermes, Scope::User) => home.map(|h| (h.join(".hermes/config.yaml"), h.join(".hermes"))),
            (Client::Hermes, Scope::Project) => None,
        }
    }
}

/// A relative command path becomes absolute (canonical if it exists, else joined with the cwd); a bare name stays a PATH lookup.
fn absolutise(b: &str, cwd: &Path) -> String {
    if !b.contains('/') && !b.contains('\\') {
        return b.to_string();
    }
    let p = Path::new(b);
    if let Ok(c) = std::fs::canonicalize(p) {
        return c.to_string_lossy().into_owned();
    }
    if p.is_absolute() {
        b.to_string()
    } else {
        let mut out = cwd.to_path_buf();
        for c in p.components() {
            match c {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    out.pop();
                }
                o => out.push(o),
            }
        }
        out.to_string_lossy().into_owned()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Aligned,
    Stale,
    Missing,
    Conflict,
    Unverified,
}
impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Aligned => "aligned",
            State::Stale => "stale",
            State::Missing => "missing",
            State::Conflict => "conflict",
            State::Unverified => "unverified",
        }
    }
}
/// What is in the file for our server, compared with what we would write.
enum Cmp {
    Equal,
    /// same arguments, a different binary path
    Stale,
    Other,
}
struct Plan {
    state: State,
    note: Option<String>,
    /// the file content for a missing/stale entry (what `--apply` writes)
    new: Option<String>,
    /// the file content that replaces a conflicting entry (what `--apply --force` writes); `None` = never overwritten
    forced: Option<String>,
    file_exists: bool,
}
impl Plan {
    fn conflict(note: impl Into<String>, forced: Option<String>, file_exists: bool) -> Plan {
        Plan { state: State::Conflict, note: Some(note.into()), new: None, forced, file_exists }
    }
}

fn classify(bin: &str, args: &[String], got_cmd: Option<&str>, got_args: Option<&[String]>) -> Cmp {
    if got_args != Some(args) {
        return Cmp::Other;
    }
    if got_cmd == Some(bin) {
        Cmp::Equal
    } else if got_cmd.is_some() {
        Cmp::Stale
    } else {
        Cmp::Other
    }
}
fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
fn jarr(v: &[String]) -> String {
    serde_json::to_string(v).unwrap()
}

// ---------- JSON ----------

fn json_key(c: Client) -> &'static str {
    if c == Client::OpenCode {
        "mcp"
    } else {
        "mcpServers"
    }
}
fn json_entry(c: Client, ctx: &Ctx) -> Value {
    if c == Client::OpenCode {
        let mut cmd = vec![ctx.bin.clone()];
        cmd.extend(ctx.args());
        json!({ "type": "local", "command": cmd })
    } else {
        json!({ "command": ctx.bin, "args": ctx.args() })
    }
}
fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array()?.iter().map(|x| x.as_str().map(str::to_string)).collect()
}
fn json_cmp(c: Client, ctx: &Ctx, entry: &Value) -> Cmp {
    let args = ctx.args();
    if c == Client::OpenCode {
        if entry["type"] != "local" {
            return Cmp::Other;
        }
        return match strings(&entry["command"]) {
            Some(v) if !v.is_empty() => classify(&ctx.bin, &args, Some(&v[0]), Some(&v[1..])),
            _ => Cmp::Other,
        };
    }
    let a = strings(&entry["args"]);
    classify(&ctx.bin, &args, entry["command"].as_str(), a.as_deref())
}
fn json_text(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    s.push('\n');
    s
}
fn plan_json(c: Client, ctx: &Ctx, path: &Path) -> Plan {
    let desired = json_entry(c, ctx);
    let key = json_key(c);
    let exists = path.is_file();
    let mut root = if exists {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return Plan::conflict(format!("cannot read: {e}"), None, true),
        };
        if text.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => v,
                Err(e) => return Plan::conflict(format!("not valid JSON ({e}); left untouched"), None, true),
            }
        }
    } else {
        json!({})
    };
    if !root.is_object() {
        return Plan::conflict("top level is not an object; left untouched", None, exists);
    }
    if c == Client::OpenCode && !exists && !root.as_object().unwrap().contains_key("$schema") {
        root["$schema"] = json!("https://opencode.ai/config.json");
    }
    match root.get(key) {
        Some(v) if !v.is_object() => {
            return Plan::conflict(format!("`{key}` is not an object; left untouched"), None, exists)
        }
        _ => {}
    }
    let existing = root.get(key).and_then(|m| m.get(SERVER)).cloned();
    let mut with = root.clone();
    if with.get(key).is_none() {
        with[key] = json!({});
    }
    with[key][SERVER] = desired;
    let new = Some(json_text(&with));
    match existing {
        None => Plan { state: State::Missing, note: None, new, forced: None, file_exists: exists },
        Some(e) => match json_cmp(c, ctx, &e) {
            Cmp::Equal => Plan { state: State::Aligned, note: None, new: None, forced: None, file_exists: exists },
            Cmp::Stale => Plan {
                state: State::Stale,
                note: Some("same arguments, a different binary path".into()),
                new: None,
                forced: new,
                file_exists: exists,
            },
            Cmp::Other => Plan::conflict("a `comparegraph` entry points elsewhere", new, exists),
        },
    }
}

// ---------- text blocks (TOML, YAML) ----------

/// A text file split into lines, remembering its line ending and whether it ended with one.
struct Doc {
    lines: Vec<String>,
    eol: &'static str,
    final_nl: bool,
}
impl Doc {
    fn new(text: &str) -> Doc {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Doc { lines: text.lines().map(str::to_string).collect(), eol, final_nl: text.is_empty() || text.ends_with('\n') }
    }
    fn join(&self, lines: &[String]) -> String {
        let mut s = lines.join(self.eol);
        if self.final_nl {
            s.push_str(self.eol);
        }
        s
    }
}
fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start().len()
}
fn blank_or_comment(l: &str) -> bool {
    let t = l.trim();
    t.is_empty() || t.starts_with('#')
}
/// `key = value` / `key: value` lines of a section, parsed as JSON scalars/arrays (TOML basic strings and YAML flow lists we write are JSON).
fn field(lines: &[String], key: &str, sep: char) -> Option<(usize, Value)> {
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix(key) {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix(sep) {
                let v = v.trim();
                // parse the quoted string / array first, so a ` #` inside a value is not taken for a comment
                let parsed = serde_json::Deserializer::from_str(v).into_iter::<Value>().next().and_then(|r| r.ok()).or_else(|| {
                    let plain = v.split(" #").next().unwrap_or(v).trim();
                    if sep == ':' && !plain.is_empty() && !plain.starts_with(['[', '{', '"', '\'']) {
                        Some(json!(plain))
                    } else {
                        None
                    }
                });
                return Some((i, parsed.unwrap_or(Value::Null)));
            }
        }
    }
    None
}
fn text_cmp(ctx: &Ctx, section: &[String], sep: char) -> (Cmp, Option<usize>) {
    let cmd = field(section, "command", sep);
    let args = field(section, "args", sep);
    let argv = args.as_ref().and_then(|(_, v)| strings(v));
    let cmd_s = cmd.as_ref().and_then(|(_, v)| v.as_str().map(str::to_string));
    (classify(&ctx.bin, &ctx.args(), cmd_s.as_deref(), argv.as_deref()), cmd.map(|(i, _)| i))
}

fn toml_block(ctx: &Ctx) -> Vec<String> {
    vec![
        format!("[mcp_servers.{SERVER}]"),
        format!("command = {}", js(&ctx.bin)),
        format!("args = {}", jarr(&ctx.args())),
    ]
}
fn plan_toml(ctx: &Ctx, path: &Path) -> Plan {
    let exists = path.is_file();
    let text = if exists {
        match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return Plan::conflict(format!("cannot read: {e}"), None, true),
        }
    } else {
        String::new()
    };
    let doc = Doc::new(&text);
    let mut lines = doc.lines.clone();
    let header = |l: &String| {
        let t = l.trim();
        t == format!("[mcp_servers.{SERVER}]") || t == format!("[mcp_servers.\"{SERVER}\"]")
    };
    let mentions = lines.iter().any(|l| {
        let t = l.trim_start();
        t.starts_with(&format!("mcp_servers.{SERVER}")) || t.starts_with(&format!("mcp_servers.\"{SERVER}\""))
    });
    let start = lines.iter().position(header);
    let Some(start) = start else {
        if mentions {
            return Plan::conflict("a `comparegraph` entry in a form setup does not edit; left untouched", None, exists);
        }
        if lines.iter().any(|l| l.trim_start().starts_with("mcp_servers") && l.contains('=')) {
            return Plan::conflict("`mcp_servers` is defined inline; left untouched", None, exists);
        }
        let mut all = lines;
        if !all.is_empty() && !all.last().unwrap().trim().is_empty() {
            all.push(String::new());
        }
        all.extend(toml_block(ctx));
        return Plan { state: State::Missing, note: None, new: Some(doc.join(&all)), forced: None, file_exists: exists };
    };
    let end = (start + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('[')).unwrap_or(lines.len());
    let body: Vec<String> = lines[start + 1..end].iter().filter(|l| !blank_or_comment(l)).cloned().collect();
    let (cmp, _) = text_cmp(ctx, &body, '=');
    // the end of the table without its trailing blank lines
    let mut real_end = end;
    while real_end > start + 1 && lines[real_end - 1].trim().is_empty() {
        real_end -= 1;
    }
    let replace_all = |lines: &mut Vec<String>| {
        lines.splice(start..real_end, toml_block(ctx));
        doc.join(lines)
    };
    match cmp {
        Cmp::Equal => Plan { state: State::Aligned, note: None, new: None, forced: None, file_exists: exists },
        Cmp::Stale => {
            let rel = lines[start + 1..end].iter().position(|l| l.trim_start().starts_with("command")).unwrap();
            lines[start + 1 + rel] = format!("command = {}", js(&ctx.bin));
            Plan {
                state: State::Stale,
                note: Some("same arguments, a different binary path".into()),
                new: None,
                forced: Some(doc.join(&lines)),
                file_exists: exists,
            }
        }
        Cmp::Other => {
            let forced = Some(replace_all(&mut lines));
            Plan::conflict("a `comparegraph` entry points elsewhere", forced, exists)
        }
    }
}

fn yaml_entry(ctx: &Ctx, ind: usize) -> Vec<String> {
    let p = " ".repeat(ind);
    vec![
        format!("{p}{SERVER}:"),
        format!("{p}  command: {}", js(&ctx.bin)),
        format!("{p}  args: {}", jarr(&ctx.args())),
    ]
}
fn plan_yaml(ctx: &Ctx, path: &Path) -> Plan {
    let exists = path.is_file();
    let text = if exists {
        match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return Plan::conflict(format!("cannot read: {e}"), None, true),
        }
    } else {
        String::new()
    };
    let doc = Doc::new(&text);
    let mut lines = doc.lines.clone();
    let top = lines.iter().position(|l| l.starts_with("mcp_servers:"));
    let Some(top) = top else {
        let mut all = lines;
        if !all.is_empty() && !all.last().unwrap().trim().is_empty() {
            all.push(String::new());
        }
        all.push("mcp_servers:".into());
        all.extend(yaml_entry(ctx, 2));
        return Plan { state: State::Missing, note: None, new: Some(doc.join(&all)), forced: None, file_exists: exists };
    };
    let after = lines[top]["mcp_servers:".len()..].trim();
    if !after.is_empty() && !after.starts_with('#') {
        return Plan::conflict("`mcp_servers` is not a block mapping; left untouched", None, exists);
    }
    let block_end = (top + 1..lines.len()).find(|&i| !blank_or_comment(&lines[i]) && indent_of(&lines[i]) == 0).unwrap_or(lines.len());
    let child_ind = (top + 1..block_end).find(|&i| !blank_or_comment(&lines[i])).map(|i| indent_of(&lines[i])).unwrap_or(2);
    let me = (top + 1..block_end).find(|&i| indent_of(&lines[i]) == child_ind && [format!("{SERVER}:"), format!("\"{SERVER}\":"), format!("'{SERVER}':")].contains(&lines[i].trim().to_string()));
    let Some(me) = me else {
        lines.splice(top + 1..top + 1, yaml_entry(ctx, child_ind));
        return Plan { state: State::Missing, note: None, new: Some(doc.join(&lines)), forced: None, file_exists: exists };
    };
    let mut me_end = (me + 1..block_end).find(|&i| !blank_or_comment(&lines[i]) && indent_of(&lines[i]) <= child_ind).unwrap_or(block_end);
    while me_end > me + 1 && blank_or_comment(&lines[me_end - 1]) {
        me_end -= 1;
    }
    let body: Vec<String> = lines[me + 1..me_end].iter().filter(|l| !blank_or_comment(l)).cloned().collect();
    let (cmp, _) = text_cmp(ctx, &body, ':');
    match cmp {
        Cmp::Equal => Plan { state: State::Aligned, note: None, new: None, forced: None, file_exists: exists },
        Cmp::Stale => {
            let rel = lines[me + 1..me_end].iter().position(|l| l.trim_start().starts_with("command")).unwrap();
            let i = me + 1 + rel;
            lines[i] = format!("{}command: {}", " ".repeat(indent_of(&lines[i])), js(&ctx.bin));
            Plan {
                state: State::Stale,
                note: Some("same arguments, a different binary path".into()),
                new: None,
                forced: Some(doc.join(&lines)),
                file_exists: exists,
            }
        }
        Cmp::Other => {
            lines.splice(me..me_end, yaml_entry(ctx, child_ind));
            Plan::conflict("a `comparegraph` entry points elsewhere", Some(doc.join(&lines)), exists)
        }
    }
}

fn plan_for(c: Client, ctx: &Ctx, path: &Path) -> Plan {
    match c {
        Client::Codex => plan_toml(ctx, path),
        Client::Hermes => plan_yaml(ctx, path),
        _ => {
            if c == Client::OpenCode && !path.exists() && path.with_extension("jsonc").exists() {
                return Plan::conflict("opencode.jsonc exists; JSON-with-comments is not edited", None, false);
            }
            plan_json(c, ctx, path)
        }
    }
}

// ---------- rows ----------

pub struct Row {
    pub client: Client,
    pub scope: Scope,
    pub path: PathBuf,
    pub state: State,
    pub action: &'static str,
    pub written: bool,
    pub backup: Option<PathBuf>,
    pub note: Option<String>,
}
impl Row {
    pub fn to_json(&self, setup: bool) -> Value {
        let mut v = json!({
            "client": self.client.name(), "scope": self.scope.name(),
            "path": self.path.to_string_lossy(), "state": self.state.name(),
        });
        if setup {
            v["action"] = json!(self.action);
            v["written"] = json!(self.written);
            if let Some(b) = &self.backup {
                v["backup"] = json!(b.to_string_lossy());
            }
        }
        if let Some(n) = &self.note {
            v["note"] = json!(n);
        }
        v
    }
}

/// The registration state of every client (what `doctor` reports): the user-scope file of each client, and the
/// project-scope file of each client that has one in this project.
pub fn registrations(ctx: &Ctx) -> Vec<Row> {
    let mut rows = Vec::new();
    for scope in [Scope::User, Scope::Project] {
        for c in ALL {
            let Some((path, _)) = ctx.locate(c, scope) else { continue };
            if scope == Scope::Project && !path.exists() {
                continue;
            }
            if !c.verified(scope) {
                rows.push(Row { client: c, scope, path, state: State::Unverified, action: "skip", written: false, backup: None, note: Some("config shape not verified".into()) });
                continue;
            }
            let p = plan_for(c, ctx, &path);
            rows.push(Row { client: c, scope, path, state: p.state, action: "none", written: false, backup: None, note: p.note });
        }
    }
    rows
}

pub struct Options {
    pub clients: Vec<Client>,
    /// `--client all`: only clients that look installed
    pub detect: bool,
    pub scope: Scope,
    pub apply: bool,
    pub force: bool,
}

fn unix_seconds() -> u64 {
    // COMPAREGRAPH_NOW fixes the clock (tests)
    if let Some(n) = std::env::var("COMPAREGRAPH_NOW").ok().and_then(|n| n.parse().ok()) {
        return n;
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
fn write_file(nominal: &Path, content: &str, backup: bool) -> Result<Option<PathBuf>> {
    use std::io::Write;
    // a symlinked config is written through: the resolved file is replaced, the link stays
    let path = std::fs::canonicalize(nominal).unwrap_or_else(|_| nominal.to_path_buf());
    let path = path.as_path();
    let io = |e| AppError::Io(path.to_path_buf(), e);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let mut bak = None;
    if backup && path.is_file() {
        let original = std::fs::read(path).map_err(io)?;
        let base = format!("{}.bak-{}", path.file_name().unwrap().to_string_lossy(), unix_seconds());
        let mut n = 1u32;
        loop {
            let name = if n == 1 { base.clone() } else { format!("{base}-{n}") };
            let b = path.with_file_name(name);
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&b) {
                Ok(mut f) => {
                    f.write_all(&original).and_then(|_| f.sync_all()).map_err(io)?;
                    bak = Some(b);
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(io(e)),
            }
        }
    }
    let tmp = path.with_file_name(format!(".{}.cgtmp-{}", path.file_name().unwrap().to_string_lossy(), std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(content.as_bytes())?;
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = f.set_permissions(meta.permissions());
        }
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    Ok(bak)
}

pub fn run(ctx: &Ctx, o: &Options) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for &c in &o.clients {
        let Some((path, detect)) = ctx.locate(c, o.scope) else {
            if !o.detect {
                return Err(AppError::InvalidArguments(format!("{} has no {} scope", c.name(), o.scope.name())));
            }
            continue;
        };
        let mut row = Row { client: c, scope: o.scope, path: path.clone(), state: State::Missing, action: "skip", written: false, backup: None, note: None };
        if !c.verified(o.scope) {
            row.state = State::Unverified;
            row.note = Some("config shape not verified; skipped".into());
            rows.push(row);
            continue;
        }
        let plan = plan_for(c, ctx, &path);
        row.state = plan.state;
        row.note = plan.note.clone();
        if o.detect && !path.exists() && !detect.exists() {
            row.note = Some("client not detected; skipped".into());
            rows.push(row);
            continue;
        }
        let (content, action) = match plan.state {
            State::Aligned => (None, "none"),
            State::Missing => (plan.new, if plan.file_exists { "add" } else { "create" }),
            State::Stale if o.force => (plan.forced, "replace"),
            State::Stale => {
                row.note = Some("same arguments, a different binary path; untouched (use --force to replace)".into());
                (None, "skip")
            }
            State::Conflict if o.force && plan.forced.is_some() => (plan.forced, "replace"),
            State::Conflict => {
                row.note = Some(format!(
                    "{}; untouched{}",
                    plan.note.as_deref().unwrap_or("conflict"),
                    if plan.forced.is_some() { " (use --force to replace)" } else { "" }
                ));
                (None, "skip")
            }
            State::Unverified => (None, "skip"),
        };
        row.action = action;
        if o.apply {
            if let Some(text) = content {
                row.backup = write_file(&path, &text, true)?;
                row.written = true;
            }
        }
        rows.push(row);
    }
    Ok(rows)
}
