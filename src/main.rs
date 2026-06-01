use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use ccsearch::{
    format_results, projects_root, resolve_claude_dir, resolve_scope, search_project_dirs,
    ContentSet, Matcher, Scope,
};

/// Search your local Claude Code conversation history.
///
/// By default `ccsearch` searches the conversations recorded for the current
/// working directory's Project, matching a case-insensitive substring.
#[derive(Parser)]
#[command(name = "ccsearch", version, about)]
struct Cli {
    /// Text to search for (case-insensitive substring).
    query: String,

    /// Override the Claude config directory. Defaults to $CLAUDE_CONFIG_DIR,
    /// then ~/.claude.
    #[arg(long, value_name = "PATH")]
    claude_dir: Option<PathBuf>,

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

    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("ccsearch: cannot read the current directory: {err}");
            return ExitCode::FAILURE;
        }
    };

    let scope = if cli.all {
        Scope::All
    } else if let Some(name_substring) = cli.project {
        Scope::Project { name_substring }
    } else {
        Scope::Current { cwd: cwd.to_string_lossy().into_owned() }
    };

    let matcher = match Matcher::new(&cli.query, cli.regex, cli.case_sensitive) {
        Ok(matcher) => matcher,
        Err(err) => {
            eprintln!("ccsearch: {err}");
            return ExitCode::FAILURE;
        }
    };

    let root = projects_root(&claude_dir);
    let project_dirs = resolve_scope(&root, &scope);
    let content = ContentSet {
        thinking: cli.thinking || cli.all_content,
        tools: cli.tools || cli.all_content,
    };
    let results = search_project_dirs(&project_dirs, &matcher, &content);

    print!("{}", format_results(&results, &matcher, cli.max_per_session));
    ExitCode::SUCCESS
}
