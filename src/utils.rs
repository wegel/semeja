//! Shared helpers for git-URL detection, chunk resolution, and result formatting.

use std::sync::LazyLock;

use regex::Regex;

use crate::types::{Chunk, SearchResult};

const GIT_URL_SCHEMES: [&str; 6] =
    ["https://", "http://", "ssh://", "git://", "git+ssh://", "file://"];

static SCP_GIT_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\w.-]+@[\w.-]+:").expect("valid scp url regex"));

/// Return true if `path` looks like a remote git URL rather than a local path.
pub fn is_git_url(path: &str) -> bool {
    if GIT_URL_SCHEMES.iter().any(|scheme| path.starts_with(scheme)) {
        return true;
    }
    // SCP-like form `user@host:path`, where the char after `:` is not `/`.
    match SCP_GIT_URL_RE.find(path) {
        Some(m) => path[m.end()..].chars().next() != Some('/'),
        None => false,
    }
}

/// Return the chunk containing `line` in `file_path`, or `None`.
///
/// A line strictly inside a chunk wins immediately; a line on a chunk's
/// last line is kept only as a fallback for end-of-file chunks.
pub fn resolve_chunk<'a>(chunks: &'a [Chunk], file_path: &str, line: usize) -> Option<&'a Chunk> {
    let mut fallback: Option<&Chunk> = None;
    for chunk in chunks {
        if chunk.file_path == file_path && chunk.start_line <= line && line <= chunk.end_line {
            if line < chunk.end_line {
                return Some(chunk);
            }
            if fallback.is_none() {
                fallback = Some(chunk);
            }
        }
    }
    fallback
}

/// Render search results as numbered, fenced code blocks under a header.
pub fn format_results(header: &str, results: &[SearchResult]) -> String {
    let mut lines: Vec<String> = vec![header.to_string(), String::new()];
    for (i, r) in results.iter().enumerate() {
        lines.push(format!("## {}. {}  [score={:.3}]", i + 1, r.chunk.location(), r.score));
        lines.push("```".to_string());
        lines.push(r.chunk.content.trim().to_string());
        lines.push("```".to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}
