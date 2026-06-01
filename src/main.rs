use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use ccsearch::{
    format_paths, format_results, format_transcript, parse_transcript, projects_root,
    resolve_claude_dir, resolve_scope, resolve_session_prefix, search_project_dirs, ContentSet,
    Matcher, Scope, SessionRef,
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
        None => run_search(&claude_dir, &cli.search),
    }
}

/// Run the `search` verb: resolve the scope, compile the Query, scan, render.
fn run_search(claude_dir: &Path, args: &SearchArgs) -> ExitCode {
    let Some(query) = args.query.as_deref() else {
        eprintln!("ccsearch: a query is required (or use `ccsearch show <session>`)");
        return ExitCode::FAILURE;
    };

    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("ccsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };

    let scope = if args.all {
        Scope::All
    } else if let Some(name_substring) = args.project.clone() {
        Scope::Project { name_substring }
    } else {
        Scope::Current { cwd: cwd.to_string_lossy().into_owned() }
    };

    let matcher = match Matcher::new(query, args.regex, args.case_sensitive) {
        Ok(matcher) => matcher,
        Err(err) => {
            eprintln!("ccsearch: {err}");
            return ExitCode::FAILURE;
        }
    };

    let root = projects_root(claude_dir);
    let project_dirs = resolve_scope(&root, &scope);
    let content = ContentSet {
        thinking: args.thinking || args.all_content,
        tools: args.tools || args.all_content,
    };
    let results = search_project_dirs(&project_dirs, &matcher, &content);

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
    let rendered = format_transcript(&turns, args.thinking);
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
