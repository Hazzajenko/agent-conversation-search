use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use ccsearch::{
    find_project_dirs, format_results, projects_root, resolve_claude_dir, search_project_dirs,
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

    let root = projects_root(&claude_dir);
    let project_dirs = find_project_dirs(&root, &cwd.to_string_lossy());
    let results = search_project_dirs(&project_dirs, &cli.query);

    print!("{}", format_results(&results));
    ExitCode::SUCCESS
}
