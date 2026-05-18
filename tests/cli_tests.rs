//! Tests for the command-line interface.

use semeja::cli::{claude_file_path, run, run_init, AGENT_FILE};
use tempfile::{tempdir, TempDir};

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// A temporary project with one indexable Python file.
fn tmp_project() -> TempDir {
    let dir = tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("auth.py"),
        "def authenticate(token):\n    return token == 'secret'\n\ndef login(user):\n    return authenticate(user)\n",
    )
    .expect("write auth.py");
    dir
}

#[test]
fn init_creates_agent_file() {
    let dir = tempdir().expect("temp dir");
    let message = run_init(dir.path(), false).expect("init");
    let dest = dir.path().join(claude_file_path());
    assert!(dest.exists());
    assert_eq!(std::fs::read_to_string(&dest).expect("read"), AGENT_FILE);
    assert!(message.contains(&claude_file_path().display().to_string()));
}

#[test]
fn init_refuses_overwrite_without_force() {
    let dir = tempdir().expect("temp dir");
    run_init(dir.path(), false).expect("first init");
    let err = run_init(dir.path(), false).expect_err("second init");
    assert!(err.contains("already exists"));
}

#[test]
fn init_overwrites_with_force() {
    let dir = tempdir().expect("temp dir");
    let dest = dir.path().join(claude_file_path());
    std::fs::create_dir_all(dest.parent().unwrap()).expect("mkdir");
    std::fs::write(&dest, "old content").expect("write old");
    run_init(dir.path(), true).expect("forced init");
    assert_eq!(std::fs::read_to_string(&dest).expect("read"), AGENT_FILE);
}

#[test]
fn agent_file_tools_are_bash_and_read_only() {
    let frontmatter = AGENT_FILE.split("---").nth(1).expect("frontmatter");
    let tools_line = frontmatter.lines().find(|l| l.starts_with("tools:")).expect("tools line");
    let tools: Vec<&str> =
        tools_line.trim_start_matches("tools:").split(',').map(str::trim).collect();
    assert_eq!(tools.len(), 2);
    assert!(tools.contains(&"Bash") && tools.contains(&"Read"));
    assert!(!tools.iter().any(|t| t.contains("mcp__")));
}

#[test]
fn cli_search_prints_results() {
    let dir = tmp_project();
    let outcome = run(&args(&["semeja", "search", "authenticate", dir.path().to_str().unwrap()]));
    assert_eq!(outcome.exit_code, 0);
    assert!(outcome.stdout.contains("Search results for"));
    assert!(outcome.stdout.contains("authenticate"));
}

#[test]
fn cli_search_reports_no_results() {
    let dir = tmp_project();
    let outcome = run(&args(&[
        "semeja",
        "search",
        "zzzznonexistentterm",
        dir.path().to_str().unwrap(),
        "--mode",
        "bm25",
    ]));
    assert_eq!(outcome.exit_code, 0);
    assert!(outcome.stdout.contains("No results found"));
}

#[test]
fn cli_find_related_handles_unknown_and_known_chunks() {
    let dir = tmp_project();
    let path = dir.path().to_str().unwrap();

    let unknown = run(&args(&["semeja", "find-related", "unknown.py", "1", path]));
    assert_eq!(unknown.exit_code, 1);
    assert!(unknown.stderr.contains("No chunk found"));

    let known = run(&args(&["semeja", "find-related", "auth.py", "2", path]));
    assert_eq!(known.exit_code, 0);
}

#[test]
fn cli_help_lists_subcommands() {
    let outcome = run(&args(&["semeja", "--help"]));
    assert_eq!(outcome.exit_code, 0);
    assert!(outcome.stdout.contains("find-related"));
}

#[test]
fn cli_savings_runs_without_panicking() {
    let outcome = run(&args(&["semeja", "savings"]));
    assert_eq!(outcome.exit_code, 0);
    assert!(!outcome.stdout.is_empty());
}
