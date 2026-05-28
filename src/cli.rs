//! Command-line interface: `search`, `find-related`, `init`, and `savings`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::embed::load_model;
use crate::index::SemejaIndex;
use crate::stats::{default_stats_file, format_savings_report};
use crate::utils::{format_results, is_git_url, resolve_chunk};

/// The embedded Claude Code sub-agent definition.
pub const AGENT_FILE: &str = include_str!("../agents/semeja-search.md");

/// First-argument values that route to the CLI rather than the MCP server.
pub const CLI_COMMANDS: &[&str] = &["search", "find-related", "init", "savings", "-h", "--help"];

/// Relative path of the Claude Code sub-agent file.
pub fn claude_file_path() -> PathBuf {
    PathBuf::from(".claude").join("agents").join("semeja-search.md")
}

/// The captured result of a CLI invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct CliOutcome {
    /// Text written to standard output.
    pub stdout: String,
    /// Text written to standard error.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: i32,
}

impl CliOutcome {
    fn out(stdout: impl Into<String>) -> CliOutcome {
        CliOutcome { stdout: stdout.into(), stderr: String::new(), exit_code: 0 }
    }

    fn err(stderr: impl Into<String>) -> CliOutcome {
        CliOutcome { stdout: String::new(), stderr: stderr.into(), exit_code: 1 }
    }
}

/// Run the CLI for the given argument vector (including the program name).
pub fn run(args: &[String]) -> CliOutcome {
    let options = Options::parse(args);
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => CliOutcome::out(help_text()),
        Some("init") => match run_init(Path::new("."), options.force) {
            Ok(message) => CliOutcome::out(message),
            Err(message) => CliOutcome::err(message),
        },
        Some("savings") => {
            CliOutcome::out(format_savings_report(&default_stats_file(), options.verbose))
        }
        Some("search") => run_search(&options),
        Some("find-related") => run_find_related(&options),
        _ => CliOutcome::out(help_text()),
    }
}

/// Write the Claude Code sub-agent file under `base`.
pub fn run_init(base: &Path, force: bool) -> Result<String, String> {
    let dest = base.join(claude_file_path());
    let display = claude_file_path();
    if dest.exists() && !force {
        return Err(format!("{} already exists. Run with --force to overwrite.", display.display()));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&dest, AGENT_FILE).map_err(|e| e.to_string())?;
    Ok(format!("Created {}", display.display()))
}

// --- Subcommand handlers ---

fn run_search(options: &Options) -> CliOutcome {
    let query = match options.positionals.first() {
        Some(query) => query.clone(),
        None => return CliOutcome::err("error: search requires a query"),
    };
    let path = options.positionals.get(1).cloned().unwrap_or_else(|| ".".to_string());
    let index = match build_index(&path, options.include_text_files, &options.model) {
        Ok(index) => index,
        Err(err) => return CliOutcome::err(err.to_string()),
    };
    match index.search(&query, options.top_k, &options.mode, None, &[], &[]) {
        Ok(results) if results.is_empty() => CliOutcome::out("No results found."),
        Ok(results) => CliOutcome::out(format_results(
            &format!("Search results for: {query:?} (mode={})", options.mode),
            &results,
        )),
        Err(err) => CliOutcome::err(err.to_string()),
    }
}

fn run_find_related(options: &Options) -> CliOutcome {
    let file_path = match options.positionals.first() {
        Some(path) => path.clone(),
        None => return CliOutcome::err("error: find-related requires a file path"),
    };
    let line: usize = match options.positionals.get(1).and_then(|l| l.parse().ok()) {
        Some(line) => line,
        None => return CliOutcome::err("error: find-related requires a line number"),
    };
    let path = options.positionals.get(2).cloned().unwrap_or_else(|| ".".to_string());
    let index = match build_index(&path, options.include_text_files, &options.model) {
        Ok(index) => index,
        Err(err) => return CliOutcome::err(err.to_string()),
    };
    let chunk = match resolve_chunk(&index.chunks, &file_path, line) {
        Some(chunk) => chunk.clone(),
        None => return CliOutcome::err(format!("No chunk found at {file_path}:{line}.")),
    };
    match index.find_related(&chunk, options.top_k) {
        Ok(results) if results.is_empty() => {
            CliOutcome::out(format!("No related chunks found for {file_path}:{line}."))
        }
        Ok(results) => {
            CliOutcome::out(format_results(&format!("Chunks related to {file_path}:{line}"), &results))
        }
        Err(err) => CliOutcome::err(err.to_string()),
    }
}

/// Build an index from a local path or git URL using the selected model.
fn build_index(path: &str, include_text_files: bool, model: &str) -> Result<SemejaIndex> {
    let encoder = load_model(Some(model))?;
    if is_git_url(path) {
        SemejaIndex::from_git(path, None, Some(encoder), None, include_text_files)
    } else {
        SemejaIndex::from_path(Path::new(path), Some(encoder), None, include_text_files)
    }
}

// --- Argument parsing ---

struct Options {
    positionals: Vec<String>,
    top_k: usize,
    mode: String,
    model: String,
    include_text_files: bool,
    force: bool,
    verbose: bool,
}

impl Options {
    fn parse(args: &[String]) -> Options {
        let mut options = Options {
            positionals: Vec::new(),
            top_k: 5,
            mode: "hybrid".to_string(),
            // SEMEJA_MODEL overrides the default; "code" / "text" select presets.
            model: std::env::var("SEMEJA_MODEL").unwrap_or_else(|_| "code".to_string()),
            include_text_files: false,
            force: false,
            verbose: false,
        };
        let mut iter = args.iter().skip(2);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-k" | "--top-k" => {
                    if let Some(value) = iter.next().and_then(|v| v.parse().ok()) {
                        options.top_k = value;
                    }
                }
                "-m" | "--mode" => {
                    if let Some(value) = iter.next() {
                        options.mode = value.clone();
                    }
                }
                "--model" => {
                    if let Some(value) = iter.next() {
                        options.model = value.clone();
                    }
                }
                // The text-model shorthand also indexes documentation files,
                // since prose search is pointless with them excluded.
                "-t" | "--text" => {
                    options.model = "text".to_string();
                    options.include_text_files = true;
                }
                "--include-text-files" => options.include_text_files = true,
                "--force" => options.force = true,
                "--verbose" => options.verbose = true,
                other if other.starts_with('-') => {}
                other => options.positionals.push(other.to_string()),
            }
        }
        options
    }
}

/// Render the top-level help text.
fn help_text() -> String {
    [
        "semeja - fast and accurate code search for agents",
        "",
        "Usage:",
        "  semeja search <query> [path] [-k N] [-m MODE] [-t | --model NAME]",
        "  semeja find-related <file_path> <line> [path] [-k N] [-t | --model NAME]",
        "  semeja init [--force]",
        "  semeja savings [--verbose]",
        "",
        "Models: 'code' (default, for source), 'text' (for prose/docs), or any",
        "Hugging Face model2vec name via --model. Override with SEMEJA_MODEL.",
        "  -t  shorthand for the text model; also indexes documentation files.",
    ]
    .join("\n")
}

/// Entry point: route to the CLI or print the MCP placeholder message.
pub fn main_entry() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let is_cli = args.get(1).map(|a| CLI_COMMANDS.contains(&a.as_str())).unwrap_or(false);
    if is_cli {
        let outcome = run(&args);
        if !outcome.stdout.is_empty() {
            println!("{}", outcome.stdout);
        }
        if !outcome.stderr.is_empty() {
            eprintln!("{}", outcome.stderr);
        }
        outcome.exit_code
    } else {
        eprintln!("{}", help_text());
        1
    }
}
