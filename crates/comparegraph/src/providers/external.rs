//! The external engine host (spec §6b): a provider that lives in another process and speaks JSON lines on stdin/stdout.
//! Protocol v1 is documented in `docs/plugin-protocol.md`. Everything the plugin prints is data, never instructions: it is
//! parsed into our own types, unknown kinds are dropped and counted, and a hang, garbage or a non-zero exit is
//! `ProviderError::Unavailable` so the build fails with the store untouched.
use super::{BuildOptions, BuildOutput, Capabilities, GraphProvider, ProviderError, ProviderStatus};
use crate::domain::{content_hash, EdgeKind, Limitation, Node, NodeKind};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const PROTOCOL: &str = "1";
pub const MANIFEST_FILE: &str = "comparegraph-plugin.toml";
pub const BIN_PREFIX: &str = "comparegraph-provider-";
const DEFAULT_TIMEOUT_SECS: u64 = 1800;
const OUTPUT_CAP: usize = 256 * 1024 * 1024;

/// stdout byte cap; `COMPAREGRAPH_PLUGIN_OUTPUT_CAP` (bytes) lowers it, which the tests use.
fn output_cap() -> usize {
    std::env::var("COMPAREGRAPH_PLUGIN_OUTPUT_CAP").ok().and_then(|v| v.parse().ok()).unwrap_or(OUTPUT_CAP)
}

/// `COMPAREGRAPH_PLUGIN_TIMEOUT` in seconds: unset = 1800; unparsable or zero is refused, never silently defaulted.
fn timeout_secs() -> Result<u64, String> {
    match std::env::var("COMPAREGRAPH_PLUGIN_TIMEOUT") {
        Err(_) => Ok(DEFAULT_TIMEOUT_SECS),
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("COMPAREGRAPH_PLUGIN_TIMEOUT='{v}' is not a positive number of seconds")),
        },
    }
}

/// The deadline of a `capabilities` probe (doctor, detect): 10 s, `COMPAREGRAPH_PLUGIN_CAPS_TIMEOUT` (whole seconds) overrides it.
fn caps_timeout_secs() -> u64 {
    std::env::var("COMPAREGRAPH_PLUGIN_CAPS_TIMEOUT").ok().and_then(|v| v.trim().parse().ok()).filter(|n| *n > 0).unwrap_or(10)
}

/// A plugin path must be relative and stay inside the clone: no absolute path, no `..` segment, no NUL, no drive letter.
fn path_ok(p: &str) -> bool {
    !(p.contains('\0')
        || p.starts_with('/')
        || p.starts_with('\\')
        || p.as_bytes().get(1) == Some(&b':')
        || p.split(['/', '\\']).any(|seg| seg == ".."))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Manifest {
    pub kind: String,
    pub name: String,
    pub version: String,
    pub protocol: String,
    pub languages: Vec<String>,
    pub network: bool,
}

#[derive(Debug, Clone)]
pub struct ExternalProvider {
    pub name: String,
    pub command: PathBuf,
    /// `None` = no manifest beside the command: the provider is `unverified`.
    pub manifest: Option<Manifest>,
    /// where `comparegraph.toml` and `.comparegraph/conform-<name>.json` live
    pub workspace: PathBuf,
}

/// What `doctor` reports for one discovered external provider.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExternalInfo {
    pub name: String,
    pub command: String,
    pub manifest: String,
    pub conformance: serde_json::Value,
    /// the plugin's own `unavailable` reason from `capabilities` (e.g. its engine is not installed); absent when it is usable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').trim_matches('\'').to_string()
}
fn parse_list(v: &str) -> Vec<String> {
    v.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(unquote)
        .filter(|s| !s.is_empty())
        .collect()
}
fn kv(line: &str) -> Option<(String, String)> {
    let line = line.split('#').next().unwrap_or("").trim();
    let (k, v) = line.split_once('=')?;
    Some((k.trim().to_string(), v.trim().to_string()))
}

pub fn parse_manifest(text: &str) -> Manifest {
    let mut m = Manifest::default();
    for line in text.lines() {
        if let Some((k, v)) = kv(line) {
            match k.as_str() {
                "kind" => m.kind = unquote(&v),
                "name" => m.name = unquote(&v),
                "version" => m.version = unquote(&v),
                "protocol" => m.protocol = unquote(&v),
                "languages" => m.languages = parse_list(&v),
                "network" => m.network = v == "true",
                _ => {}
            }
        }
    }
    m
}

/// `[providers.<name>] command = "…"` entries of `<workspace>/comparegraph.toml`, in file order.
pub fn configured(workspace: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(workspace.join("comparegraph.toml")) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            cur = t
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .strip_prefix("providers.")
                .map(unquote);
        } else if let (Some(name), Some((k, v))) = (&cur, kv(t)) {
            if k == "command" {
                let p = PathBuf::from(unquote(&v));
                let p = if p.is_absolute() { p } else { workspace.join(p) };
                out.push((name.clone(), p));
            }
        }
    }
    out
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default()
}

fn is_file(p: &Path) -> bool {
    p.is_file()
}

impl ExternalProvider {
    fn with_manifest(workspace: &Path, name: &str, command: PathBuf) -> Self {
        let manifest = command
            .parent()
            .map(|d| d.join(MANIFEST_FILE))
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| parse_manifest(&t));
        Self { name: name.into(), command, manifest, workspace: workspace.to_path_buf() }
    }

    /// (1) `comparegraph.toml` in the workspace, then (2) `comparegraph-provider-<name>` on PATH.
    pub fn discover(workspace: &Path, name: &str) -> Option<Self> {
        if let Some((_, c)) = configured(workspace).into_iter().find(|(n, _)| n == name) {
            return Some(Self::with_manifest(workspace, name, c));
        }
        let bin = format!("{BIN_PREFIX}{name}");
        path_dirs()
            .into_iter()
            .map(|d| d.join(&bin))
            .find(|p| is_file(p))
            .map(|c| Self::with_manifest(workspace, name, c))
    }

    /// Every external provider `doctor` can see, with the last conformance result.
    pub fn list(workspace: &Path) -> Vec<ExternalInfo> {
        let mut names: Vec<String> = configured(workspace).into_iter().map(|(n, _)| n).collect();
        for d in path_dirs() {
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if let Some(n) = e.file_name().to_string_lossy().strip_prefix(BIN_PREFIX) {
                        if !n.is_empty() && !names.iter().any(|x| x == n) {
                            names.push(n.to_string());
                        }
                    }
                }
            }
        }
        names
            .into_iter()
            .filter_map(|n| Self::discover(workspace, &n))
            .map(|p| {
                let conformance = std::fs::read_to_string(conform_path(workspace, &p.name))
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|v| {
                        let names = |ok: bool| -> Vec<String> {
                            v["fixtures"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().filter(|f| (f["passed"] == true) == ok).filter_map(|f| f["name"].as_str().map(String::from)).collect()
                        };
                        let (conformed, failed) = (names(true), names(false));
                        let summary = format!("conformed: {}; failed: {}", conformed.join(", "), failed.join(", "));
                        serde_json::json!({ "passed": v["passed"], "fixtures": v["fixtures"].as_array().map(|a| a.len()), "conformed": conformed, "failed": failed, "summary": summary })
                    })
                    .unwrap_or(serde_json::Value::Null);
                let unavailable = p.unavailable_reason();
                ExternalInfo { name: p.name.clone(), command: p.command.display().to_string(), manifest: p.manifest_state(), conformance, unavailable }
            })
            .collect()
    }

    pub fn manifest_state(&self) -> String {
        match &self.manifest {
            None => "unverified".into(),
            Some(m) if m.protocol != PROTOCOL => format!("protocol_mismatch ({})", m.protocol),
            Some(m) => format!("ok ({} {})", m.name, m.version),
        }
    }

    fn allowed(&self) -> Result<(), String> {
        match &self.manifest {
            None if std::env::var("COMPAREGRAPH_ALLOW_UNVERIFIED").map(|v| v == "1").unwrap_or(false) => Ok(()),
            None => Err(format!(
                "no {MANIFEST_FILE} beside {}: unverified; pass --allow-unverified to use it",
                self.command.display()
            )),
            Some(m) if m.protocol != PROTOCOL => Err(format!("manifest protocol '{}' is not '{PROTOCOL}'", m.protocol)),
            Some(_) => Ok(()),
        }
    }

    /// The conform gate, per language. `languages` are the fixture names of the languages present in the repo (see
    /// `fixture_name`). With no result file the provider is unconformed and refused. With one, only a present language that has a
    /// failed fixture refuses the build; a language without a fixture is not judged. A result without a fixture list falls back to
    /// its overall `passed`. `--allow-unverified` waives all of it.
    pub fn conformance_gate(&self, languages: &std::collections::BTreeSet<String>) -> Result<(), String> {
        if std::env::var("COMPAREGRAPH_ALLOW_UNVERIFIED").map(|v| v == "1").unwrap_or(false) {
            return Ok(());
        }
        let hint = format!("run `comparegraph conform {0}` first, or pass --allow-unverified", self.name);
        let Some(v) = std::fs::read_to_string(conform_path(&self.workspace, &self.name))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        else {
            return Err(format!("{} has no conformance result: {hint}", self.name));
        };
        let fixtures = v["fixtures"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if fixtures.is_empty() {
            return if v["passed"] == true { Ok(()) } else { Err(format!("{} has no passing conformance result: {hint}", self.name)) };
        }
        let failed: Vec<&str> = fixtures
            .iter()
            .filter(|f| f["passed"] != true)
            .filter_map(|f| f["name"].as_str())
            .filter(|n| languages.contains(*n))
            .collect();
        if failed.is_empty() {
            Ok(())
        } else {
            Err(format!("{} failed conformance for {} (present in this repo): {hint}", self.name, failed.join(", ")))
        }
    }

    /// A plugin whose engine is missing answers `capabilities` with `"unavailable": "<reason + install hint>"`; `doctor` shows it
    /// and `build` refuses before it starts a run. `None` when the plugin is usable, or cannot be asked (unverified, not runnable).
    pub fn unavailable_reason(&self) -> Option<String> {
        let (d, _) = self.exchange_within(serde_json::json!({ "comparegraph": PROTOCOL, "op": "capabilities" }), None, Some(caps_timeout_secs())).ok()?;
        d["unavailable"].as_str().filter(|s| !s.is_empty()).map(String::from)
    }

    fn unavailable(&self, why: impl Into<String>) -> ProviderError {
        ProviderError::Unavailable(self.name.clone(), why.into())
    }

    /// One request line in, answer lines out. Returns (the `data` of the `ok` answer, progress file count).
    fn exchange(&self, request: serde_json::Value, repo_root: Option<&Path>) -> Result<(serde_json::Value, usize), ProviderError> {
        self.exchange_within(request, repo_root, None)
    }

    /// `limit` shortens the deadline below the build timeout (a capabilities probe must not hold `doctor` for 30 minutes).
    fn exchange_within(&self, request: serde_json::Value, repo_root: Option<&Path>, limit: Option<u64>) -> Result<(serde_json::Value, usize), ProviderError> {
        self.allowed().map_err(|e| self.unavailable(e))?;
        let secs = timeout_secs().map_err(|e| self.unavailable(e))?;
        let secs = limit.map_or(secs, |l| l.min(secs));
        let cap = output_cap();
        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut cmd = Command::new(&self.command);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(r) = repo_root {
            cmd.env("COMPAREGRAPH_REPO_ROOT", r);
        }
        cmd.env("COMPAREGRAPH_NETWORK", if self.manifest.as_ref().is_some_and(|m| m.network) { "1" } else { "0" });
        let mut child = cmd.spawn().map_err(|e| self.unavailable(format!("cannot start {}: {e}", self.command.display())))?;
        let mut stdin = child.stdin.take().unwrap();
        let line = format!("{request}\n");
        // a plugin that exits without reading its request is judged by its exit and output, not by the broken pipe
        let _ = stdin.write_all(line.as_bytes());
        drop(stdin);
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (err_tx, err_rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = std::io::Read::take(&mut stderr, 4096).read_to_end(&mut buf);
            let _ = err_tx.send(String::from_utf8_lossy(&buf).trim().to_string());
            // drain the rest so the child never blocks on a full pipe
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        });
        // stderr text is a courtesy: never wait on it longer than a moment (a grandchild may hold the pipe open)
        let err_text = |rx: &mpsc::Receiver<String>| rx.recv_timeout(Duration::from_millis(300)).unwrap_or_default();
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let mut rd = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let mut l = String::new();
                let room = cap.saturating_sub(total);
                // the cap applies while the line is read, so one endless line cannot be buffered
                match std::io::Read::take(&mut rd, room as u64 + 1).read_line(&mut l) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if total > cap {
                            let _ = tx.send(Err(format!("output exceeds the {cap}-byte cap")));
                            break;
                        }
                        if tx.send(Ok(l)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("unreadable output: {e}")));
                        break;
                    }
                }
            }
        });
        let mut progress = 0usize;
        let mut answer: Option<Result<serde_json::Value, ProviderError>> = None;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(Ok(l)) => {
                    let l = l.trim();
                    if l.is_empty() {
                        continue;
                    }
                    let v: serde_json::Value = match serde_json::from_str(l) {
                        Ok(v) => v,
                        Err(_) => {
                            answer = Some(Err(self.unavailable(format!("not JSON: {}", l.chars().take(80).collect::<String>()))));
                            break;
                        }
                    };
                    if let Some(p) = v.get("progress") {
                        progress = progress.max(p.get("files").and_then(|f| f.as_u64()).unwrap_or(0) as usize);
                    } else if v.get("ok") == Some(&serde_json::Value::Bool(true)) {
                        answer = Some(Ok(v.get("data").cloned().unwrap_or(serde_json::Value::Null)));
                        break;
                    } else if v.get("ok") == Some(&serde_json::Value::Bool(false)) {
                        let e = &v["error"];
                        answer = Some(Err(self.unavailable(format!(
                            "{}: {}",
                            e["code"].as_str().unwrap_or("error"),
                            e["message"].as_str().unwrap_or("")
                        ))));
                        break;
                    } else {
                        answer = Some(Err(self.unavailable("a line is neither progress nor an answer")));
                        break;
                    }
                }
                Ok(Err(why)) => {
                    answer = Some(Err(self.unavailable(why)));
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    answer = Some(Err(self.unavailable(format!("timed out after {secs} s (COMPAREGRAPH_PLUGIN_TIMEOUT)"))));
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let Some(answer) = answer else {
            // stdout closed without an answer: give the process a moment to exit, then kill it; never wait unbounded
            let grace = Instant::now() + Duration::from_secs(2).min(deadline.saturating_duration_since(Instant::now()));
            let status = loop {
                match child.try_wait() {
                    Ok(Some(s)) => break Some(s),
                    Ok(None) if Instant::now() < grace => std::thread::sleep(Duration::from_millis(10)),
                    _ => break None,
                }
            };
            let err = err_text(&err_rx);
            return match status {
                Some(s) => Err(self.unavailable(format!("exited ({s}) without an answer {err}"))),
                None => {
                    kill_tree(&mut child);
                    let _ = child.wait();
                    Err(self.unavailable(format!("closed stdout without an answer and kept running; killed {err}")))
                }
            };
        };
        if answer.is_err() {
            kill_tree(&mut child);
            let _ = child.wait();
            return answer.map(|d| (d, progress));
        }
        // the answer is in; the process must now exit cleanly within the deadline
        loop {
            match child.try_wait() {
                Ok(Some(s)) => {
                    if !s.success() {
                        let err = err_text(&err_rx);
                        return Err(self.unavailable(format!("exited with {s} {err}")));
                    }
                    break;
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
                _ => {
                    kill_tree(&mut child);
                    let _ = child.wait();
                    return Err(self.unavailable("did not exit after answering"));
                }
            }
        }
        answer.map(|d| (d, progress))
    }
}

/// Kill the child and, on Unix, its whole process group (the child leads one, see `process_group(0)`), so grandchildren die with it.
///
/// A direct `kill(2)` on the negated pid, never a shell-out: `/usr/bin/kill -KILL -1597919` (procps-ng) keeps only the first
/// digit of a negative argument after a signal option and calls `kill(-1, SIGKILL)` — every process of the user, on any host
/// whose pids exceed 1,000,000. The pid guard keeps `-1` (everyone) and `0` (our own group) out.
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id();
        if pid > 1 && pid <= i32::MAX as u32 {
            // SAFETY: kill(2) takes a pid and a signal; no memory is touched. A negative pid addresses that process group.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
    let _ = child.kill();
}

fn str_of(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

impl GraphProvider for ExternalProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> String {
        self.manifest.as_ref().map(|m| m.version.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| "unknown".into())
    }
    fn detect(&self) -> ProviderStatus {
        let (installed, detail) = if !self.command.is_file() {
            (false, format!("{} not found", self.command.display()))
        } else {
            let engine = match self.allowed() {
                Ok(()) => self.unavailable_reason().map_or(Ok(()), Err),
                Err(e) => Err(e),
            };
            match engine.and_then(|_| timeout_secs().map(|_| ())) {
                Ok(()) => (true, format!("external, protocol {PROTOCOL}: {}", self.command.display())),
                Err(e) => (false, e),
            }
        };
        ProviderStatus { installed, version: self.version(), detail }
    }
    fn capabilities(&self) -> Capabilities {
        let empty = Capabilities { languages: vec![], edge_kinds: vec![], communities: false, embeddings: false };
        let Ok((d, _)) = self.exchange_within(serde_json::json!({ "comparegraph": PROTOCOL, "op": "capabilities" }), None, Some(caps_timeout_secs())) else {
            return empty;
        };
        let strs = |k: &str| d[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_else(Vec::<String>::new);
        Capabilities {
            languages: strs("languages"),
            edge_kinds: strs("edge_kinds").iter().filter_map(|k| EdgeKind::parse(k)).collect(),
            communities: d["communities"].as_bool().unwrap_or(false),
            embeddings: d["embeddings"].as_bool().unwrap_or(false),
        }
    }
    fn build(&self, repo_root: &Path, opts: &BuildOptions) -> Result<BuildOutput, ProviderError> {
        let req = serde_json::json!({
            "comparegraph": PROTOCOL, "op": "build", "repo_root": repo_root.display().to_string(),
            "options": { "max_file_bytes": opts.max_file_bytes, "max_files": opts.max_files, "full": opts.full },
        });
        let (data, progress) = self.exchange(req, Some(repo_root))?;
        if !data.is_object() {
            return Err(self.unavailable("build answer has no data object"));
        }
        let mut out = BuildOutput::default();
        let (mut bad_nodes, mut bad_edges, mut bad_paths) = (0usize, 0usize, 0usize);
        let mut bad_qns = std::collections::HashSet::<String>::new();
        for n in data["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let Some(kind) = NodeKind::parse(&str_of(n, "kind")) else {
                bad_nodes += 1;
                continue;
            };
            let qn = str_of(n, "qualified_name");
            if qn.is_empty() {
                bad_nodes += 1;
                continue;
            }
            if !path_ok(&str_of(n, "path")) {
                bad_paths += 1;
                bad_qns.insert(qn);
                continue;
            }
            let lines = n.get("lines").and_then(|l| l.as_array());
            let line = |k: &str, i: usize| -> u32 {
                n.get(k).and_then(|x| x.as_u64()).or_else(|| lines.and_then(|l| l.get(i)).and_then(|x| x.as_u64())).unwrap_or(0) as u32
            };
            let name = Some(str_of(n, "name")).filter(|s| !s.is_empty()).unwrap_or_else(|| qn.rsplit(['.', ':', '/']).next().unwrap_or(&qn).to_string());
            let ch = Some(str_of(n, "content_hash")).filter(|s| !s.is_empty()).unwrap_or_else(|| content_hash(&[&qn]));
            out.nodes.push(Node {
                id: None,
                kind,
                name,
                qualified_name: qn,
                path: str_of(n, "path"),
                line_start: line("line_start", 0),
                line_end: line("line_end", 1),
                content_hash: ch,
                language: str_of(n, "language"),
                provider_ref: Some(str_of(n, "provider_ref")).filter(|s| !s.is_empty()),
            });
        }
        for e in data["edges"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let Some(kind) = EdgeKind::parse(&str_of(e, "kind")) else {
                bad_edges += 1;
                continue;
            };
            let (s, t) = (str_of(e, "source_qn"), str_of(e, "target_qn"));
            if s.is_empty() || t.is_empty() {
                bad_edges += 1;
                continue;
            }
            if bad_qns.contains(&s) || bad_qns.contains(&t) {
                bad_paths += 1;
                continue;
            }
            let c = e.get("confidence").and_then(|c| c.as_f64()).filter(|c| (0.0..=1.0).contains(c)).unwrap_or(0.5);
            out.edges_by_qn.push((kind, s, t, c));
        }
        for l in data["limitations"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            out.limitations.push(Limitation::new(
                l.get("code").and_then(|c| c.as_str()).unwrap_or("plugin"),
                l.get("detail").and_then(|c| c.as_str()).or_else(|| l.as_str()).unwrap_or(""),
            ));
        }
        if bad_paths > 0 {
            out.limitations.push(Limitation::new(
                "bad_path",
                format!("{bad_paths} node(s)/edge(s) dropped: a path that is absolute, has a '..' segment or a NUL, or an edge to such a node"),
            ));
        }
        if bad_nodes + bad_edges > 0 {
            out.limitations.push(Limitation::new(
                "unknown_kind",
                format!("{bad_nodes} node(s) and {bad_edges} edge(s) dropped: a kind (or an endpoint) this host does not know"),
            ));
        }
        let file_nodes = out.nodes.iter().filter(|n| n.kind == NodeKind::File).count();
        let paths = out.nodes.iter().map(|n| n.path.as_str()).collect::<std::collections::HashSet<_>>().len();
        let inferred = data["files_parsed"].as_u64().is_none();
        out.files_seen = data["files_seen"].as_u64().map(|x| x as usize).unwrap_or(progress.max(file_nodes).max(paths));
        out.files_parsed = data["files_parsed"].as_u64().map(|x| x as usize).unwrap_or(file_nodes.max(if out.nodes.is_empty() { 0 } else { paths }));
        if inferred {
            out.limitations.push(Limitation::new("files_parsed_inferred", format!("the plugin gave no files_parsed; {} inferred from its file nodes and paths", out.files_parsed)));
        }
        Ok(out)
    }
}

pub fn conform_path(workspace: &Path, name: &str) -> PathBuf {
    workspace.join(".comparegraph").join(format!("conform-{name}.json"))
}

/// The conformance fixture a walker language id belongs to (`typescript` covers tsx, javascript, jsx and mjs).
pub fn fixture_name(language: &str) -> &str {
    match language {
        "tsx" | "javascript" | "jsx" | "mjs" => "typescript",
        l => l,
    }
}
