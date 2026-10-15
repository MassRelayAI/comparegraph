#![allow(clippy::result_large_err)] // serde_json::Value is larger with arbitrary_precision
use crate::adapters::envelope::Envelope;
use crate::application::graph as g;
use crate::application::{
    add::add_repo,
    build::build_repo,
    check::check,
    export::{export_csv, export_json},
    setup,
    search::{search, SearchQuery},
    AppError,
};
use crate::providers::{by_name, BuildOptions};
use crate::store::Store;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "comparegraph",
    version,
    about = "Many clones, one taxonomy, evidence on every cell"
)]
pub struct Cli {
    #[arg(long, global = true, default_value = ".")]
    pub root: PathBuf,
    #[arg(long, global = true)]
    pub pretty: bool,
    /// use an external provider that has no comparegraph-plugin.toml beside it
    #[arg(long, global = true)]
    pub allow_unverified: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}
#[derive(Subcommand)]
pub enum Cmd {
    /// Register a clone (read, never run)
    Add {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// register this clone as the workspace's root (spec §2b product/baseline modes)
        #[arg(long)]
        as_root: bool,
        /// re-point the registered repo `--name` at this path (claims and runs stay)
        #[arg(long, requires = "name")]
        replace: bool,
    },
    /// Parse a registered clone into the graph (one run)
    Build {
        repo: String,
        #[arg(long, default_value = "reference")]
        provider: String,
        /// build with several providers in turn (comma-separated, e.g. reference,crg) and score their agreement against the first
        #[arg(long, value_delimiter = ',')]
        providers: Vec<String>,
        #[arg(long, default_value_t = 1_048_576)]
        max_file_bytes: u64,
        #[arg(long, default_value_t = 50_000)]
        max_files: usize,
        /// bypass the per-file parse cache
        #[arg(long)]
        full: bool,
        /// print progress lines to stderr while building (also COMPAREGRAPH_PROGRESS=1); stdout stays one envelope
        #[arg(long)]
        progress: bool,
    },
    /// Verify: schema, determinism, locations
    Check,
    /// Export the graph
    Export {
        #[command(subcommand)]
        what: ExportWhat,
    },
    /// Full-text search over symbols of each repo's latest run
    Search {
        query: String,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// A symbol with callers, callees, community, licence lane; --detail full adds its current lines
    Show {
        repo: String,
        qualified_name: String,
        #[arg(long)]
        detail: Option<String>,
        /// cap on the callers and on the callees listed (1..=2000); a truncated list says so in the limitations
        #[arg(long, default_value_t = 50)]
        max_callers: usize,
    },
    /// Nodes reachable from a symbol over calls/extends/implements, with hop distance and the path to each
    Traverse {
        repo: String,
        qualified_name: String,
        #[arg(long, default_value = "both")]
        direction: String,
        #[arg(long, default_value_t = 3)]
        depth: usize,
        #[arg(long, default_value_t = 200)]
        max_nodes: usize,
        #[arg(long, default_value = "calls,extends,implements")]
        kinds: String,
        #[arg(long, default_value_t = 0.5)]
        min_confidence: f64,
        /// page size of the items (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Everything that transitively depends on a symbol or file(s), by hop, plus the tests that reach it
    Impact {
        repo: String,
        qualified_name: Option<String>,
        #[arg(long = "file")]
        file: Vec<String>,
        #[arg(long, default_value_t = 5)]
        depth: usize,
        #[arg(long, default_value_t = 500)]
        max_nodes: usize,
        #[arg(long, default_value_t = 0.5)]
        min_confidence: f64,
        /// page size of the items (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Tests linked to a symbol directly or through at most 3 call hops
    TestsFor {
        repo: String,
        qualified_name: String,
        #[arg(long, default_value_t = 200)]
        max_nodes: usize,
        #[arg(long, default_value_t = 0.5)]
        min_confidence: f64,
        /// page size of the items (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Symbols nothing calls that are exported, handlers or main
    EntryPoints {
        repo: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 0.5)]
        min_confidence: f64,
    },
    /// Candidates nothing references (dynamic dispatch is invisible)
    DeadCode {
        repo: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 0.5)]
        min_confidence: f64,
    },
    /// Report the store, every provider and each agent client's registration of the MCP server
    Doctor {
        /// compare registrations against this command instead of the running executable
        #[arg(long)]
        bin: Option<String>,
        /// one or two plain-text lines for a session-start hook (the one output that is not an envelope; --pretty is ignored): nothing without a store, else the repo count and the server registration of each detected client
        #[arg(long)]
        brief: bool,
    },
    /// Register the MCP server with agent clients: a plan by default, `--apply` writes (additive, backed up)
    Setup {
        /// claude, codex, cursor, gemini, opencode, hermes, windsurf, antigravity, kiro, copilot, copilot-cli, qwen, or all (installed clients only)
        #[arg(long, default_value = "all")]
        client: String,
        /// user (the client's own config) or project (the current directory)
        #[arg(long, default_value = "user")]
        scope: String,
        #[arg(long)]
        apply: bool,
        /// replace a `comparegraph` entry that differs: another workspace or shape (conflict) or a moved binary (stale); both are left untouched without it
        #[arg(long)]
        force: bool,
        /// the command to register (default: this binary)
        #[arg(long)]
        bin: Option<String>,
        /// do not write the CompareGraph block into the project's agent instruction file (CLAUDE.md, AGENTS.md, GEMINI.md, ...)
        #[arg(long)]
        no_rules: bool,
        /// do not register Claude Code's SessionStart hook
        #[arg(long)]
        no_hook: bool,
    },
    /// Print the MCP tool catalog as JSON (redirect to docs/catalog.json); --check compares it with the committed file
    Catalog {
        /// exit 1 (error catalog_stale) when the file differs byte-for-byte from the live catalog
        #[arg(long)]
        check: bool,
        /// the file --check compares against (default docs/catalog.json, relative to the current directory)
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Registered repos with their latest run summary
    Repos {
        /// page size (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Module clusters of a repo's latest run
    Communities {
        repo: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Serve the read verbs as an MCP stdio server (JSON-RPC, one object per line)
    Mcp,
    /// Candidate symbols and communities per feature of a family in one clone; writes nothing
    Map {
        repo: String,
        family: String,
        #[arg(long)]
        feature: Option<String>,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// Is a feature absent from a clone's graph? Four channels (symbols, paths, communities, kinds); reads, writes nothing
    AbsentCheck {
        repo: String,
        family: String,
        feature: String,
    },
    /// Score each clone against each family (related|adjacent|unrelated) and record the band
    Relate {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        family: Option<String>,
        /// read the latest stored rows; compute and record nothing
        #[arg(long)]
        stored: bool,
    },
    /// Record a claim (has|partial|none|unknown) about a feature in a clone, with citations that resolve now
    Claim {
        #[arg(required_unless_present = "batch")]
        repo: Option<String>,
        #[arg(required_unless_present = "batch")]
        feature: Option<String>,
        #[arg(required_unless_present = "batch")]
        verdict: Option<String>,
        #[arg(long, required_unless_present = "batch")]
        author: Option<String>,
        /// JSON lines, one claim per line (fields as the flags; `at` is an array), `-` for stdin; one store open, one
        /// transaction, one envelope line out per row in, then a summary line
        #[arg(long, value_name = "JSONL|-")]
        batch: Option<String>,
        #[arg(long)]
        family: Option<String>,
        #[arg(long = "at")]
        at: Vec<String>,
        #[arg(long)]
        searched: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Latest claim per (clone, feature); --history lists every row with its supersede chain
    Claims {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        family: Option<String>,
        #[arg(long)]
        feature: Option<String>,
        #[arg(long)]
        history: bool,
    },
    /// Record a decision (adopt|reject|park) about a clone, or one feature of it; append-only
    Decide {
        #[arg(required_unless_present = "batch")]
        repo: Option<String>,
        #[arg(required_unless_present = "batch")]
        disposition: Option<String>,
        #[arg(long, required_unless_present = "batch")]
        why: Option<String>,
        #[arg(long, required_unless_present = "batch")]
        by: Option<String>,
        /// JSON lines, one decision per line (fields as the flags), `-` for stdin; one store open, one transaction,
        /// one envelope line out per row in, then a summary line
        #[arg(long, value_name = "JSONL|-")]
        batch: Option<String>,
        #[arg(long)]
        feature: Option<String>,
        /// disambiguates --feature when its id exists in several families
        #[arg(long)]
        family: Option<String>,
    },
    /// Latest decision per scope; --history lists every row
    Decisions {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        history: bool,
    },
    /// Active features of a family against every clone: verdicts, prevalence, gaps and differentiators
    Matrix {
        family: String,
        /// score the family's comparators that have no stored relatedness band first
        #[arg(long)]
        relate: bool,
    },
    /// Taxonomy packs: add, list, show
    Family {
        #[command(subcommand)]
        what: FamilyCmd,
    },
    /// Run the golden fixtures against a provider
    Conform {
        provider: String,
        #[arg(long, default_value = "fixtures")]
        fixtures: PathBuf,
    },
}
#[derive(Subcommand)]
pub enum FamilyCmd {
    /// Read a pack directory (rows.md) into the store; features it no longer lists become retired
    Add { dir: PathBuf },
    /// Registered families
    List,
    /// One family with its features by group
    Show { name: String },
    /// Write a draft taxonomy (rows.draft.md) from the terms the given repos share; no model
    Propose {
        name: String,
        #[arg(long, value_delimiter = ',', required = true)]
        repos: Vec<String>,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        min_repos: usize,
        #[arg(long, default_value_t = 60)]
        max: usize,
    },
}
#[derive(Subcommand)]
pub enum ExportWhat {
    Json {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// RFC 4180 CSV files (LF line endings): nodes, edges, claims, decisions, one matrix per family
    Csv {
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        out: PathBuf,
    },
    /// An Obsidian-style markdown vault, regenerated whole
    Vault {
        #[arg(long)]
        out: PathBuf,
    },
}

fn verb_of(c: &Cmd) -> &'static str {
    match c {
        Cmd::Add { .. } => "add",
        Cmd::Build { .. } => "build",
        Cmd::Check => "check",
        Cmd::Export { .. } => "export",
        Cmd::Search { .. } => "search",
        Cmd::Show { .. } => "show",
        Cmd::Traverse { .. } => "traverse",
        Cmd::Impact { .. } => "impact",
        Cmd::TestsFor { .. } => "tests-for",
        Cmd::EntryPoints { .. } => "entry-points",
        Cmd::DeadCode { .. } => "dead-code",
        Cmd::Doctor { .. } => "doctor",
        Cmd::Setup { .. } => "setup",
        Cmd::Catalog { .. } => "catalog",
        Cmd::Repos { .. } => "repos",
        Cmd::Mcp => "mcp",
        Cmd::Communities { .. } => "communities",
        Cmd::Conform { .. } => "conform",
        Cmd::Family { .. } => "family",
        Cmd::Map { .. } => "map",
        Cmd::AbsentCheck { .. } => "absent-check",
        Cmd::Relate { .. } => "relate",
        Cmd::Claim { .. } => "claim",
        Cmd::Claims { .. } => "claims",
        Cmd::Decide { .. } => "decide",
        Cmd::Decisions { .. } => "decisions",
        Cmd::Matrix { .. } => "matrix",
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    if cli.allow_unverified {
        std::env::set_var("COMPAREGRAPH_ALLOW_UNVERIFIED", "1");
    }
    if matches!(cli.cmd, Cmd::Mcp) {
        crate::adapters::mcp::serve(&cli.root, std::io::stdin().lock(), std::io::stdout().lock());
        return 0;
    }
    if let Cmd::Catalog { check: false, .. } = cli.cmd {
        print!("{}", crate::application::catalog::render(crate::adapters::mcp::tools()));
        return 0;
    }
    if let Cmd::Claim { batch: Some(b), .. } | Cmd::Decide { batch: Some(b), .. } = &cli.cmd {
        return run_batch_cli(&cli, b);
    }
    if let Cmd::Doctor { bin, brief: true } = &cli.cmd {
        print!("{}", doctor_brief(&cli.root, bin.as_deref()));
        return 0;
    }
    let env = envelope_for(&cli);
    if cli.pretty {
        let v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
    } else {
        println!("{}", env.to_json());
    }
    if env.ok {
        0
    } else {
        1
    }
}

/// `doctor --brief`: empty without a store; otherwise the repo count and the server registration of each detected client.
fn doctor_brief(root: &std::path::Path, bin: Option<&str>) -> String {
    // a zero-byte file is no store: opening it would migrate it into one
    if std::fs::metadata(root.join(".comparegraph").join("comparegraph.db")).map_or(true, |m| !m.is_file() || m.len() == 0) {
        return String::new();
    }
    let Ok(store) = Store::open_existing(root) else { return String::new() };
    let Ok(repos) = store.repos() else { return String::new() };
    let ctx = setup::Ctx::from_env(root, bin);
    let found = setup::detected_states(&ctx);
    let off: Vec<_> = found.iter().filter(|(_, s)| !matches!(s, setup::State::Aligned | setup::State::Plugin)).collect();
    let server = if found.is_empty() {
        "no agent client detected".to_string()
    } else if off.is_empty() {
        let names: Vec<_> = found.iter().map(|(c, _)| c.name()).collect();
        format!("server aligned for {}", names.join(", "))
    } else {
        let all: Vec<_> = found.iter().map(|(c, s)| format!("{} {}", c.name(), s.name())).collect();
        format!("server: {}", all.join(", "))
    };
    let mut out = format!("CompareGraph: {} repos, store {}; {server}\n", repos.len(), store.path().display());
    if !off.is_empty() {
        let list: Vec<_> = off.iter().map(|(c, s)| format!("{} {}", c.name(), s.name())).collect();
        // `--apply` alone only adds what is missing; a stale or conflicting entry needs `--force`
        let force = if off.iter().all(|(_, s)| *s == setup::State::Missing) { "" } else { " --force" };
        out.push_str(&format!("Run `comparegraph setup --apply{force}` (state: {}).\n", list.join(", ")));
    }
    out
}

fn print_line(v: &serde_json::Value, pretty: bool) {
    if pretty {
        println!("{}", serde_json::to_string_pretty(v).unwrap());
    } else {
        println!("{v}");
    }
}

/// `claim --batch` / `decide --batch`: one envelope per input row in input order (with its `line`), printed as each
/// row is decided, then a summary envelope `{rows, ok, skipped, failed, committed}`; an error envelope as the last line means the
/// transaction did not commit and no row landed. Exit 0 only when every row is ok.
fn run_batch_cli(cli: &Cli, path: &str) -> i32 {
    use std::io::BufRead;
    let verb = verb_of(&cli.cmd);
    let stray = match &cli.cmd {
        Cmd::Claim { repo, feature, verdict, author, family, at, searched, note, .. } => {
            !(repo.is_none() && feature.is_none() && verdict.is_none() && author.is_none()
                && family.is_none() && at.is_empty() && searched.is_none() && note.is_none())
        }
        Cmd::Decide { repo, disposition, why, by, feature, family, .. } => {
            !(repo.is_none() && disposition.is_none() && why.is_none() && by.is_none()
                && feature.is_none() && family.is_none())
        }
        _ => false,
    };
    let fail = |e: AppError| -> i32 {
        eprintln!("comparegraph: {e}");
        print_line(&serde_json::from_str(&Envelope::err(verb, &e).to_json()).unwrap(), cli.pretty);
        1
    };
    if stray {
        return fail(AppError::InvalidArguments(
            "--batch takes its rows from the input; do not also pass positionals or row flags".into(),
        ));
    }
    let reader: Box<dyn BufRead> = if path == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin().lock()))
    } else {
        match std::fs::File::open(path) {
            Ok(f) => Box::new(std::io::BufReader::new(f)),
            Err(e) => return fail(AppError::Io(PathBuf::from(path), e)),
        }
    };
    let pretty = cli.pretty;
    let print_row = |line: usize, res: Result<serde_json::Value, (&'static str, String)>, conf: &'static str| {
        let env = match res {
            Ok(v) => Envelope::ok(verb, v, conf, vec![], None),
            Err((code, msg)) => {
                if msg.starts_with("line ") {
                    eprintln!("comparegraph: {msg}");
                } else {
                    eprintln!("comparegraph: line {line}: {msg}");
                }
                let mut e = Envelope::err(verb, &AppError::InvalidArguments(msg));
                if let Some(b) = e.error.as_mut() {
                    b.code = code;
                }
                e
            }
        };
        let mut v: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
        v["line"] = serde_json::json!(line);
        print_line(&v, pretty);
    };
    let summary = match &cli.cmd {
        Cmd::Claim { .. } => crate::application::claim::claim_batch(&cli.root, reader, |r| {
            let conf = match r.data.as_ref().map(|d| d.confidence.as_str()) {
                Some("derived") => "derived",
                Some("inferred") => "inferred",
                _ => "unknown",
            };
            print_row(r.line, batch_value(r.data, r.error), conf)
        }),
        _ => crate::application::decide::decide_batch(&cli.root, reader, |r| {
            print_row(r.line, batch_value(r.data, r.error), "observed")
        }),
    };
    // the last line is the summary: rows are durable only when it says committed
    let sum = match summary {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let mut summary = Envelope::ok(
        verb,
        serde_json::json!({ "rows": sum.rows, "ok": sum.ok, "skipped": sum.skipped, "failed": sum.failed, "committed": true }),
        "observed",
        vec![],
        None,
    );
    summary.ok = sum.failed == 0;
    print_line(&serde_json::from_str(&summary.to_json()).unwrap(), cli.pretty);
    if sum.failed == 0 {
        0
    } else {
        1
    }
}

fn batch_value<T: serde::Serialize>(
    data: Option<T>,
    error: Option<crate::application::claim::BatchError>,
) -> Result<serde_json::Value, (&'static str, String)> {
    match (data, error) {
        (Some(d), _) => Ok(serde_json::to_value(d).unwrap()),
        (None, Some(e)) => Err((e.code, e.message)),
        (None, None) => Err(("internal", "row has neither data nor error".into())),
    }
}

/// Run one verb and wrap the outcome in the envelope; shared by the CLI and the MCP adapter.
pub fn envelope_for(cli: &Cli) -> Envelope {
    let verb = verb_of(&cli.cmd);
    match dispatch(cli) {
        Ok(v) => {
            let conf = if matches!(
                cli.cmd,
                Cmd::Check | Cmd::Claims { .. } | Cmd::Matrix { .. }
            ) {
                "derived"
            } else if matches!(cli.cmd, Cmd::Claim { .. }) {
                match v["confidence"].as_str() {
                    Some("derived") => "derived",
                    Some("inferred") => "inferred",
                    _ => "unknown",
                }
            } else if let Some(c) = v["evidence_confidence"].as_str() {
                match c {
                    "derived" => "derived",
                    "inferred" => "inferred",
                    "unknown" => "unknown",
                    _ => "observed",
                }
            } else if matches!(cli.cmd, Cmd::Show { .. }) {
                if v["stale"] == serde_json::Value::Bool(true) {
                    "unknown"
                } else {
                    "observed"
                }
            } else {
                "observed"
            };
            let mut v = v;
            if let Some(o) = v.as_object_mut() {
                o.remove("evidence_confidence");
            }
            let lims: Vec<crate::domain::Limitation> = v
                .as_object_mut()
                .and_then(|o| o.remove("limitation_items"))
                .and_then(|x| serde_json::from_value(x).ok())
                .unwrap_or_default();
            let page = v
                .as_object_mut()
                .and_then(|o| o.remove("page"))
                .and_then(|x| serde_json::from_value::<crate::adapters::envelope::Page>(x).ok());
            Envelope::ok(verb, v, conf, lims, page)
        }
        Err((e, data)) => {
            eprintln!("comparegraph: {e}");
            let mut env = Envelope::err(verb, &e);
            if matches!(cli.cmd, Cmd::Catalog { .. }) && matches!(e, AppError::CheckFailed(_)) {
                if let Some(b) = env.error.as_mut() {
                    b.code = "catalog_stale";
                    b.hint = Some("run `comparegraph catalog > docs/catalog.json`".into());
                }
            }
            if let Some(d) = data {
                env.data = d;
            }
            env
        }
    }
}

fn progress_on(flag: bool) -> bool {
    flag || std::env::var_os("COMPAREGRAPH_PROGRESS").is_some_and(|v| v != "0" && !v.is_empty())
}

type Failure = (AppError, Option<serde_json::Value>);

fn dispatch(cli: &Cli) -> Result<serde_json::Value, Failure> {
    dispatch_inner(cli).map_err(|e| (e, None)).and_then(|r| r)
}

fn dispatch_inner(cli: &Cli) -> Result<Result<serde_json::Value, Failure>, AppError> {
    Ok(Ok(match &cli.cmd {
        Cmd::Add {
            path,
            name,
            as_root,
            replace,
        } if *replace => serde_json::to_value(crate::application::add::replace_repo(&cli.root, path, name.as_deref().unwrap_or_default(), *as_root)?).unwrap(),
        Cmd::Add {
            path,
            name,
            as_root,
            ..
        } => serde_json::to_value(add_repo(&cli.root, path, name.as_deref(), *as_root)?).unwrap(),
        Cmd::Build {
            repo,
            provider,
            providers,
            max_file_bytes,
            max_files,
            full,
            progress,
        } if !providers.is_empty() => {
            let rep = crate::application::build::build_many(
                &cli.root,
                repo,
                providers,
                &BuildOptions { max_file_bytes: *max_file_bytes, max_files: *max_files, full: *full, progress: progress_on(*progress), ..BuildOptions::default() },
            )?;
            eprint!("{}", crate::application::agreement::table(&rep.agreement));
            serde_json::to_value(rep).unwrap()
        }
        Cmd::Build {
            repo,
            provider,
            max_file_bytes,
            max_files,
            full,
            progress,
            ..
        } => serde_json::to_value(build_repo(
            &cli.root,
            repo,
            provider,
            &BuildOptions {
                max_file_bytes: *max_file_bytes,
                max_files: *max_files,
                full: *full,
                progress: progress_on(*progress),
                ..BuildOptions::default()
            },
        )?)
        .unwrap(),
        Cmd::Check => {
            let r = check(&cli.root)?;
            let v = serde_json::to_value(&r).unwrap();
            if !r.ok() {
                return Ok(Err((
                    AppError::CheckFailed(format!("{} problem(s)", r.problems.len())),
                    Some(v),
                )));
            }
            v
        }
        Cmd::Export {
            what: ExportWhat::Json { repo, out },
        } => {
            let v = export_json(&cli.root, repo.as_deref())?;
            if let Some(p) = out {
                std::fs::write(p, serde_json::to_string_pretty(&v).unwrap())
                    .map_err(|e| AppError::Io(p.clone(), e))?;
                serde_json::json!({ "written": p, "repos": v["repos"].as_array().map(|a| a.len()).unwrap_or(0) })
            } else {
                v
            }
        }
        Cmd::Export {
            what: ExportWhat::Csv { repo, out },
        } => export_csv(&cli.root, repo.as_deref(), out)?,
        Cmd::Export {
            what: ExportWhat::Vault { out },
        } => {
            let r = crate::application::vault::export_vault(&cli.root, out)?;
            serde_json::json!({ "written": out, "repos": r.repos, "files": r.files, "communities": r.communities, "symbols": r.symbols })
        }
        Cmd::Search {
            query,
            repo,
            kind,
            limit,
            cursor,
        } => {
            let kind = match kind {
                Some(k) => Some(
                    crate::domain::NodeKind::parse(k)
                        .ok_or_else(|| AppError::Empty(format!("unknown kind {k}")))?,
                ),
                None => None,
            };
            let offset = match cursor {
                Some(c) => c
                    .parse::<usize>()
                    .map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
                None => 0,
            };
            let p = search(
                &cli.root,
                &SearchQuery {
                    text: query.clone(),
                    repo: repo.clone(),
                    kind,
                    limit: (*limit).clamp(1, 500),
                    offset,
                },
            )?;
            serde_json::json!({ "items": p.items, "page": { "next": p.next, "total": p.total } })
        }
        Cmd::Show {
            repo,
            qualified_name,
            detail,
            max_callers,
        } => {
            let full = match detail.as_deref() {
                None | Some("compact") => false,
                Some("full") => true,
                Some(d) => {
                    return Err(AppError::Empty(format!(
                        "unknown detail {d} (compact|full)"
                    )))
                }
            };
            serde_json::to_value(crate::application::show::show_with(&cli.root, repo, qualified_name, full, *max_callers)?).unwrap()
        }
        Cmd::Traverse {
            repo,
            qualified_name,
            direction,
            depth,
            max_nodes,
            kinds,
            min_confidence,
            limit,
            cursor,
        } => g::traverse(
            &cli.root,
            repo,
            &g::TraverseOpts {
                qualified_name,
                direction: g::Direction::parse(direction)?,
                depth: *depth,
                max_nodes: *max_nodes,
                kinds: g::parse_kinds(kinds)?,
                min_conf: *min_confidence,
                limit: *limit,
                cursor: cursor.as_deref(),
            },
        )?,
        Cmd::Impact {
            repo,
            qualified_name,
            file,
            depth,
            max_nodes,
            min_confidence,
            limit,
            cursor,
        } => g::impact(
            &cli.root,
            repo,
            &g::ImpactOpts {
                qualified_name: qualified_name.as_deref(),
                files: file,
                depth: *depth,
                max_nodes: *max_nodes,
                min_conf: *min_confidence,
                limit: *limit,
                cursor: cursor.as_deref(),
            },
        )?,
        Cmd::TestsFor {
            repo,
            qualified_name,
            max_nodes,
            min_confidence,
            limit,
            cursor,
        } => g::tests_for(&cli.root, repo, qualified_name, *max_nodes, *min_confidence, *limit, cursor.as_deref())?,
        Cmd::EntryPoints {
            repo,
            limit,
            cursor,
            min_confidence,
        } => g::entry_points(&cli.root, repo, *limit, cursor.as_deref(), *min_confidence)?,
        Cmd::DeadCode {
            repo,
            limit,
            cursor,
            min_confidence,
        } => g::dead_code(&cli.root, repo, *limit, cursor.as_deref(), *min_confidence)?,
        Cmd::Map {
            repo,
            family,
            feature,
            limit,
        } => serde_json::to_value(crate::application::map::map(
            &cli.root,
            repo,
            family,
            feature.as_deref(),
            (*limit).clamp(1, 50),
        )?)
        .unwrap(),
        Cmd::Relate {
            repo,
            family,
            stored,
        } => {
            let items = if *stored {
                crate::application::map::latest_relatedness(
                    &cli.root,
                    repo.as_deref(),
                    family.as_deref(),
                )?
            } else {
                crate::application::map::relate(&cli.root, repo.as_deref(), family.as_deref())?
            };
            let total = items.len();
            serde_json::json!({ "items": items, "page": { "next": null, "total": total } })
        }
        Cmd::AbsentCheck {
            repo,
            family,
            feature,
        } => serde_json::to_value(crate::application::absent::absent_check(
            &cli.root, repo, family, feature,
        )?)
        .unwrap(),
        Cmd::Claim {
            repo,
            feature,
            verdict,
            author,
            family,
            at,
            searched,
            note,
            batch: _,
        } => serde_json::to_value(crate::application::claim::claim(
            &cli.root,
            &crate::application::claim::ClaimInput {
                repo: repo.as_deref().unwrap_or_default(),
                feature: feature.as_deref().unwrap_or_default(),
                family: family.as_deref(),
                verdict: verdict.as_deref().unwrap_or_default(),
                author: author.as_deref().unwrap_or_default(),
                at,
                searched: searched.as_deref(),
                note: note.as_deref(),
                skip_equal: false,
            },
        )?)
        .unwrap(),
        Cmd::Claims {
            repo,
            family,
            feature,
            history,
        } => {
            let items = crate::application::claim::claims(
                &cli.root,
                repo.as_deref(),
                family.as_deref(),
                feature.as_deref(),
                *history,
            )?;
            let total = items.len();
            serde_json::json!({ "items": items, "page": { "next": null, "total": total } })
        }
        Cmd::Decide {
            repo,
            disposition,
            why,
            by,
            feature,
            family,
            batch: _,
        } => serde_json::to_value(crate::application::decide::decide(
            &cli.root,
            repo.as_deref().unwrap_or_default(),
            disposition.as_deref().unwrap_or_default(),
            why.as_deref().unwrap_or_default(),
            by.as_deref().unwrap_or_default(),
            feature.as_deref(),
            family.as_deref(),
        )?)
        .unwrap(),
        Cmd::Decisions { repo, history } => {
            let items =
                crate::application::decide::decisions(&cli.root, repo.as_deref(), *history)?;
            let total = items.len();
            serde_json::json!({ "items": items, "page": { "next": null, "total": total } })
        }
        Cmd::Matrix { family, relate } => {
            let m = crate::application::matrix::matrix_with(&cli.root, family, *relate)?;
            let mut v = serde_json::to_value(&m).unwrap();
            if m.not_run > 0 {
                v["limitation_items"] = serde_json::json!([{
                    "code": "relate_not_run",
                    "detail": format!(
                        "relate has not been run for {} of {} comparators; run `comparegraph relate` (or `matrix --relate`)",
                        m.not_run, m.comparators
                    ),
                }]);
            }
            v
        }
        Cmd::Family { what } => {
            use crate::application::family as fam;
            match what {
                FamilyCmd::Add { dir } => {
                    serde_json::to_value(fam::add_family(&cli.root, dir)?).unwrap()
                }
                FamilyCmd::List => {
                    let items = fam::list_families(&cli.root)?;
                    let total = items.len();
                    serde_json::json!({ "items": items, "page": { "next": null, "total": total } })
                }
                FamilyCmd::Propose {
                    name,
                    repos,
                    out,
                    min_repos,
                    max,
                } => serde_json::to_value(crate::application::propose::propose(
                    &cli.root,
                    name,
                    repos,
                    out.as_deref(),
                    *min_repos,
                    *max,
                )?)
                .unwrap(),
                FamilyCmd::Show { name } => {
                    serde_json::to_value(fam::show_family(&cli.root, name)?).unwrap()
                }
            }
        }
        Cmd::Doctor { bin, .. } => {
            let providers: Vec<_> = ["reference"].iter().map(|n| { let p = by_name(n).unwrap(); serde_json::json!({ "name": n, "status": p.detect(), "capabilities": p.capabilities() }) }).collect();
            let external = crate::providers::external::ExternalProvider::list(&cli.root);
            let ctx = setup::Ctx::from_env(&cli.root, bin.as_deref());
            let mut limitations = Vec::new();
            if !ctx.explicit_bin && ctx.dev_build {
                limitations.push(serde_json::json!({ "code": "dev_build", "detail": "compared against a binary in a build directory; pass --bin <installed path>" }));
            }
            let clients: Vec<_> = setup::registrations(&ctx).iter().chain(setup::extra_registrations(&ctx).iter()).map(|r| r.to_json(false)).collect();
            if !cli.root.join(".comparegraph").join("comparegraph.db").is_file() {
                return Ok(Ok(
                    serde_json::json!({ "store": null, "schema_version": null, "repos": 0, "providers": providers, "external_providers": external, "clients": clients, "limitation_items": limitations }),
                ));
            }
            let store = Store::open_existing(&cli.root)?;
            serde_json::json!({ "store": store.path(), "schema_version": store.schema_version()?, "repos": store.repos()?.len(), "providers": providers, "external_providers": external, "clients": clients, "limitation_items": limitations })
        }
        Cmd::Setup { client, scope, apply, force, bin, no_rules, no_hook } => {
            let scope = match scope.as_str() {
                "user" => setup::Scope::User,
                "project" => setup::Scope::Project,
                o => return Err(AppError::InvalidArguments(format!("unknown scope {o} (user|project)"))),
            };
            let (clients, detect) = if client == "all" {
                (setup::ALL.to_vec(), true)
            } else {
                match setup::Client::parse(client) {
                    Some(c) => (vec![c], false),
                    None => {
                        let names: Vec<_> = setup::ALL.iter().map(|c| c.name()).collect();
                        return Err(AppError::InvalidArguments(format!("unknown client {client} ({}|all)", names.join("|"))));
                    }
                }
            };
            let ctx = setup::Ctx::from_env(&cli.root, bin.as_deref());
            if !ctx.explicit_bin && ctx.dev_build {
                return Err(AppError::InvalidArguments("running from a build directory; pass --bin <installed path>".into()));
            }
            if ctx.home.is_none() {
                return Err(AppError::InvalidArguments("HOME is not set".into()));
            }
            let rows = setup::run(&ctx, &setup::Options { clients, detect, scope, apply: *apply, force: *force, no_rules: *no_rules, no_hook: *no_hook })?;
            let limitations: Vec<_> = rows
                .iter()
                .filter(|r| matches!(r.state, setup::State::Conflict | setup::State::Stale) && !r.written)
                .map(|r| serde_json::json!({ "code": r.state.name(), "detail": format!("{} {} ({}): {}", r.client, r.kind, r.path.display(), r.note.as_deref().unwrap_or("")) }))
                .collect();
            serde_json::json!({
                "apply": apply, "force": force, "scope": scope.name(), "bin": ctx.bin, "workspace": ctx.workspace,
                "dry_run": !apply,
                "clients": rows.iter().map(|r| r.to_json(true)).collect::<Vec<_>>(),
                "limitation_items": limitations,
            })
        }
        Cmd::Catalog { check, file } => {
            let live = crate::application::catalog::render(crate::adapters::mcp::tools());
            if !*check {
                return Ok(Ok(serde_json::json!({ "catalog": serde_json::from_str::<serde_json::Value>(&live).unwrap() })));
            }
            let p = file.clone().unwrap_or_else(|| PathBuf::from("docs/catalog.json"));
            let disk = std::fs::read_to_string(&p).map_err(|e| AppError::Io(p.clone(), e))?;
            if disk != live {
                return Ok(Err((
                    AppError::CheckFailed(format!("{} differs from the live catalog", p.display())),
                    Some(serde_json::json!({ "file": p, "stale": true })),
                )));
            }
            serde_json::json!({ "file": p, "stale": false })
        }
        Cmd::Mcp => unreachable!("mcp is served by run()"),
        Cmd::Repos { limit, cursor } => {
            let offset = match cursor {
                Some(c) => c
                    .parse::<usize>()
                    .map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
                None => 0,
            };
            let limit = (*limit).clamp(1, 500);
            let store = Store::open_existing(&cli.root)?;
            let mut out = Vec::new();
            let all = store.repos()?;
            let total = all.len();
            for r in all.into_iter().skip(offset).take(limit) {
                let run = match r.id {
                    Some(id) => store.latest_run(id)?,
                    None => None,
                };
                let latest = match run {
                    Some(run) => {
                        let rid = run.id.expect("stored run has id");
                        serde_json::json!({
                            "status": run.status,
                            "nodes": store.nodes_of_run(rid)?.len(),
                            "communities": store.communities_of_run(rid)?.len(),
                            "store_hash": run.store_hash,
                        })
                    }
                    None => serde_json::Value::Null,
                };
                out.push(serde_json::json!({
                    "name": r.name, "licence": r.licence, "lane": r.lane, "role": r.role,
                    "head": r.head, "latest_run": latest,
                }));
            }
            let next = (offset + out.len() < total).then(|| (offset + out.len()).to_string());
            serde_json::json!({ "items": out, "page": { "next": next, "total": total } })
        }
        Cmd::Communities {
            repo,
            limit,
            cursor,
        } => {
            let offset = match cursor {
                Some(c) => c
                    .parse::<usize>()
                    .map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
                None => 0,
            };
            let limit = (*limit).clamp(1, 500);
            let store = Store::open_existing(&cli.root)?;
            let r = store
                .repo_by_name(repo)?
                .ok_or_else(|| AppError::NotFound(format!("no repo {repo}")))?;
            let run = store
                .latest_done_run(r.id.expect("stored repo has id"))?
                .ok_or_else(|| AppError::Empty(format!("{repo} has no finished run")))?;
            let mut rows = store.communities_of_run(run.id.expect("stored run has id"))?;
            rows.sort_by(|a, b| {
                b.members
                    .len()
                    .cmp(&a.members.len())
                    .then(a.label.cmp(&b.label))
            });
            let total = rows.len();
            let items: Vec<_> = rows
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|c| serde_json::json!({ "label": c.label, "keywords": c.keywords, "cohesion": c.cohesion, "size": c.members.len() }))
                .collect();
            let next = if offset + items.len() < total {
                Some((offset + items.len()).to_string())
            } else {
                None
            };
            serde_json::json!({ "items": items, "page": { "next": next, "total": total } })
        }
        Cmd::Conform { provider, fixtures } => {
            let rep = crate::application::conform::conform_in(&cli.root, provider, fixtures)?;
            let v = serde_json::to_value(&rep).unwrap();
            if crate::providers::external::ExternalProvider::discover(&cli.root, provider).is_some() && provider != "reference" {
                let f = crate::providers::external::conform_path(&cli.root, provider);
                if let Some(d) = f.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap());
            }
            if !rep.passed {
                let msg = rep
                    .fixtures
                    .iter()
                    .filter(|f| !f.passed)
                    .map(|f| {
                        format!(
                            "{}: nodes {:.2}/{:.2} edges {:.2}/{:.2}",
                            f.name,
                            f.nodes.precision,
                            f.nodes.recall,
                            f.edges.precision,
                            f.edges.recall
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                return Ok(Err((AppError::BelowFloor(msg), Some(v))));
            }
            v
        }
    }))
}
