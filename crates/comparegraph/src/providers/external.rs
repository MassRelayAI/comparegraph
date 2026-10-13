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
}

/// What `doctor` reports for one discovered external provider.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExternalInfo {
    pub name: String,
    pub command: String,
    pub manifest: String,
    pub conformance: serde_json::Value,
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
    fn with_manifest(name: &str, command: PathBuf) -> Self {
        let manifest = command
            .parent()
            .map(|d| d.join(MANIFEST_FILE))
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| parse_manifest(&t));
        Self { name: name.into(), command, manifest }
    }

    /// (1) `comparegraph.toml` in the workspace, then (2) `comparegraph-provider-<name>` on PATH.
    pub fn discover(workspace: &Path, name: &str) -> Option<Self> {
        if let Some((_, c)) = configured(workspace).into_iter().find(|(n, _)| n == name) {
            return Some(Self::with_manifest(name, c));
        }
        let bin = format!("{BIN_PREFIX}{name}");
        path_dirs()
            .into_iter()
            .map(|d| d.join(&bin))
            .find(|p| is_file(p))
            .map(|c| Self::with_manifest(name, c))
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
                    .map(|v| serde_json::json!({ "passed": v["passed"], "fixtures": v["fixtures"].as_array().map(|a| a.len()) }))
                    .unwrap_or(serde_json::Value::Null);
                ExternalInfo { name: p.name.clone(), command: p.command.display().to_string(), manifest: p.manifest_state(), conformance }
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

    fn unavailable(&self, why: impl Into<String>) -> ProviderError {
        ProviderError::Unavailable(self.name.clone(), why.into())
    }

    /// One request line in, answer lines out. Returns (the `data` of the `ok` answer, progress file count).
    fn exchange(&self, request: serde_json::Value, repo_root: Option<&Path>) -> Result<(serde_json::Value, usize), ProviderError> {
        self.allowed().map_err(|e| self.unavailable(e))?;
        let secs = std::env::var("COMPAREGRAPH_PLUGIN_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_TIMEOUT_SECS);
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
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = std::io::Read::take(&mut stderr, 4096).read_to_end(&mut buf);
            // drain the rest so the child never blocks on a full pipe
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            String::from_utf8_lossy(&buf).trim().to_string()
        });
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        std::thread::spawn(move || {
            let mut rd = BufReader::new(stdout);
            let mut total = 0usize;
            loop {
                let mut l = String::new();
                match rd.read_line(&mut l) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if total > OUTPUT_CAP {
                            let _ = tx.send(Err(format!("output exceeds the {} MiB cap", OUTPUT_CAP / 1024 / 1024)));
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
            let status = child.wait().ok();
            let err = err_thread.join().unwrap_or_default();
            return Err(self.unavailable(format!("exited ({}) without an answer {err}", status.map(|s| s.to_string()).unwrap_or_default())));
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
                        let err = err_thread.join().unwrap_or_default();
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
/// digit of a negative argument after a signal option and calls `kill(-1, SIGKILL)` — every process of the user. That spelling
/// logged the whole desktop out five times on 2026-10-13 on a host whose pids exceed 1,000,000
/// (docs/reports/2026-10-13-user-wide-kill-from-kill-tree.md). The pid guard keeps `-1` (everyone) and `0` (our own group) out.
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
            match self.allowed() {
                Ok(()) => (true, format!("external, protocol {PROTOCOL}: {}", self.command.display())),
                Err(e) => (false, e),
            }
        };
        ProviderStatus { installed, version: self.version(), detail }
    }
    fn capabilities(&self) -> Capabilities {
        let empty = Capabilities { languages: vec![], edge_kinds: vec![], communities: false, embeddings: false };
        let Ok((d, _)) = self.exchange(serde_json::json!({ "comparegraph": PROTOCOL, "op": "capabilities" }), None) else {
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
        let (mut bad_nodes, mut bad_edges) = (0usize, 0usize);
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
            let c = e.get("confidence").and_then(|c| c.as_f64()).filter(|c| (0.0..=1.0).contains(c)).unwrap_or(0.5);
            out.edges_by_qn.push((kind, s, t, c));
        }
        for l in data["limitations"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            out.limitations.push(Limitation::new(
                l.get("code").and_then(|c| c.as_str()).unwrap_or("plugin"),
                l.get("detail").and_then(|c| c.as_str()).or_else(|| l.as_str()).unwrap_or(""),
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
