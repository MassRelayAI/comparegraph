#![allow(clippy::result_large_err)] // serde_json::Value is larger with arbitrary_precision
use crate::adapters::envelope::Envelope;
use crate::application::graph as g;
use crate::application::{
    build::build_repo,
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
    /// re-root the clone paths a copied store holds, as `FROM=TO` (repeatable): `--root-map /data/repos=D:/repos`
    #[arg(long, global = true, value_name = "FROM=TO")]
    pub root_map: Vec<String>,
    /// seconds a writer waits for another writer's store lock before failing (default 30, at most 3600; env COMPAREGRAPH_WAIT)
    #[arg(long, global = true, value_name = "SECS")]
    pub wait: Option<u64>,
    #[command(subcommand)]
    pub cmd: Cmd,
}
#[derive(Subcommand)]
pub enum UiCmd {
    /// Products with evidence, and what each has
    Products,
    /// The screens of a product with archetype and captures
    Screens { product: String },
    /// One capture: metrics with pass/fail, layout, style, image path
    Show {
        product: String,
        screen: String,
        #[arg(long, default_value = "populated")]
        state: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
    },
    /// Two products on the same screen, state and viewport: metrics, layout and style in words, both image paths
    Diff {
        /// a screen id or a map.csv row id
        target: String,
        #[arg(long)]
        ours: String,
        #[arg(long = "ref")]
        refp: String,
        #[arg(long, default_value = "populated")]
        state: String,
        #[arg(long)]
        viewport: Option<String>,
        /// the reference's device; a different one than --viewport is refused (cross-device)
        #[arg(long)]
        ref_viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
        #[arg(long)]
        ref_seat: Option<String>,
    },
    /// Rows of a family by products, each cell pass count / 12 or `-`, via map.csv
    Matrix {
        family: String,
        #[arg(long, default_value = "populated")]
        state: String,
        #[arg(long)]
        viewport: Option<String>,
        /// one column per device (`desktop-1440,iphone` or `all`): each cell is `pass/12 <support class>`, `-` or `n/a (absent)`; plus support-class counts per product and device
        #[arg(long)]
        devices: Option<String>,
        /// also list the captures with failed network calls (4xx/5xx/network error), from `net.failed`
        #[arg(long)]
        net: bool,
        /// the map.csv to read (default: ui/map.csv, else <ui-root>/map.csv where the sweep keeps it)
        #[arg(long)]
        map: Option<PathBuf>,
    },
    /// A product's design system (system.json, else derived) against the targets
    System { product: String },
    /// One flow (ui-flow/1) of a product
    Flow { product: String, task: String },
    /// The menu tree of a product and its derived numbers
    Nav {
        product: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
    },
    /// Two menu trees compared: places, depth, items per level, duplicates, dead items, overflow-only items
    NavDiff {
        #[arg(long)]
        ours: String,
        #[arg(long = "ref")]
        refp: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
        #[arg(long)]
        ref_seat: Option<String>,
    },
    /// Two products' network behaviour on the same screen, state and device: requests added, removed, changed by normalised method + path + response shape
    NetDiff {
        #[arg(long)]
        ours: String,
        #[arg(long = "ref")]
        refp: String,
        #[arg(long)]
        screen: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        device: Option<String>,
    },
    /// The interaction graph of a product (ui-graph/1): reachable screens, depth, orphans, dead actions, cycles, actions per screen
    Graph {
        product: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
    },
    /// The shortest click path between two screens of a product's graph (bounded by --max-len)
    Path {
        product: String,
        from: String,
        to: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
        /// most clicks looked at, 1..=20
        #[arg(long, default_value_t = 8)]
        max_len: usize,
    },
    /// Two interaction graphs compared: depth to each comparable screen, orphans, dead actions; `--ref target` builds the intended graph from target-nav.json
    GraphDiff {
        #[arg(long)]
        ours: String,
        #[arg(long = "ref")]
        refp: String,
        #[arg(long)]
        viewport: Option<String>,
        #[arg(long)]
        seat: Option<String>,
        #[arg(long)]
        ref_seat: Option<String>,
        /// which tree of target-nav.json (app | router | console); default by product
        #[arg(long)]
        tree: Option<String>,
    },
    /// A journey per product: completed / partial at step N / n/a, and its numbers
    Journey {
        journey: String,
        /// comma list of products
        #[arg(long)]
        products: Option<String>,
    },
    /// The coverage table of a run; a blank cell makes it incomplete
    Coverage { run: String },
    /// The determinism gate: two runs must agree on domHash, metrics, image dHash (<= 4 bits) and failed network calls for every shared key.
    /// Give each run its own ui root and compare with --root-a/--root-b; the two-run-id form only works when the runs wrote different evidence files.
    Verify {
        run_a: Option<String>,
        run_b: Option<String>,
        /// ui root holding one run (instead of run ids)
        #[arg(long, requires = "root_b", conflicts_with_all = ["run_a", "run_b"])]
        root_a: Option<PathBuf>,
        /// ui root holding one run (instead of run ids)
        #[arg(long, requires = "root_a")]
        root_b: Option<PathBuf>,
        /// also re-hash every evidence file and image blob of both runs against the digests the probe recorded (integrity_mismatch when one changed)
        #[arg(long)]
        integrity: bool,
    },
    /// Every cap the UI verbs apply (max-nodes, file size, page size, path length ...) with its unit and what is reported when it is hit
    Limits,
}
#[derive(Subcommand)]
pub enum Cmd {
    /// Register a clone (read, never run); with --name and no path, amend a registered repo's notes
    Add {
        path: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        /// register this clone as the workspace's root (spec §2b product/baseline modes)
        #[arg(long)]
        as_root: bool,
        /// re-point the registered repo `--name` at this path (claims and runs stay)
        #[arg(long, requires = "name")]
        replace: bool,
        /// record this SPDX licence instead of reading the clone's LICENSE file (a partial export has none)
        #[arg(long, value_name = "SPDX", conflicts_with = "licence_from")]
        licence: Option<String>,
        /// record this lane (take or study) instead of the one the licence gives
        #[arg(long)]
        lane: Option<String>,
        /// read the licence from another clone (the full source of a partial export)
        #[arg(long, value_name = "SOURCE_CLONE")]
        licence_from: Option<PathBuf>,
        /// the upstream commit this clone was exported from (a fresh `git init` has no head); '' clears it
        #[arg(long, value_name = "SHA")]
        source_commit: Option<String>,
        /// where this project's core now lives (a registered repo name or a URL); '' clears it
        #[arg(long, value_name = "REPO_OR_URL")]
        moved_to: Option<String>,
        /// default subset for `build`: read only files matching these gitignore-style globs (repeat; '' clears)
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,
        /// default subset for `build`: never read files matching these globs (repeat; '' clears); wins over --include
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
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
        /// read only files matching these gitignore-style globs (repeat); with --exclude, replaces the repo's `add` subset
        #[arg(long, value_name = "GLOB")]
        include: Vec<String>,
        /// never read files matching these globs (repeat); wins over --include; recorded on the run
        #[arg(long, value_name = "GLOB")]
        exclude: Vec<String>,
    },
    /// Verify: schema, search index, citation locations, the action log. Writes nothing unless --mark
    Check {
        /// write `stale = 1` on the locations found stale (CLI only; logged)
        #[arg(long)]
        mark: bool,
        /// with --mark: also clear `stale` on locations whose cited lines hash as recorded again
        #[arg(long, requires = "mark")]
        recompute: bool,
        /// also recompute every latest run's store_hash (reads every node and edge)
        #[arg(long)]
        deep: bool,
        /// who asked (required with --mark; recorded in the action log)
        #[arg(long)]
        by: Option<String>,
    },
    /// Copy the store out: a consistent full copy (VACUUM INTO) or the non-rebuildable rows only; verified, with a manifest
    Backup {
        /// the folder the backup goes into (default .comparegraph/backups)
        #[arg(long)]
        out: Option<PathBuf>,
        /// only the rows that cannot be rebuilt (repos, families, features, runs without graph, claims, locations,
        /// decisions, relatedness, meta) and the action log, as one JSONL
        #[arg(long)]
        claims_only: bool,
        /// then remove the oldest backups of the same kind in --out beyond this many
        #[arg(long)]
        keep: Option<usize>,
        /// who takes it (recorded in the action log)
        #[arg(long)]
        by: String,
    },
    /// Replace the store with a backup: a plan by default; --apply verifies it, refuses while the store is in use,
    /// takes a pre-restore backup and moves the current store aside (never deleted)
    RestoreStore {
        backup: PathBuf,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        by: Option<String>,
    },
    /// Store retention: prune the parse cache (--parse-cache) or mark stuck runs abandoned (--stale-runs); dry-run by default
    Gc {
        /// drop parse-cache rows of other provider versions, then VACUUM
        #[arg(long, required_unless_present = "stale_runs", conflicts_with = "stale_runs")]
        parse_cache: bool,
        /// runs still `running` past --older-than-hours become `abandoned`
        #[arg(long)]
        stale_runs: bool,
        /// keep this many provider versions (the running binary's first); default 1, or the policy's retention
        #[arg(long, requires = "parse_cache")]
        keep_versions: Option<usize>,
        /// drop every cache row (the next build of each clone parses every file)
        #[arg(long, requires = "parse_cache", conflicts_with = "keep_versions")]
        all_cache: bool,
        /// with --stale-runs: the age past which a `running` run is abandoned
        #[arg(long, default_value_t = 24)]
        older_than_hours: u64,
        /// a relevance-policy/1 file whose retention.parse_cache_versions is the default keep
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        /// who runs it (required with --apply; recorded in the action log)
        #[arg(long)]
        by: Option<String>,
    },
    /// Per repo: the factor counts and, with a relevance-policy/1 file, a score and a class; reads only
    Relevance {
        /// the policy (default .comparegraph/policy.json when present; none: every repo `unrated`)
        #[arg(long)]
        policy: Option<PathBuf>,
        /// restrict every factor to one family
        #[arg(long)]
        family: Option<String>,
        /// one repo with every term of its score
        #[arg(long)]
        explain: Option<String>,
        /// only this class (ROOT, KEEP-PRIMARY, KEEP, KEEP-NEW, ARCHIVE, REMOVE-CANDIDATE, unrated)
        #[arg(long)]
        class: Option<String>,
        /// the day ages are counted to (default: the newest timestamp in the store, not the clock)
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<String>,
        /// page size (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Soft-archive a repo: a manifest first, then a flag; its rows stay, it leaves the default views and `check`
    Archive {
        repo: String,
        #[arg(long)]
        why: String,
        #[arg(long)]
        by: String,
        #[arg(long)]
        policy: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        /// archive a primary, a pinned, an adopted or a port-source repo anyway; the reason is recorded
        #[arg(long, value_name = "REASON")]
        force_with_reason: Option<String>,
    },
    /// Bring an archived repo back (by its manifest or its name)
    Restore {
        target: String,
        #[arg(long)]
        by: String,
        /// the clone's folder now (default: the path at archive time)
        #[arg(long)]
        path: Option<PathBuf>,
        /// the clone is at another head: build it as it is now
        #[arg(long)]
        rebuild: bool,
        #[arg(long)]
        apply: bool,
    },
    /// The action log: `governance verify` re-walks its hash chain against the store
    Governance {
        #[command(subcommand)]
        what: GovernanceCmd,
    },
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
    /// Registered repos with their latest run summary (archived repos only with --archived or --all)
    Repos {
        /// page size (1..=500)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        /// only the archived repos
        #[arg(long, conflicts_with = "all")]
        archived: bool,
        /// active and archived repos
        #[arg(long)]
        all: bool,
        /// only the repos whose code must not be ported (lane not take, a never-copy licence, or no licence read), with why
        #[arg(long)]
        no_port: bool,
        /// a relevance-policy/1 file whose licences.never_copy_extra extends the never-copy set
        #[arg(long)]
        policy: Option<PathBuf>,
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
    /// Is a feature absent from a clone's graph? Four channels (symbols, paths, communities, kinds) plus the ranked
    /// `read_set` a `none` must read; reads, writes nothing
    AbsentCheck {
        repo: String,
        family: String,
        feature: String,
        /// also search these terms (the ones a `claim none --searched` would carry); they join the read set
        #[arg(long)]
        searched: Option<String>,
        /// also rank css/scss/html parser nodes in the read set (left out by default; a `none` is judged without them)
        #[arg(long)]
        include_non_code: bool,
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
        /// with --searched on a none: the qualified name of a candidate a reader opened and rejected; repeat the flag,
        /// one name each (a name may hold a comma, a space or a newline). A value that is no candidate's name is split
        /// on commas and spaces (the older `a,b` form). A none whose top candidates are all named is recorded `observed`
        #[arg(long = "read-hits")]
        read_hits: Vec<String>,
        #[arg(long)]
        note: Option<String>,
        /// the upstream commit the citations were read at (7-40 hex); must agree with the head the build read
        #[arg(long = "at-commit", value_name = "SHA")]
        at_commit: Option<String>,
        /// 'path::regex': cite the one code line of path the regex matches (stored as path:N); zero or several refuse
        #[arg(long = "anchor", value_name = "PATH::REGEX")]
        anchor: Vec<String>,
        /// a symbol or regex the cited lines must match: one for every --at, or one per --at in order
        #[arg(long = "expect", value_name = "SYMBOL|REGEX")]
        expect: Vec<String>,
        /// accept a citation that is only blank, comment or docstring lines (the comment is the evidence)
        #[arg(long = "allow-comment")]
        allow_comment: bool,
        /// has/partial evidence present upstream at this path[:lines] but absent from the exported tree
        #[arg(long = "out-of-subset", value_name = "PATH[:LINES]")]
        out_of_subset: Option<String>,
        /// skip the reach walk: the has/partial claim records no reach
        #[arg(long = "no-reach")]
        no_reach: bool,
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
        /// only the rows with a cell whose latest claim has this reach: reached | unreached | unknown
        #[arg(long)]
        reach: Option<String>,
    },
    /// Read and compare UI evidence files (screens, menus, journeys, runs): `ui products|screens|show|diff|matrix|system|flow|nav|nav-diff|net-diff|journey|coverage|verify`
    Ui {
        /// the directory holding `ui/` (default: --root)
        #[arg(long, global = true)]
        ui_root: Option<PathBuf>,
        /// files read at most (default 200, max 2000); a truncated answer says so in evidence.limitations
        #[arg(long, global = true, default_value_t = 200)]
        max_nodes: usize,
        /// compare runs/captures made from different inputs (probe version, registry, archetypes, fixture, env); the drift is then stated in limitations
        #[arg(long, global = true)]
        allow_input_drift: bool,
        /// the cursor of the next page (the `page.next` of a previous answer) for the listing verbs
        #[arg(long, global = true)]
        cursor: Option<String>,
        /// items per page of a listing (default 50, max 500)
        #[arg(long, global = true)]
        page_size: Option<usize>,
        /// the locale of the capture to select (default en-US)
        #[arg(long, global = true)]
        locale: Option<String>,
        /// the theme of the capture to select (default light)
        #[arg(long, global = true)]
        theme: Option<String>,
        #[command(subcommand)]
        what: UiCmd,
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
pub enum GovernanceCmd {
    /// Re-walk the action log: every hash, a gapless seq, the head in the store, archive flags against their manifests
    Verify,
}
#[derive(Subcommand)]
pub enum FamilyCmd {
    /// Read a pack directory (rows.md) into the store; features it no longer lists become retired
    Add {
        dir: PathBuf,
        /// add a pack whose front matter says `draft: true` (it would retire every row it does not list)
        #[arg(long)]
        accept_draft: bool,
    },
    /// Registered families
    List,
    /// One family with its features by group
    Show { name: String },
    /// Write a draft taxonomy (rows.draft.md) from the terms the given repos share; or, with --from-claims, draft the
    /// feature ids a standard repo's scored rows name and the family lacks (rows.from-claims.draft.md); no model
    Propose {
        name: String,
        #[arg(long, value_delimiter = ',', required_unless_present = "from_claims", conflicts_with = "from_claims")]
        repos: Vec<String>,
        /// the standard repo whose scored rows (--rows) are checked against family NAME
        #[arg(long, requires = "rows")]
        from_claims: Option<String>,
        /// JSON lines in the `claim --batch` shape (`feature` required; `repo`, `title`, `group`, `verdict`, `note`,
        /// `at` used), `-` for stdin
        #[arg(long, requires = "from_claims")]
        rows: Option<String>,
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
        Cmd::Check { .. } => "check",
        Cmd::Backup { .. } => "backup",
        Cmd::RestoreStore { .. } => "restore-store",
        Cmd::Gc { .. } => "gc",
        Cmd::Relevance { .. } => "relevance",
        Cmd::Archive { .. } => "archive",
        Cmd::Restore { .. } => "restore",
        Cmd::Governance { .. } => "governance-verify",
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
        Cmd::Ui { what, .. } => match what {
            UiCmd::Products => "ui-products",
            UiCmd::Screens { .. } => "ui-screens",
            UiCmd::Show { .. } => "ui-show",
            UiCmd::Diff { .. } => "ui-diff",
            UiCmd::Matrix { .. } => "ui-matrix",
            UiCmd::System { .. } => "ui-system",
            UiCmd::Flow { .. } => "ui-flow",
            UiCmd::Nav { .. } => "ui-nav",
            UiCmd::NavDiff { .. } => "ui-nav-diff",
            UiCmd::NetDiff { .. } => "ui-net-diff",
            UiCmd::Graph { .. } => "ui-graph",
            UiCmd::Path { .. } => "ui-path",
            UiCmd::GraphDiff { .. } => "ui-graph-diff",
            UiCmd::Journey { .. } => "ui-journey",
            UiCmd::Coverage { .. } => "ui-coverage",
            UiCmd::Verify { .. } => "ui-verify",
            UiCmd::Limits => "ui-limits",
        },
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    if cli.allow_unverified {
        std::env::set_var("COMPAREGRAPH_ALLOW_UNVERIFIED", "1");
    }
    if !cli.root_map.is_empty() {
        std::env::set_var("COMPAREGRAPH_ROOT_MAP", cli.root_map.join(";"));
    }
    // `--wait` past the maximum is refused; a malformed COMPAREGRAPH_WAIT is refused, a huge one is clamped with a note
    let max = crate::store::MAX_WAIT_SECS;
    let wait_err = match cli.wait {
        Some(w) if w > max => Some(format!("--wait {w} is above the maximum of {max} s")),
        Some(w) => {
            std::env::set_var("COMPAREGRAPH_WAIT", w.to_string());
            None
        }
        None => match crate::store::wait_env() {
            Err(e) => Some(e),
            Ok(Some(w)) if w > max => {
                eprintln!("comparegraph: COMPAREGRAPH_WAIT={w} is above the maximum; waiting {max} s");
                None
            }
            Ok(_) => None,
        },
    };
    if let Some(msg) = wait_err {
        let e = AppError::InvalidArguments(msg);
        eprintln!("comparegraph: {e}");
        print_line(&serde_json::from_str(&Envelope::err(verb_of(&cli.cmd), &e).to_json()).unwrap(), cli.pretty);
        return 1;
    }
    // the store's holder note names this verb when another writer times out on the lock
    std::env::set_var("COMPAREGRAPH_VERB", verb_of(&cli.cmd));
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
        Cmd::Claim { repo, feature, verdict, author, family, at, searched, read_hits, note, at_commit, anchor, expect, allow_comment, out_of_subset, no_reach, .. } => {
            !(repo.is_none() && feature.is_none() && verdict.is_none() && author.is_none()
                && family.is_none() && at.is_empty() && searched.is_none() && read_hits.is_empty() && note.is_none())
                || at_commit.is_some() || !anchor.is_empty() || !expect.is_empty() || *allow_comment || out_of_subset.is_some() || *no_reach
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
    let print_row = |line: usize, res: Result<serde_json::Value, BatchErr>, conf: &'static str| {
        let env = match res {
            Ok(mut v) => {
                // as `envelope_for` does for a single claim: the row's qualifiers go to evidence.limitations
                let lims: Vec<crate::domain::Limitation> = v
                    .as_object_mut()
                    .and_then(|o| o.remove("limitation_items"))
                    .and_then(|x| serde_json::from_value(x).ok())
                    .unwrap_or_default();
                Envelope::ok(verb, v, conf, lims, None)
            }
            Err((code, msg, data)) => {
                if msg.starts_with("line ") {
                    eprintln!("comparegraph: {msg}");
                } else {
                    eprintln!("comparegraph: line {line}: {msg}");
                }
                let mut e = Envelope::err(verb, &AppError::InvalidArguments(msg));
                if let Some(b) = e.error.as_mut() {
                    b.code = code;
                    b.data = data;
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
                Some("observed") => "observed",
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

/// A failed batch row: code, message, structured detail (`error.data`).
type BatchErr = (&'static str, String, Option<serde_json::Value>);

fn batch_value<T: serde::Serialize>(
    data: Option<T>,
    error: Option<crate::application::claim::BatchError>,
) -> Result<serde_json::Value, BatchErr> {
    match (data, error) {
        (Some(d), _) => Ok(serde_json::to_value(d).unwrap()),
        (None, Some(e)) => Err((e.code, e.message, e.data)),
        (None, None) => Err(("internal", "row has neither data nor error".into(), None)),
    }
}

/// Run one verb and wrap the outcome in the envelope; shared by the CLI and the MCP adapter.
pub fn envelope_for(cli: &Cli) -> Envelope {
    let verb = verb_of(&cli.cmd);
    match dispatch(cli) {
        Ok(v) => {
            let conf = if matches!(
                cli.cmd,
                Cmd::Check { .. } | Cmd::Claims { .. } | Cmd::Matrix { .. } | Cmd::Governance { .. }
                    | Cmd::Ui { what: UiCmd::Diff { .. } | UiCmd::Matrix { .. } | UiCmd::NavDiff { .. } | UiCmd::NetDiff { .. } | UiCmd::GraphDiff { .. } | UiCmd::Verify { .. }, .. }
            ) {
                "derived"
            } else if matches!(cli.cmd, Cmd::Claim { .. }) {
                match v["confidence"].as_str() {
                    Some("observed") => "observed",
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
                    b.recovery = Some(crate::application::catalog::recovery("catalog_stale"));
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
            licence,
            lane,
            licence_from,
            source_commit,
            moved_to,
            include,
            exclude,
        } => {
            use crate::application::add::{add_repo_with, amend_repo, replace_repo_with, LicenceOverride};
            let over = LicenceOverride {
                licence: licence.clone(),
                lane: lane.clone(),
                licence_from: licence_from.clone(),
                source_commit: source_commit.clone(),
                moved_to: moved_to.clone(),
                include: include.clone(),
                exclude: exclude.clone(),
            };
            let Some(path) = path else {
                let Some(name) = name else {
                    return Err(AppError::InvalidArguments("add needs a path, or --name of a registered repo to amend".into()));
                };
                if *replace || *as_root {
                    return Err(AppError::InvalidArguments("--replace and --as-root need a path".into()));
                }
                return Ok(Ok(serde_json::to_value(amend_repo(&cli.root, name, &over)?).unwrap()));
            };
            if *replace {
                serde_json::to_value(replace_repo_with(&cli.root, path, name.as_deref().unwrap_or_default(), *as_root, &over)?).unwrap()
            } else {
                serde_json::to_value(add_repo_with(&cli.root, path, name.as_deref(), *as_root, &over)?).unwrap()
            }
        }
        Cmd::Build {
            repo,
            provider,
            providers,
            max_file_bytes,
            max_files,
            full,
            progress,
            include,
            exclude,
        } if !providers.is_empty() => {
            let rep = crate::application::build::build_many(
                &cli.root,
                repo,
                providers,
                &BuildOptions {
                    max_file_bytes: *max_file_bytes,
                    max_files: *max_files,
                    full: *full,
                    progress: progress_on(*progress),
                    include: include.clone(),
                    exclude: exclude.clone(),
                    ..BuildOptions::default()
                },
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
            include,
            exclude,
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
                include: include.clone(),
                exclude: exclude.clone(),
                ..BuildOptions::default()
            },
        )?)
        .unwrap(),
        Cmd::Check { mark, recompute, deep, by } => {
            let r = crate::application::check::check_with(
                &cli.root,
                &crate::application::check::CheckOptions { mark: *mark, recompute: *recompute, deep: *deep, by: by.clone() },
            )?;
            let v = serde_json::to_value(&r).unwrap();
            if !r.ok() {
                let e = if r.only_unavailable() {
                    AppError::structured(
                        "clone_unavailable",
                        format!("{} clone(s) could not be read as their runs read them; nothing was written for them", r.clone_unavailable.len()),
                        Some("put the clone back at the head its run read (data.clone_unavailable), or `archive` the repo".into()),
                        None,
                    )
                } else {
                    AppError::CheckFailed(format!("{} problem(s)", r.problems.len()))
                };
                return Ok(Err((e, Some(v))));
            }
            v
        }
        Cmd::Backup { out, claims_only, keep, by } => {
            let kind = if *claims_only { crate::application::backup::Kind::ClaimsOnly } else { crate::application::backup::Kind::Full };
            crate::application::backup::backup(&cli.root, out.as_deref(), kind, *keep, by)?
        }
        Cmd::RestoreStore { backup, apply, by } => {
            crate::application::backup::restore_store(&cli.root, backup, *apply, by.as_deref().unwrap_or(""))?
        }
        Cmd::Gc { parse_cache, stale_runs: _, keep_versions, all_cache, older_than_hours, policy, apply, by } => {
            let by = by.as_deref().unwrap_or("");
            if *parse_cache {
                let policy_keep = crate::application::relevance::load_policy(&cli.root, policy.as_deref())?
                    .and_then(|p| p.policy.retention.map(|r| r.parse_cache_versions));
                crate::application::retain::gc_parse_cache(
                    &cli.root,
                    &crate::application::retain::CacheOpts { keep_versions: *keep_versions, all: *all_cache, apply: *apply, by, policy_keep },
                )?
            } else {
                crate::application::retain::gc_stale_runs(&cli.root, *older_than_hours, *apply, by)?
            }
        }
        Cmd::Relevance { policy, family, explain, class, as_of, limit, cursor } => crate::application::relevance::relevance(
            &cli.root,
            &crate::application::relevance::Opts {
                policy: policy.as_deref(),
                family: family.as_deref(),
                explain: explain.as_deref(),
                class: class.as_deref(),
                as_of: as_of.as_deref(),
                limit: *limit,
                cursor: cursor.as_deref(),
            },
        )?,
        Cmd::Archive { repo, why, by, policy, apply, force_with_reason } => crate::application::archive::archive(
            &cli.root,
            repo,
            &crate::application::archive::ArchiveOpts { why, by, policy: policy.as_deref(), apply: *apply, force_with_reason: force_with_reason.as_deref() },
        )?,
        Cmd::Restore { target, by, path, rebuild, apply } => crate::application::archive::restore(
            &cli.root,
            target,
            &crate::application::archive::RestoreOpts { by, path: path.as_deref(), rebuild: *rebuild, apply: *apply },
        )?,
        Cmd::Governance { what: GovernanceCmd::Verify } => {
            let r = crate::application::governance::verify(&cli.root)?;
            let v = serde_json::to_value(&r).unwrap();
            if !r.ok() {
                let at = r.broken_at.as_ref().map(|b| format!(" at line {}", b.line)).unwrap_or_default();
                return Ok(Err((AppError::CheckFailed(format!("the action log does not verify{at}: {}", r.problems.join("; "))), Some(v))));
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
            searched,
            include_non_code,
        } => serde_json::to_value(crate::application::absent::absent_check(
            &cli.root, repo, family, feature, searched.as_deref(), *include_non_code,
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
            read_hits,
            note,
            batch: _,
            at_commit,
            anchor,
            expect,
            allow_comment,
            out_of_subset,
            no_reach,
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
                read_hits,
                note: note.as_deref(),
                skip_equal: false,
                evidence: crate::application::claim::Evidence {
                    at_commit: at_commit.as_deref(),
                    anchors: anchor,
                    expect,
                    allow_comment: *allow_comment,
                    out_of_subset: out_of_subset.as_deref(),
                    no_reach: *no_reach,
                },
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
        Cmd::Ui { ui_root, max_nodes, allow_input_drift, cursor, page_size, locale, theme, what } => {
            use crate::application::ui as u;
            let o = u::Opts::from_flags(*allow_input_drift, cursor.as_deref(), *page_size, locale.as_deref(), theme.as_deref())?;
            let o = &o;
            let ur = ui_root.clone().unwrap_or_else(|| cli.root.clone());
            let (ur, m) = (ur.as_path(), *max_nodes);
            fn vp(o: &Option<String>) -> Option<&str> {
                o.as_deref()
            }
            fn st(s: &str) -> &str {
                s
            }
            match what {
                UiCmd::Products => u::products(ur, m, o)?,
                UiCmd::Limits => crate::application::ui_limits::limits(),
                UiCmd::Screens { product } => u::screens(ur, m, product, o)?,
                UiCmd::Show { product, screen, state, viewport, seat } => u::show(ur, m, product, screen, st(state), vp(viewport), vp(seat), o)?,
                UiCmd::Diff { target, ours, refp, state, viewport, ref_viewport, seat, ref_seat } => {
                    u::diff(ur, m, target, ours, refp, st(state), vp(viewport), vp(ref_viewport), vp(seat), vp(ref_seat), o)?
                }
                UiCmd::Matrix { family, state, viewport, devices, net, map } => u::matrix(&cli.root, ur, m, family, st(state), vp(viewport), vp(devices), *net, map.as_deref(), o)?,
                UiCmd::System { product } => u::system(ur, m, product)?,
                UiCmd::Flow { product, task } => u::flow(ur, m, product, task)?,
                UiCmd::Nav { product, viewport, seat } => u::nav(ur, m, product, vp(viewport), vp(seat))?,
                UiCmd::NavDiff { ours, refp, viewport, seat, ref_seat } => u::nav_diff_verb(ur, m, ours, refp, vp(viewport), vp(seat), vp(ref_seat))?,
                UiCmd::NetDiff { ours, refp, screen, state, device } => crate::application::ui_net::net_diff(ur, m, ours, refp, vp(screen), vp(state), vp(device), o)?,
                UiCmd::Graph { product, viewport, seat } => crate::application::ui_graph::graph(ur, m, product, vp(viewport), vp(seat))?,
                UiCmd::Path { product, from, to, viewport, seat, max_len } => crate::application::ui_graph::path(ur, m, product, from, to, vp(viewport), vp(seat), *max_len)?,
                UiCmd::GraphDiff { ours, refp, viewport, seat, ref_seat, tree } => crate::application::ui_graph::graph_diff(ur, m, ours, refp, vp(viewport), vp(seat), vp(ref_seat), vp(tree))?,
                UiCmd::Journey { journey, products } => {
                    let ps: Vec<String> = products.as_deref().map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
                    u::journey(ur, journey, &ps, o)?
                }
                UiCmd::Coverage { run } => u::coverage(ur, run)?,
                UiCmd::Verify { run_a, run_b, root_a, root_b, integrity } => {
                    let r = match (run_a, run_b, root_a, root_b) {
                        (None, None, Some(a), Some(b)) => u::verify_roots(a, b, m, o, *integrity),
                        (Some(a), Some(b), None, None) => u::verify(ur, m, a, b, o, *integrity),
                        _ => return Err(AppError::InvalidArguments("ui verify takes two run ids (RUN_A RUN_B) or --root-a DIR --root-b DIR, not a mix".into())),
                    };
                    match r {
                        Ok(v) => v,
                        Err(f) => return Ok(Err(f)),
                    }
                }
            }
        }
        Cmd::Matrix { family, relate, reach } => {
            let m = crate::application::matrix::matrix_reach(&cli.root, family, *relate, reach.as_deref())?;
            let mut v = serde_json::to_value(&m).unwrap();
            let mut lims = vec![];
            if m.not_run > 0 {
                lims.push(serde_json::json!({
                    "code": "relate_not_run",
                    "detail": format!(
                        "relate has not been run for {} of {} comparators; run `comparegraph relate` (or `matrix --relate`)",
                        m.not_run, m.comparators
                    ),
                }));
            }
            if m.archived_skipped > 0 {
                lims.push(serde_json::json!({"code": "archived_skipped", "detail": format!("{} archived repo(s) are not columns", m.archived_skipped)}));
            }
            if !lims.is_empty() {
                v["limitation_items"] = serde_json::json!(lims);
            }
            v
        }
        Cmd::Family { what } => {
            use crate::application::family as fam;
            match what {
                FamilyCmd::Add { dir, accept_draft } => {
                    serde_json::to_value(fam::add_family_with(&cli.root, dir, *accept_draft)?).unwrap()
                }
                FamilyCmd::List => {
                    let items = fam::list_families(&cli.root)?;
                    let total = items.len();
                    serde_json::json!({ "items": items, "page": { "next": null, "total": total } })
                }
                FamilyCmd::Propose {
                    name,
                    from_claims: Some(repo),
                    rows: Some(rows),
                    out,
                    ..
                } => {
                    let reader: Box<dyn std::io::BufRead> = if rows == "-" {
                        Box::new(std::io::BufReader::new(std::io::stdin().lock()))
                    } else {
                        let f = std::fs::File::open(rows)
                            .map_err(|e| AppError::Io(PathBuf::from(rows), e))?;
                        Box::new(std::io::BufReader::new(f))
                    };
                    serde_json::to_value(crate::application::propose::from_claims(
                        &cli.root,
                        name,
                        repo,
                        reader,
                        rows,
                        out.as_deref(),
                    )?)
                    .unwrap()
                }
                FamilyCmd::Propose {
                    name,
                    repos,
                    out,
                    min_repos,
                    max,
                    ..
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
                    serde_json::json!({ "store": null, "schema_version": null, "repos": 0, "unmapped_roots": [], "providers": providers, "external_providers": external, "clients": clients, "limitation_items": limitations }),
                ));
            }
            let store = Store::open_existing(&cli.root)?;
            let repos = store.repos()?;
            // a registered clone that is not on this machine (a store copied from another one): `--root-map` re-roots it
            let unmapped: Vec<_> = repos.iter().filter(|r| !std::path::Path::new(&r.path).is_dir()).map(|r| serde_json::json!({ "name": r.name, "path": r.path })).collect();
            if !unmapped.is_empty() {
                limitations.push(serde_json::json!({ "code": "unmapped_roots", "detail": format!("{} registered clone path(s) do not exist here; pass --root-map FROM=TO", unmapped.len()) }));
            }
            // the action log and the archive count (governance verify, summarised)
            let governance = match crate::application::governance::verify_with(&store, &cli.root) {
                Ok(g) => serde_json::json!({ "actions": g.actions, "chain_ok": g.ok(), "broken_at": g.broken_at.map(|b| b.line), "archived": g.archived }),
                Err(e) => serde_json::json!({ "error": e.to_string() }),
            };
            serde_json::json!({ "store": store.path(), "schema_version": store.schema_version()?, "repos": repos.len(), "unmapped_roots": unmapped, "providers": providers, "external_providers": external, "clients": clients, "governance": governance, "limitation_items": limitations })
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
        Cmd::Repos { limit, cursor, archived, all, no_port, policy } => {
            let offset = match cursor {
                Some(c) => c
                    .parse::<usize>()
                    .map_err(|_| AppError::InvalidArguments(format!("bad cursor {c}")))?,
                None => 0,
            };
            let limit = (*limit).clamp(1, 500);
            let extra: Vec<String> = match crate::application::relevance::load_policy(&cli.root, policy.as_deref())? {
                Some(p) => p.policy.licences.map(|l| l.never_copy_extra).unwrap_or_default(),
                None => vec![],
            };
            let store = Store::open_existing(&cli.root)?;
            let arch = crate::application::archive::archived_names(&store)?;
            let mut out = Vec::new();
            let every = store.repos()?;
            let skipped = if *archived || *all { 0 } else { every.iter().filter(|r| arch.contains(&r.name)).count() };
            let all_rows: Vec<_> = every
                .into_iter()
                .filter(|r| if *archived { arch.contains(&r.name) } else { *all || !arch.contains(&r.name) })
                .filter(|r| !*no_port || !crate::domain::licence::no_port_reasons(r.licence.as_deref(), &r.lane, &extra).is_empty())
                .collect();
            let total = all_rows.len();
            for r in all_rows.into_iter().skip(offset).take(limit) {
                let run = match r.id {
                    Some(id) => store.latest_run(id)?,
                    None => None,
                };
                let latest = match run {
                    Some(run) => {
                        let rid = run.id.expect("stored run has id");
                        let (inc, exc) = store.run_subset(rid)?;
                        serde_json::json!({
                            "status": run.status, "include": inc, "exclude": exc,
                            "nodes": store.nodes_of_run(rid)?.len(),
                            "communities": store.communities_of_run(rid)?.len(),
                            "store_hash": run.store_hash,
                        })
                    }
                    None => serde_json::Value::Null,
                };
                let mut row = serde_json::json!({
                    "name": r.name, "licence": r.licence, "lane": r.lane, "role": r.role,
                    "head": r.head, "source_commit": r.source_commit, "moved_to": r.moved_to,
                    "include": r.include, "exclude": r.exclude, "latest_run": latest,
                });
                if *archived || *all {
                    row["archived"] = serde_json::json!(arch.contains(&r.name));
                }
                if *no_port {
                    row["no_port"] = serde_json::json!(crate::domain::licence::no_port_reasons(r.licence.as_deref(), &r.lane, &extra));
                }
                out.push(row);
            }
            let next = (offset + out.len() < total).then(|| (offset + out.len()).to_string());
            let mut lims = vec![];
            if skipped > 0 {
                lims.push(serde_json::json!({"code": "archived_skipped", "detail": format!("{skipped} archived repo(s) not listed; --archived or --all lists them")}));
            }
            serde_json::json!({ "items": out, "page": { "next": next, "total": total }, "limitation_items": lims })
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
