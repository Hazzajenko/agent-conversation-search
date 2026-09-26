use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand, ValueEnum};

use agsearch::{
    export_timestamp_now, failed_in_store_session, failed_in_stores, format_current,
    format_export_markdown, format_failures, format_paths, format_projects, format_results,
    format_session_paths, format_sessions, format_stats, format_touch_paths, format_touches,
    format_transcript_for_harness, format_usage_breakdown, format_usage_ranking,
    format_windowed_for_harness, group_failures, list_store_projects, list_store_sessions,
    parse_store_transcript, parse_transcript_locator, resolve_claude_dir, resolve_codex_dir,
    resolve_current_context, resolve_current_session, resolve_current_thread,
    resolve_store_session_prefix, resolve_store_session_prefix_including_subagents,
    search_store_session, search_store_session_with_file, search_stores, search_stores_with_file,
    since_cutoff, timestamp_is_since, touches_in_store_session, touches_in_stores,
    usage_calls_for_session, usage_ranking, ContentSet, FileSelector, Harness, Matcher, Scope,
    SessionHandle, SessionLocator, SessionUsage, StoreSessionRef, Stores, UsageRankingSort,
};

/// Search local coding conversation history across Harnesses.
///
/// By default `agsearch <QUERY>` searches the conversations recorded for the
/// current working directory's Project, matching a case-insensitive substring.
/// `agsearch show <SESSION>` renders a whole conversation as a Transcript.
/// `agsearch current` prints the Session that invoked this command.
#[derive(Parser)]
#[command(name = "agsearch", version, about)]
struct Cli {
    /// Override the Claude config directory. Defaults to $CLAUDE_CONFIG_DIR,
    /// then ~/.claude.
    #[arg(long, value_name = "PATH", global = true)]
    claude_dir: Option<PathBuf>,

    /// Override the Codex config directory. Defaults to $CODEX_HOME, then ~/.codex.
    #[arg(long, value_name = "PATH", global = true)]
    codex_dir: Option<PathBuf>,

    /// Search only one Harness. By default every available Store is searched.
    #[arg(long, value_enum, global = true)]
    harness: Option<HarnessChoice>,

    /// Include subagent threads in search, listing, and id resolution.
    #[arg(long, global = true)]
    include_subagents: bool,

    #[command(subcommand)]
    command: Option<Command>,

    /// The default verb: bare `agsearch <QUERY>` searches.
    #[command(flatten)]
    search: SearchArgs,
}

#[derive(Clone, Copy, ValueEnum)]
enum HarnessChoice {
    Claude,
    Codex,
}

#[derive(Subcommand)]
enum Command {
    /// Search conversations (the default verb).
    Search(SearchArgs),
    /// Render a whole Session as a Transcript.
    Show(ShowArgs),
    /// List the Sessions in a scope, newest first (no Query).
    Sessions(SessionsArgs),
    /// List the Projects in the Store, newest-touched first (no Query).
    Projects(ProjectsArgs),
    /// Show the Current Session identified by the invoking Harness.
    ///
    /// Reads `CLAUDE_CODE_SESSION_ID` or `CODEX_SESSION_ID` / `CODEX_THREAD_ID`.
    /// Fails if no supported Harness identity is available, if the Session is
    /// not in the configured Stores, or if both Harnesses identify a Session
    /// and `--harness` is omitted. Never guesses from the newest Session.
    Current(CurrentArgs),
    /// Export one Session snapshot to a file or standard output.
    ///
    /// Writes a point-in-time snapshot of one Session to one destination.
    /// Markdown (the default) is a readable document with provenance followed
    /// by the Transcript; `--format raw` copies the Harness Session file
    /// exactly. `current` exports only the top-level Session. An existing
    /// destination is rejected unless `--force` is passed.
    Export(ExportArgs),
    /// Show per-call token Usage for one Session, or rank Sessions by Usage.
    ///
    /// `agsearch usage` ranks the Sessions in scope by total token Usage
    /// (tokens only, no money), biggest first. `agsearch usage <SESSION>`
    /// shows one row per model call inside that Session, in turn order
    /// (Claude Code) or file order (Codex `token_count` Records), with
    /// a total line.
    ///
    /// The selector accepts the same forms as `show`: a git-style id prefix,
    /// `current`, or `current-thread`. Omit it to rank instead of showing one
    /// breakdown.
    ///
    /// Ranking scope reuses the `sessions` flags: `--all`, `--project <SUBSTR>`,
    /// `--harness <claude|codex>`, `--since <WHEN>`, `--include-subagents`,
    /// and `--include-current` (the ranking excludes the Current Session Family
    /// by default). Subagent threads fold into their parent row by default;
    /// `--include-subagents` lists them as their own rows instead. Ranking
    /// order is `--sort {total,output,input,calls}` (default `total`),
    /// truncated to `--limit N` (default 20). On the breakdown, `--sort`
    /// accepts only `total` (biggest-first); the default is turn order
    /// (Claude Code) or file order (Codex).
    Usage(UsageArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum UsageSort {
    /// Biggest total first. On the breakdown this flips turn order to the
    /// leaderboard; on the ranking this is the default order.
    Total,
    /// Biggest output first (ranking only).
    Output,
    /// Biggest input first (ranking only).
    Input,
    /// Most model calls first (ranking only).
    Calls,
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    /// Readable document with provenance plus the Transcript.
    Markdown,
    /// Exact copy of the Harness Session file, no added metadata.
    Raw,
}

/// Arguments for the `search` verb. Flattened into [`Cli`] so bare
/// `agsearch <QUERY>` and explicit `agsearch search <QUERY>` are identical.
#[derive(Args)]
struct SearchArgs {
    /// Text to search for (case-insensitive substring). Required for `search`,
    /// and must be non-empty ("" would match everything); under --failed it is
    /// an optional filter, where empty likewise means unfiltered. Validated at
    /// runtime so it can be omitted when a subcommand is used.
    query: Option<String>,

    /// Search across every project, not just the current directory's.
    #[arg(long, conflicts_with = "project")]
    all: bool,

    /// Search Projects whose encoded logical working-directory key contains this substring
    /// (case-insensitive).
    #[arg(long, value_name = "SUBSTR")]
    project: Option<String>,

    /// Search within one Session. Pass `current` for the top-level Current
    /// Session, `current-thread` for the calling thread, or a git-style
    /// id-prefix resolved across the whole Store.
    #[arg(long, value_name = "SELECTOR", conflicts_with_all = ["all", "project"])]
    session: Option<String>,

    /// Treat the query as a regular expression instead of a literal substring.
    #[arg(long, short = 'e')]
    regex: bool,

    /// Match case-sensitively (the default ignores case).
    #[arg(long, short = 's')]
    case_sensitive: bool,

    /// Also search assistant thinking blocks.
    #[arg(long)]
    thinking: bool,

    /// Also search tool calls and tool results.
    #[arg(long)]
    tools: bool,

    /// Search every content kind (equivalent to --thinking --tools).
    #[arg(long)]
    all_content: bool,

    /// Maximum matches shown per session; 0 means unlimited.
    #[arg(long, short = 'm', value_name = "N", default_value_t = 3)]
    max_per_session: usize,

    /// Print only the matching session file paths (for piping).
    #[arg(long, short = 'l')]
    files: bool,

    /// List failed tool calls by structure instead of searching text. The query
    /// becomes optional and, when given, filters failures by command / error.
    #[arg(long)]
    failed: bool,

    /// With --failed, print each failure's full error text instead of the
    /// single salient line.
    #[arg(long)]
    full: bool,

    /// Aggregate failures into a counts table by tool and error signature
    /// instead of listing them (the aggregate counterpart to --failed). Implies
    /// failure analysis and inherits scope, --since, and the optional Query.
    #[arg(long, conflicts_with = "file")]
    stats: bool,

    /// Only Sessions touched since this point: a relative duration (3d, 2w, 1h)
    /// or an absolute ISO date (2026-05-01). Composes with every scope.
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,

    /// Include the Current Session Family, which multi-Session analysis
    /// excludes by default so a search cannot return the conversation that
    /// asked for it. Applies to text search, --failed, --stats, and --file.
    #[arg(long)]
    include_current: bool,

    /// List file Touches by File Selector instead of searching text. With no
    /// Query, lists every Touch of the selected file grouped by Session in the
    /// same id-and-turn handoff shape as Matches. With a Query, runs the
    /// normal text search but only inside Sessions that contain a Touch of the
    /// selected file (output is the standard Match shape). Cannot be used
    /// with --failed or --stats.
    #[arg(long, value_name = "SELECTOR", conflicts_with_all = ["failed", "stats"])]
    file: Option<String>,

    /// With --file, narrow to write Touches (Edit, Write, MultiEdit,
    /// NotebookEdit, Codex apply_patch): without a Query only write Touch rows
    /// are listed; with a Query only Sessions with a write Touch are searched.
    /// Requires --file.
    #[arg(long)]
    written: bool,
}

/// Arguments for the `show` verb.
#[derive(Args)]
struct ShowArgs {
    /// A git-style unique prefix of a session-id (resolved across the whole
    /// Store), `current` for the top-level Current Session, `current-thread`
    /// for the calling thread, or `-` to read a Session file path from stdin.
    session: String,

    /// Expand assistant thinking blocks (collapsed to a count by default).
    #[arg(long)]
    thinking: bool,

    /// Render only the turns around this turn number (e.g. a turn from a search
    /// hit) instead of the whole Transcript.
    #[arg(long, value_name = "TURN")]
    around: Option<usize>,

    /// Turns of context to show on either side of --around.
    #[arg(long, value_name = "N", default_value_t = 3)]
    context: usize,
}

/// Arguments for the `sessions` verb: scope and recency only — no Query, and
/// none of search's content / failure flags (clap rejects them). Lists the
/// Sessions in scope so you can grab a short-id for `show` (ADR 0004).
#[derive(Args)]
struct SessionsArgs {
    /// List Sessions across every Project, not just the current directory's.
    #[arg(long, conflicts_with = "project")]
    all: bool,

    /// List Sessions in Projects whose encoded logical working-directory key contains this substring
    /// (case-insensitive).
    #[arg(long, value_name = "SUBSTR")]
    project: Option<String>,

    /// Only Sessions touched since this point: a relative duration (3d, 2w, 1h)
    /// or an absolute ISO date (2026-05-01).
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,

    /// Print only the Session file paths (for piping into `show -`).
    #[arg(long, short = 'l')]
    files: bool,
}

/// Arguments for the `projects` verb: a whole-Store listing with optional
/// recency and name filters. No `--all` (it is always whole-Store) and no `-l`
/// (nothing consumes a Project path) — clap rejects them. ADR 0004 / ADR 0005.
#[derive(Args)]
struct ProjectsArgs {
    /// Only Projects touched since this point: a relative duration (3d, 2w, 1h)
    /// or an absolute ISO date (2026-05-01).
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,

    /// List only Projects whose encoded logical working-directory key contains this substring
    /// (case-insensitive).
    #[arg(long, value_name = "SUBSTR")]
    project: Option<String>,
}

/// Arguments for the `current` verb: inspect the Session that invoked
/// `agsearch`. Fails rather than guessing when no Harness identity is available.
#[derive(Args)]
struct CurrentArgs {
    /// Print only the full top-level Session ID.
    #[arg(long, conflicts_with = "path")]
    id_only: bool,

    /// Print only the source Session path.
    #[arg(long)]
    path: bool,
}

/// Arguments for the `export` verb: write one Session snapshot to one
/// destination. The Session selector accepts a unique id prefix, `current`
/// for the top-level Current Session, or `current-thread` for the calling
/// thread. The destination is a file path, or `-` for standard output.
#[derive(Args)]
struct ExportArgs {
    /// A git-style unique prefix of a session-id, `current` for the top-level
    /// Current Session, or `current-thread` for the calling thread.
    session: String,

    /// Destination file path, or `-` to write the Export to standard output.
    destination: String,

    /// Export format: readable Markdown (the default) or an exact raw copy.
    #[arg(long, value_enum, default_value_t = ExportFormat::Markdown)]
    format: ExportFormat,

    /// Overwrite the destination when it already exists.
    #[arg(long)]
    force: bool,

    /// Expand assistant thinking blocks in Markdown Export (collapsed by
    /// default, like `show`).
    #[arg(long)]
    thinking: bool,
}

/// Arguments for the `usage` verb: per-call breakdown for one Session, or a
/// Usage ranking across the scope. The selector accepts the same forms as
/// `show` (id prefix, `current`, `current-thread`). Without a selector the
/// scope flags (`--all`, `--project`, `--since`, `--include-current`) behave
/// exactly like `sessions`, ranking the Sessions in scope by summed Usage.
#[derive(Args)]
struct UsageArgs {
    /// A git-style unique prefix of a session-id, `current` for the top-level
    /// Current Session, or `current-thread` for the calling thread. Omit to
    /// rank the Sessions in scope instead of showing one breakdown.
    session: Option<String>,

    /// Reorder the rows: biggest-total-first on the breakdown (default is turn
    /// order); on the ranking, rank by this column (default `total`).
    #[arg(long, value_enum)]
    sort: Option<UsageSort>,

    /// List Sessions across every Project, not just the current directory's
    /// (ranking only; ignored with a selector).
    #[arg(long, conflicts_with = "project")]
    all: bool,

    /// List Sessions in Projects whose encoded logical working-directory key
    /// contains this substring, case-insensitive (ranking only; ignored with
    /// a selector).
    #[arg(long, value_name = "SUBSTR")]
    project: Option<String>,

    /// Only Sessions touched since this point: a relative duration (3d, 2w,
    /// 1h) or an absolute ISO date (2026-05-01) (ranking only; ignored with
    /// a selector).
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,

    /// Include the Current Session Family, which the ranking excludes by
    /// default so the conversation asking the question does not pollute the
    /// answer (ranking only; ignored with a selector).
    #[arg(long)]
    include_current: bool,

    /// Maximum Sessions shown in the ranking (default 20; ranking only;
    /// ignored with a selector).
    #[arg(long, value_name = "N", default_value_t = 20)]
    limit: usize,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let env_config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let env_codex_home = std::env::var("CODEX_HOME").ok();
    let home = dirs::home_dir();
    let Some(claude_dir) = resolve_claude_dir(
        cli.claude_dir.as_deref(),
        env_config_dir.as_deref(),
        home.as_deref(),
    ) else {
        eprintln!(
            "agsearch: could not determine the Claude config directory; \
             pass --claude-dir or set $CLAUDE_CONFIG_DIR"
        );
        return ExitCode::FAILURE;
    };
    let Some(codex_dir) = resolve_codex_dir(
        cli.codex_dir.as_deref(),
        env_codex_home.as_deref(),
        home.as_deref(),
    ) else {
        eprintln!(
            "agsearch: could not determine the Codex config directory; \
             pass --codex-dir or set $CODEX_HOME"
        );
        return ExitCode::FAILURE;
    };

    let selected = cli.harness.map(|choice| match choice {
        HarnessChoice::Claude => Harness::Claude,
        HarnessChoice::Codex => Harness::Codex,
    });
    let stores = Stores::with_claude_and_codex(&claude_dir, &codex_dir)
        .selecting(selected)
        .including_subagents(cli.include_subagents);

    match cli.command {
        Some(Command::Show(args)) => run_show(&stores, &args),
        Some(Command::Search(args)) => run_search(stores, &args),
        Some(Command::Sessions(args)) => run_sessions(&stores, &args),
        Some(Command::Projects(args)) => run_projects(&stores, &args),
        Some(Command::Current(args)) => run_current(&stores, &args),
        Some(Command::Export(args)) => run_export(&stores, &args),
        Some(Command::Usage(args)) => run_usage(stores, &args),
        None => run_search(stores, &cli.search),
    }
}

/// Parse an optional `--since` value into a Unix cutoff, printing the standard
/// error and signalling failure when it is unparseable. `Ok(None)` means no
/// `--since` was given; `Err` means parse failed (caller should exit).
fn parse_since(value: Option<&str>) -> Result<Option<i64>, ()> {
    let Some(value) = value else { return Ok(None) };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    match since_cutoff(value, now) {
        Some(cutoff) => Ok(Some(cutoff)),
        None => {
            eprintln!(
                "agsearch: could not parse --since '{value}' \
                 (use a duration like 3d/2w/1h, or an ISO date like 2026-05-01)"
            );
            Err(())
        }
    }
}

/// Run the `current` verb: resolve the invoking Harness's Current Session and
/// print it. Never infers identity from the newest Session in the Store.
fn run_current(stores: &Stores, args: &CurrentArgs) -> ExitCode {
    match resolve_current_context(stores) {
        Ok(context) => {
            let rendered = if args.id_only {
                format!("{}\n", context.session.session_id)
            } else if args.path {
                format!("{}\n", context.session.locator)
            } else {
                format_current(&context)
            };
            let _ = write!(anstream::stdout(), "{rendered}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("agsearch: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Run the `sessions` verb: resolve the scope, list its Sessions newest-first,
/// apply `--since`, then render the headers (or just paths under `-l`).
fn run_sessions(stores: &Stores, args: &SessionsArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("agsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let cutoff = match parse_since(args.since.as_deref()) {
        Ok(cutoff) => cutoff,
        Err(()) => return ExitCode::FAILURE,
    };

    let scope = build_scope(args.all, args.project.as_deref(), &cwd);
    let mut sessions = list_store_sessions(stores, &scope);
    if let Some(cutoff) = cutoff {
        sessions.retain(|s| timestamp_is_since(s.timestamp.as_deref(), cutoff));
    }

    let rendered = if args.files {
        format_session_paths(&sessions)
    } else {
        format_sessions(&sessions, &stores.short_ids())
    };
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Run the `projects` verb: list the Store's Projects (optionally narrowed to
/// directories whose name matches `--project`), newest-touched first, applying
/// `--since`. Always whole-Store, so it needs no cwd or `build_scope`.
fn run_projects(stores: &Stores, args: &ProjectsArgs) -> ExitCode {
    let cutoff = match parse_since(args.since.as_deref()) {
        Ok(cutoff) => cutoff,
        Err(()) => return ExitCode::FAILURE,
    };

    let scope = match args.project.as_deref() {
        Some(name_substring) => Scope::Project {
            name_substring: name_substring.to_string(),
        },
        None => Scope::All,
    };
    let mut projects = list_store_projects(stores, &scope);
    if let Some(cutoff) = cutoff {
        projects.retain(|p| timestamp_is_since(p.last_touched.as_deref(), cutoff));
    }

    let _ = write!(anstream::stdout(), "{}", format_projects(&projects));
    ExitCode::SUCCESS
}

/// Resolve a session selector (`current`, `current-thread`, or an id prefix)
/// to one Session, printing the standard error and returning failure on any
/// miss. Shared by `show`, `export`, and search `--session` so the handoff
/// from any listing verb behaves identically.
fn resolve_session_selector(stores: &Stores, selector: &str) -> Result<SessionHandle, ExitCode> {
    if selector == "current" {
        return resolve_current_session(stores).map_err(|err| {
            eprintln!("agsearch: {err}");
            ExitCode::FAILURE
        });
    }
    if selector == "current-thread" {
        return resolve_current_thread(stores).map_err(|err| {
            eprintln!("agsearch: {err}");
            ExitCode::FAILURE
        });
    }
    match resolve_store_session_prefix(stores, selector) {
        StoreSessionRef::Unique(session) => Ok(session),
        StoreSessionRef::NotFound => {
            eprintln!("agsearch: no session matches '{selector}'");
            Err(ExitCode::FAILURE)
        }
        StoreSessionRef::Ambiguous(ids) => {
            report_ambiguous(selector, &ids);
            Err(ExitCode::FAILURE)
        }
    }
}

/// Like [`resolve_session_selector`] but including subagent threads. The
/// `usage` breakdown uses this so a worker row always hands off.
fn resolve_session_selector_including_subagents(
    stores: &Stores,
    selector: &str,
) -> Result<SessionHandle, ExitCode> {
    if selector == "current" {
        return resolve_current_session(stores).map_err(|err| {
            eprintln!("agsearch: {err}");
            ExitCode::FAILURE
        });
    }
    if selector == "current-thread" {
        return resolve_current_thread(stores).map_err(|err| {
            eprintln!("agsearch: {err}");
            ExitCode::FAILURE
        });
    }
    match resolve_store_session_prefix_including_subagents(stores, selector) {
        StoreSessionRef::Unique(session) => Ok(session),
        StoreSessionRef::NotFound => {
            eprintln!("agsearch: no session matches '{selector}'");
            Err(ExitCode::FAILURE)
        }
        StoreSessionRef::Ambiguous(ids) => {
            report_ambiguous(selector, &ids);
            Err(ExitCode::FAILURE)
        }
    }
}

fn report_ambiguous(selector: &str, ids: &[String]) {
    eprintln!(
        "agsearch: '{selector}' is ambiguous — {} sessions match:",
        ids.len()
    );
    for id in ids.iter().take(10) {
        eprintln!("  {id}");
    }
    if ids.len() > 10 {
        eprintln!("  … and {} more", ids.len() - 10);
    }
}

/// Run the `search` verb: resolve the scope, then either list Touches by
/// structure (`--file`, no Query), search text restricted to touching Sessions
/// (`--file` with a Query), list Failures by structure (`--failed`, Query
/// optional), or search text (Query required).
fn run_search(stores: Stores, args: &SearchArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("agsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };

    // --session resolves to one Session file; otherwise we operate over the
    // Project scope. (--session conflicts with --all / --project.)
    // `current` selects the top-level Current Session, `current-thread`
    // selects the calling thread (the same Session at the top level). Both
    // resolve through the current-context module, so unavailable and ambiguous
    // context fails with the resolver's error and a worker thread is
    // accessible without --include-subagents.
    let session_path = match &args.session {
        Some(selector) => match resolve_session_selector(&stores, selector) {
            Ok(session) => Some(session),
            Err(code) => return code,
        },
        None => None,
    };

    // A Query, compiled — required for text search, optional under --failed.
    // An empty string is treated as missing: "" matches every Record, so a
    // shell variable expanding to empty would otherwise dump the whole scope.
    // Under --failed the query is an optional filter, so empty = unfiltered.
    // Whitespace-only stays valid — a user who quotes a space may mean it.
    let matcher = match args.query.as_deref().filter(|q| !q.is_empty()) {
        Some(query) => match Matcher::new(query, args.regex, args.case_sensitive) {
            Ok(matcher) => Some(matcher),
            Err(err) => {
                eprintln!("agsearch: {err}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    // Optional recency cutoff, applied to whichever result set we produce.
    let cutoff = match parse_since(args.since.as_deref()) {
        Ok(cutoff) => cutoff,
        Err(()) => return ExitCode::FAILURE,
    };

    // Multi-Session analysis hides the Current Session Family so a request to
    // recall earlier work cannot match the conversation that made it (ADR
    // 0011). Naming a Session is intent to include it, so --session skips the
    // exclusion, as does --include-current.
    let stores = if args.include_current || session_path.is_some() {
        stores
    } else {
        stores.excluding_current_family()
    };

    // --file with no Query lists Touches by structure; --file with a Query
    // (issue 27) runs the normal text search but only inside Sessions that
    // contain a Touch of the selected file. --failed/--stats with --file are
    // rejected by clap conflicts. --written narrows --file to write Touches,
    // so it requires --file.
    if args.written && args.file.is_none() {
        eprintln!("agsearch: --written requires --file");
        return ExitCode::FAILURE;
    }
    if let Some(selector_str) = args.file.as_deref() {
        let selector = match FileSelector::parse(selector_str) {
            Ok(selector) => selector,
            Err(msg) => {
                eprintln!("agsearch: {msg}");
                return ExitCode::FAILURE;
            }
        };
        if let Some(matcher) = matcher.as_ref() {
            let content = ContentSet {
                thinking: args.thinking || args.all_content,
                tools: args.tools || args.all_content,
            };
            let mut results = match &session_path {
                Some(session) => search_store_session_with_file(
                    &stores,
                    session,
                    matcher,
                    &content,
                    &selector,
                    args.written,
                ),
                None => search_stores_with_file(
                    &stores,
                    &build_scope(args.all, args.project.as_deref(), &cwd),
                    matcher,
                    &content,
                    &selector,
                    args.written,
                ),
            };
            if let Some(cutoff) = cutoff {
                results.retain(|r| timestamp_is_since(r.timestamp.as_deref(), cutoff));
            }
            // Let anstream decide whether colour is wanted, as in plain search.
            let color =
                anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never;
            let rendered = if args.files {
                format_paths(&results)
            } else {
                format_results(
                    &results,
                    matcher,
                    args.max_per_session,
                    color,
                    &stores.short_ids(),
                )
            };
            let _ = write!(anstream::stdout(), "{rendered}");
            return ExitCode::SUCCESS;
        }
        let mut results = match &session_path {
            Some(session) => touches_in_store_session(&stores, session, &selector, args.written),
            None => touches_in_stores(
                &stores,
                &build_scope(args.all, args.project.as_deref(), &cwd),
                &selector,
                args.written,
            ),
        };
        if let Some(cutoff) = cutoff {
            results.retain(|r| timestamp_is_since(r.timestamp.as_deref(), cutoff));
        }
        let rendered = if args.files {
            format_touch_paths(&results)
        } else {
            format_touches(&results, args.max_per_session, &stores.short_ids())
        };
        let _ = write!(anstream::stdout(), "{rendered}");
        return ExitCode::SUCCESS;
    }

    // --stats and --failed share one scan; --stats aggregates, --failed lists.
    if args.failed || args.stats {
        let mut results = match &session_path {
            Some(session) => failed_in_store_session(&stores, session, matcher.as_ref()),
            None => failed_in_stores(
                &stores,
                &build_scope(args.all, args.project.as_deref(), &cwd),
                matcher.as_ref(),
            ),
        };
        if let Some(cutoff) = cutoff {
            results.retain(|r| timestamp_is_since(r.timestamp.as_deref(), cutoff));
        }
        let rendered = if args.stats {
            format_stats(&group_failures(&results))
        } else {
            format_failures(
                &results,
                args.max_per_session,
                args.full,
                &stores.short_ids(),
            )
        };
        let _ = write!(anstream::stdout(), "{rendered}");
        return ExitCode::SUCCESS;
    }

    let Some(matcher) = matcher else {
        eprintln!(
            "agsearch: a query is required (or use `agsearch show <session>`, or `--failed`)"
        );
        return ExitCode::FAILURE;
    };

    let content = ContentSet {
        thinking: args.thinking || args.all_content,
        tools: args.tools || args.all_content,
    };
    let mut results = match &session_path {
        Some(session) => search_store_session(&stores, session, &matcher, &content),
        None => search_stores(
            &stores,
            &build_scope(args.all, args.project.as_deref(), &cwd),
            &matcher,
            &content,
        ),
    };
    if let Some(cutoff) = cutoff {
        results.retain(|r| timestamp_is_since(r.timestamp.as_deref(), cutoff));
    }

    // Let anstream decide whether colour is wanted (TTY, NO_COLOR, CLICOLOR_*)
    // and strip codes on the way out when it is not.
    let color = anstream::AutoStream::choice(&std::io::stdout()) != anstream::ColorChoice::Never;
    let rendered = if args.files {
        format_paths(&results)
    } else {
        format_results(
            &results,
            &matcher,
            args.max_per_session,
            color,
            &stores.short_ids(),
        )
    };
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// The scope a verb covers when not pinned to a single Session: `--all`,
/// `--project <substr>`, or the current working directory's Project. Shared by
/// `search` and `sessions` so their scope flags behave identically (ADR 0004).
fn build_scope(all: bool, project: Option<&str>, cwd: &Path) -> Scope {
    if all {
        Scope::All
    } else if let Some(name_substring) = project {
        Scope::Project {
            name_substring: name_substring.to_string(),
        }
    } else {
        Scope::Current {
            cwd: cwd.to_string_lossy().into_owned(),
        }
    }
}

/// Run the `show` verb: resolve the Session (by prefix, current-context
/// selector, or a path from stdin), then render it as a Transcript.
fn run_show(stores: &Stores, args: &ShowArgs) -> ExitCode {
    let (turns, harness) = if args.session == "-" {
        match read_locator_from_stdin() {
            Some(locator) => match parse_transcript_locator(&locator) {
                Some((harness, turns)) => (turns, harness),
                None => {
                    eprintln!("agsearch: cannot read {locator}");
                    return ExitCode::FAILURE;
                }
            },
            None => {
                eprintln!("agsearch: no session path on stdin");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let session = match resolve_session_selector(stores, &args.session) {
            Ok(session) => session,
            Err(code) => return code,
        };
        let Some(turns) = parse_store_transcript(stores, &session) else {
            eprintln!("agsearch: cannot read {}", session.info.locator);
            return ExitCode::FAILURE;
        };
        (turns, session.info.harness)
    };
    // --around windows the Transcript on a turn (from a search hit); without it
    // the whole Transcript is rendered. --context only applies inside a window.
    let rendered = match args.around {
        Some(around) => {
            format_windowed_for_harness(&turns, around, args.context, args.thinking, harness)
        }
        None => format_transcript_for_harness(&turns, args.thinking, harness),
    };
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Run the `export` verb: resolve one Session (by prefix or current-context
/// selector), then write one point-in-time snapshot to one destination.
///
/// Markdown (the default) renders provenance plus the Transcript via the
/// existing renderer; raw copies the Harness Session file byte-for-byte.
/// Destination `-` writes to standard output. An existing file destination is
/// rejected unless `--force` is passed. Every Export captures the content
/// available when the command runs and finishes without waiting for the
/// Harness.
fn run_export(stores: &Stores, args: &ExportArgs) -> ExitCode {
    let handle = match resolve_session_selector(stores, &args.session) {
        Ok(handle) => handle,
        Err(code) => return code,
    };

    match args.format {
        ExportFormat::Raw => {
            let bytes = match stores.read_raw(&handle) {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("agsearch: cannot read {}: {err}", handle.info.locator);
                    return ExitCode::FAILURE;
                }
            };
            if args.destination == "-" {
                let mut stdout = std::io::stdout();
                if stdout.write_all(&bytes).is_err() {
                    eprintln!("agsearch: cannot write to standard output");
                    return ExitCode::FAILURE;
                }
                return ExitCode::SUCCESS;
            }
            let dest = PathBuf::from(&args.destination);
            if dest.is_dir() {
                eprintln!("agsearch: destination '{}' is a directory", dest.display());
                return ExitCode::FAILURE;
            }
            if dest.exists() && !args.force {
                eprintln!(
                    "agsearch: destination '{}' already exists (pass --force to overwrite)",
                    dest.display()
                );
                return ExitCode::FAILURE;
            }
            if let Err(err) = std::fs::write(&dest, &bytes) {
                eprintln!("agsearch: cannot write {}: {err}", dest.display());
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        ExportFormat::Markdown => {
            let Some(turns) = parse_store_transcript(stores, &handle) else {
                eprintln!("agsearch: cannot read {}", handle.info.locator);
                return ExitCode::FAILURE;
            };
            let rendered = format_export_markdown(
                &handle.info,
                &turns,
                args.thinking,
                &export_timestamp_now(),
            );
            if args.destination == "-" {
                let _ = write!(anstream::stdout(), "{rendered}");
                return ExitCode::SUCCESS;
            }
            let dest = PathBuf::from(&args.destination);
            if dest.is_dir() {
                eprintln!("agsearch: destination '{}' is a directory", dest.display());
                return ExitCode::FAILURE;
            }
            if dest.exists() && !args.force {
                eprintln!(
                    "agsearch: destination '{}' already exists (pass --force to overwrite)",
                    dest.display()
                );
                return ExitCode::FAILURE;
            }
            if let Err(err) = std::fs::write(&dest, rendered.as_bytes()) {
                eprintln!("agsearch: cannot write {}: {err}", dest.display());
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
    }
}

/// Run the `usage` verb: with a selector, show the per-call breakdown for one
/// Session; without one, rank the Sessions in scope by summed Usage.
/// The selector accepts the same forms as `show` (id prefix, `current`,
/// `current-thread`) and resolves through the centralized current-context
/// resolver (ADR 0014), so the handoff from any listing verb works. The
/// ranking reuses the `sessions` scope flags (`--all`, `--project`, `--since`)
/// plus `--include-current`, excludes the Current Session Family by default
/// (ADR 0011), and orders by `--sort` truncated to `--limit`.
fn run_usage(stores: Stores, args: &UsageArgs) -> ExitCode {
    if let Some(selector) = args.session.as_deref() {
        return run_usage_breakdown(&stores, args, selector);
    }
    run_usage_ranking(stores, args)
}

/// Run the `usage` breakdown for one explicitly selected Session. Scope and
/// ranking flags are ignored here; only `--sort total` applies (other sorts
/// belong to the ranking).
fn run_usage_breakdown(stores: &Stores, args: &UsageArgs, selector: &str) -> ExitCode {
    if let Some(sort) = args.sort {
        if !matches!(sort, UsageSort::Total) {
            let name = match sort {
                UsageSort::Output => "output",
                UsageSort::Input => "input",
                UsageSort::Calls => "calls",
                UsageSort::Total => "total",
            };
            eprintln!(
                "agsearch: --sort {name} is only valid for usage ranking (without a session selector)"
            );
            return ExitCode::FAILURE;
        }
    }
    let handle = match resolve_session_selector_including_subagents(stores, selector) {
        Ok(handle) => handle,
        Err(code) => return code,
    };

    let Some(breakdown) = usage_calls_for_session(stores, &handle) else {
        eprintln!("agsearch: cannot read {}", handle.info.locator);
        return ExitCode::FAILURE;
    };
    // A Codex Session whose summed turns disagree with its final running
    // total keeps the sums (the file may be truncated or double-logged).
    if let Some(warning) = breakdown.warning {
        eprintln!("agsearch: warning: {warning}");
    }
    let sort_total = matches!(args.sort, Some(UsageSort::Total));
    let rendered = format_usage_breakdown(&breakdown.calls, sort_total, &stores.short_ids());
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Run the `usage` ranking: list the Sessions in scope ordered by summed
/// Usage, truncated to `--limit`. Scope (`--all`, `--project`, `--harness`,
/// `--since`) behaves exactly like `sessions`; the Current Session Family is
/// excluded unless `--include-current` is passed. Sessions with no Usage are
/// omitted and counted on the final skipped line.
fn run_usage_ranking(stores: Stores, args: &UsageArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("agsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let cutoff = match parse_since(args.since.as_deref()) {
        Ok(cutoff) => cutoff,
        Err(()) => return ExitCode::FAILURE,
    };

    // Multi-Session analysis hides the Current Session Family so a request to
    // recall earlier work cannot match the conversation that made it (ADR
    // 0011), unless --include-current puts it back.
    let stores = if args.include_current {
        stores
    } else {
        stores.excluding_current_family()
    };

    let scope = build_scope(args.all, args.project.as_deref(), &cwd);
    let sort = match args.sort.unwrap_or(UsageSort::Total) {
        UsageSort::Total => UsageRankingSort::Total,
        UsageSort::Output => UsageRankingSort::Output,
        UsageSort::Input => UsageRankingSort::Input,
        UsageSort::Calls => UsageRankingSort::Calls,
    };
    let (mut rows, skipped): (Vec<SessionUsage>, usize) =
        usage_ranking(&stores, &scope, cutoff, sort);
    rows.truncate(args.limit);

    let rendered = format_usage_ranking(&rows, skipped, &stores.short_ids());
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Read the first non-empty line of stdin as a Session file path (for
/// `agsearch -l … | … | agsearch show -`).
fn read_locator_from_stdin() -> Option<SessionLocator> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok()?;
    input
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| SessionLocator::File(PathBuf::from(line)))
}
