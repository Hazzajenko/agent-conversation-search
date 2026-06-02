use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};

use ccsearch::{
    failed_in_project_dirs, failed_in_session_file, format_failures, format_paths, format_results,
    format_session_paths, format_sessions, format_stats, format_transcript, format_windowed,
    group_failures, list_sessions, parse_transcript, projects_root, resolve_claude_dir,
    resolve_scope, resolve_session_prefix, search_project_dirs, search_session_file, since_cutoff,
    timestamp_is_since, ContentSet, Matcher, Scope, SessionRef,
};

/// Search your local Claude Code conversation history.
///
/// By default `ccsearch <QUERY>` searches the conversations recorded for the
/// current working directory's Project, matching a case-insensitive substring.
/// `ccsearch show <SESSION>` renders a whole conversation as a Transcript.
#[derive(Parser)]
#[command(name = "ccsearch", version, about)]
struct Cli {
    /// Override the Claude config directory. Defaults to $CLAUDE_CONFIG_DIR,
    /// then ~/.claude.
    #[arg(long, value_name = "PATH", global = true)]
    claude_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,

    /// The default verb: bare `ccsearch <QUERY>` searches.
    #[command(flatten)]
    search: SearchArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Search conversations (the default verb).
    Search(SearchArgs),
    /// Render a whole Session as a Transcript.
    Show(ShowArgs),
    /// List the Sessions in a scope, newest first (no Query).
    Sessions(SessionsArgs),
}

/// Arguments for the `search` verb. Flattened into [`Cli`] so bare
/// `ccsearch <QUERY>` and explicit `ccsearch search <QUERY>` are identical.
#[derive(Args)]
struct SearchArgs {
    /// Text to search for (case-insensitive substring). Required for `search`;
    /// validated at runtime so it can be omitted when a subcommand is used.
    query: Option<String>,

    /// Search across every project, not just the current directory's.
    #[arg(long, conflicts_with = "project")]
    all: bool,

    /// Search projects whose directory name contains this substring
    /// (case-insensitive).
    #[arg(long, value_name = "SUBSTR")]
    project: Option<String>,

    /// Search within a single Session, resolved by git-style id-prefix across
    /// the whole Store (the "where in this conversation" scope).
    #[arg(long, value_name = "PREFIX", conflicts_with_all = ["all", "project"])]
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
    #[arg(long)]
    stats: bool,

    /// Only Sessions touched since this point: a relative duration (3d, 2w, 1h)
    /// or an absolute ISO date (2026-05-01). Composes with every scope.
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,
}

/// Arguments for the `show` verb.
#[derive(Args)]
struct ShowArgs {
    /// A git-style unique prefix of a session-id (resolved across the whole
    /// Store), or `-` to read a Session file path from stdin.
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

    /// List Sessions in Projects whose directory name contains this substring
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

fn main() -> ExitCode {
    let cli = Cli::parse();

    let env_config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let home = dirs::home_dir();
    let Some(claude_dir) = resolve_claude_dir(
        cli.claude_dir.as_deref(),
        env_config_dir.as_deref(),
        home.as_deref(),
    ) else {
        eprintln!(
            "ccsearch: could not determine the Claude config directory; \
             pass --claude-dir or set $CLAUDE_CONFIG_DIR"
        );
        return ExitCode::FAILURE;
    };

    match cli.command {
        Some(Command::Show(args)) => run_show(&claude_dir, &args),
        Some(Command::Search(args)) => run_search(&claude_dir, &args),
        Some(Command::Sessions(args)) => run_sessions(&claude_dir, &args),
        None => run_search(&claude_dir, &cli.search),
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
                "ccsearch: could not parse --since '{value}' \
                 (use a duration like 3d/2w/1h, or an ISO date like 2026-05-01)"
            );
            Err(())
        }
    }
}

/// Run the `sessions` verb: resolve the scope, list its Sessions newest-first,
/// apply `--since`, then render the headers (or just paths under `-l`).
fn run_sessions(claude_dir: &Path, args: &SessionsArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("ccsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let root = projects_root(claude_dir);

    let cutoff = match parse_since(args.since.as_deref()) {
        Ok(cutoff) => cutoff,
        Err(()) => return ExitCode::FAILURE,
    };

    let scope = build_scope(args.all, args.project.as_deref(), &cwd);
    let mut sessions = list_sessions(&resolve_scope(&root, &scope));
    if let Some(cutoff) = cutoff {
        sessions.retain(|s| timestamp_is_since(s.timestamp.as_deref(), cutoff));
    }

    let rendered = if args.files {
        format_session_paths(&sessions)
    } else {
        format_sessions(&sessions)
    };
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Run the `search` verb: resolve the scope, then either list Failures by
/// structure (`--failed`, Query optional) or search text (Query required).
fn run_search(claude_dir: &Path, args: &SearchArgs) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("ccsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };

    let root = projects_root(claude_dir);

    // --session resolves to one Session file; otherwise we operate over the
    // scope's Project dirs. (--session conflicts with --all / --project.)
    let session_path = match &args.session {
        Some(prefix) => match resolve_session_prefix(&root, prefix) {
            SessionRef::Unique(path) => Some(path),
            SessionRef::NotFound => {
                eprintln!("ccsearch: no session matches '{prefix}'");
                return ExitCode::FAILURE;
            }
            SessionRef::Ambiguous(ids) => {
                eprintln!("ccsearch: '{prefix}' is ambiguous — {} sessions match:", ids.len());
                for id in ids.iter().take(10) {
                    eprintln!("  {id}");
                }
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    // A Query, compiled — required for text search, optional under --failed.
    let matcher = match &args.query {
        Some(query) => match Matcher::new(query, args.regex, args.case_sensitive) {
            Ok(matcher) => Some(matcher),
            Err(err) => {
                eprintln!("ccsearch: {err}");
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

    // --stats and --failed share one scan; --stats aggregates, --failed lists.
    if args.failed || args.stats {
        let mut results = match &session_path {
            Some(path) => failed_in_session_file(path, matcher.as_ref()),
            None => failed_in_project_dirs(&resolve_scope(&root, &build_scope(args.all, args.project.as_deref(), &cwd)), matcher.as_ref()),
        };
        if let Some(cutoff) = cutoff {
            results.retain(|r| timestamp_is_since(r.timestamp.as_deref(), cutoff));
        }
        let rendered = if args.stats {
            format_stats(&group_failures(&results))
        } else {
            format_failures(&results, args.max_per_session, args.full)
        };
        let _ = write!(anstream::stdout(), "{rendered}");
        return ExitCode::SUCCESS;
    }

    let Some(matcher) = matcher else {
        eprintln!("ccsearch: a query is required (or use `ccsearch show <session>`, or `--failed`)");
        return ExitCode::FAILURE;
    };

    let content = ContentSet {
        thinking: args.thinking || args.all_content,
        tools: args.tools || args.all_content,
    };
    let mut results = match &session_path {
        Some(path) => search_session_file(path, &matcher, &content),
        None => search_project_dirs(&resolve_scope(&root, &build_scope(args.all, args.project.as_deref(), &cwd)), &matcher, &content),
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
        format_results(&results, &matcher, args.max_per_session, color)
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
        Scope::Project { name_substring: name_substring.to_string() }
    } else {
        Scope::Current { cwd: cwd.to_string_lossy().into_owned() }
    }
}

/// Run the `show` verb: resolve the Session (by prefix, or a path from stdin),
/// then render it as a Transcript.
fn run_show(claude_dir: &Path, args: &ShowArgs) -> ExitCode {
    let path = if args.session == "-" {
        match read_path_from_stdin() {
            Some(path) => path,
            None => {
                eprintln!("ccsearch: no session path on stdin");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let root = projects_root(claude_dir);
        match resolve_session_prefix(&root, &args.session) {
            SessionRef::Unique(path) => path,
            SessionRef::NotFound => {
                eprintln!("ccsearch: no session matches '{}'", args.session);
                return ExitCode::FAILURE;
            }
            SessionRef::Ambiguous(ids) => {
                eprintln!(
                    "ccsearch: '{}' is ambiguous — {} sessions match:",
                    args.session,
                    ids.len()
                );
                for id in ids.iter().take(10) {
                    eprintln!("  {id}");
                }
                if ids.len() > 10 {
                    eprintln!("  … and {} more", ids.len() - 10);
                }
                return ExitCode::FAILURE;
            }
        }
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("ccsearch: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let turns = parse_transcript(&text);
    // --around windows the Transcript on a turn (from a search hit); without it
    // the whole Transcript is rendered. --context only applies inside a window.
    let rendered = match args.around {
        Some(around) => format_windowed(&turns, around, args.context, args.thinking),
        None => format_transcript(&turns, args.thinking),
    };
    let _ = write!(anstream::stdout(), "{rendered}");
    ExitCode::SUCCESS
}

/// Read the first non-empty line of stdin as a Session file path (for
/// `ccsearch -l … | … | ccsearch show -`).
fn read_path_from_stdin() -> Option<PathBuf> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok()?;
    input
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}
