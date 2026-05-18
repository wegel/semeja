//! In-process MCP-style server: cached indexes behind `search`/`find_related` tools.

use std::path::Path;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::index::SemejaIndex;
use crate::types::{Chunk, SearchResult};
use crate::utils::{format_results, is_git_url, resolve_chunk};

/// Maximum number of cached indexes kept in memory.
pub const CACHE_MAX_SIZE: usize = 10;

// --- Index abstraction ---

/// The index operations the server needs; implemented by [`SemejaIndex`].
pub trait Index {
    /// Search the index with the given query and mode.
    fn search(&self, query: &str, top_k: usize, mode: &str) -> Result<Vec<SearchResult>>;
    /// Return chunks similar to the given seed chunk.
    fn find_related(&self, chunk: &Chunk, top_k: usize) -> Result<Vec<SearchResult>>;
    /// All indexed chunks.
    fn chunks(&self) -> &[Chunk];
}

impl Index for SemejaIndex {
    fn search(&self, query: &str, top_k: usize, mode: &str) -> Result<Vec<SearchResult>> {
        SemejaIndex::search(self, query, top_k, mode, None, &[], &[])
    }

    fn find_related(&self, chunk: &Chunk, top_k: usize) -> Result<Vec<SearchResult>> {
        SemejaIndex::find_related(self, chunk, top_k)
    }

    fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
}

/// Build the default [`SemejaIndex`] for a source string.
pub fn default_builder(source: &str, git_ref: Option<&str>, include_text: bool) -> Result<SemejaIndex> {
    if is_git_url(source) {
        SemejaIndex::from_git(source, git_ref, None, None, include_text)
    } else {
        SemejaIndex::from_path(Path::new(source), None, None, include_text)
    }
}

// --- Index cache ---

type Builder<I> = Box<dyn Fn(&str, Option<&str>, bool) -> Result<I>>;

/// An LRU cache of indexed repositories and local paths.
pub struct IndexCache<I> {
    builder: Builder<I>,
    include_text_files: bool,
    entries: Vec<(String, Rc<I>)>,
}

impl<I> IndexCache<I> {
    /// Create a cache backed by the given index builder.
    pub fn new(builder: Builder<I>, include_text_files: bool) -> IndexCache<I> {
        IndexCache { builder, include_text_files, entries: Vec::new() }
    }

    /// Number of currently cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when an entry exists for the given resolved cache key.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Remove the cache entry for a source, if present.
    pub fn evict(&mut self, source: &str) {
        let key = compute_cache_key(source, None);
        self.entries.retain(|(k, _)| *k != key);
    }

    /// Return an index for the source, building and caching it on first access.
    pub fn get(&mut self, source: &str, git_ref: Option<&str>) -> Result<Rc<I>> {
        let key = compute_cache_key(source, git_ref);
        if let Some(pos) = self.entries.iter().position(|(k, _)| *k == key) {
            let entry = self.entries.remove(pos);
            let index = entry.1.clone();
            self.entries.push(entry);
            return Ok(index);
        }
        if self.entries.len() >= CACHE_MAX_SIZE {
            self.entries.remove(0);
        }
        let index = Rc::new((self.builder)(source, git_ref, self.include_text_files)?);
        self.entries.push((key, index.clone()));
        Ok(index)
    }
}

/// Compute the canonical cache key for a source.
pub fn compute_cache_key(source: &str, git_ref: Option<&str>) -> String {
    if is_git_url(source) {
        match git_ref {
            Some(reference) => format!("{source}@{reference}"),
            None => source.to_string(),
        }
    } else {
        Path::new(source)
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| source.to_string())
    }
}

// --- Server ---

/// An MCP-style server exposing `search` and `find_related` tools.
pub struct Server<I> {
    cache: IndexCache<I>,
    default_source: Option<String>,
}

impl<I: Index> Server<I> {
    /// Build a server backed by the given cache and optional default source.
    pub fn new(cache: IndexCache<I>, default_source: Option<String>) -> Server<I> {
        Server { cache, default_source }
    }

    /// Invoke a tool by name with JSON arguments, returning its text output.
    pub fn call_tool(&mut self, tool: &str, args: &Value) -> String {
        match tool {
            "search" => self.search_tool(args),
            "find_related" => self.find_related_tool(args),
            other => format!("Unknown tool: {other}"),
        }
    }

    fn search_tool(&mut self, args: &Value) -> String {
        let query = str_arg(args, "query").unwrap_or_default();
        let mode = str_arg(args, "mode").unwrap_or_else(|| "hybrid".to_string());
        let top_k = int_arg(args, "top_k").unwrap_or(5);
        let index = match self.resolve_index(args) {
            Ok(index) => index,
            Err(message) => return message,
        };
        match index.search(&query, top_k, &mode) {
            Ok(results) if results.is_empty() => "No results found.".to_string(),
            Ok(results) => format_results(&format!("Search results for: {query:?} (mode={mode})"), &results),
            Err(err) => format!("Search failed: {err}"),
        }
    }

    fn find_related_tool(&mut self, args: &Value) -> String {
        let file_path = str_arg(args, "file_path").unwrap_or_default();
        let line = int_arg(args, "line").unwrap_or(0);
        let top_k = int_arg(args, "top_k").unwrap_or(5);
        let index = match self.resolve_index(args) {
            Ok(index) => index,
            Err(message) => return message,
        };
        let chunk = match resolve_chunk(index.chunks(), &file_path, line) {
            Some(chunk) => chunk.clone(),
            None => {
                return format!(
                    "No chunk found at {file_path}:{line}. \
                     Make sure the file is indexed and the line number is within a known chunk."
                )
            }
        };
        match index.find_related(&chunk, top_k) {
            Ok(results) if results.is_empty() => {
                format!("No related chunks found for {file_path}:{line}.")
            }
            Ok(results) => format_results(&format!("Chunks related to {file_path}:{line}"), &results),
            Err(err) => format!("find_related failed: {err}"),
        }
    }

    /// Resolve and cache the index for a tool call, or return an error message.
    fn resolve_index(&mut self, args: &Value) -> Result<Rc<I>, String> {
        let repo = str_arg(args, "repo");
        get_index(repo.as_deref(), self.default_source.as_deref(), &mut self.cache)
    }
}

/// Return a cached index for a repo, rejecting unsafe git transport schemes.
pub fn get_index<I>(
    repo: Option<&str>,
    default_source: Option<&str>,
    cache: &mut IndexCache<I>,
) -> Result<Rc<I>, String> {
    if let Some(repo) = repo {
        if is_git_url(repo) && !(repo.starts_with("https://") || repo.starts_with("http://")) {
            return Err(format!(
                "Only https://, http://, or local directory paths are accepted as `repo`. Got: {repo:?}"
            ));
        }
    }
    let source = repo.or(default_source).ok_or_else(|| {
        "No repo specified and no default index. \
         Pass an https:// or http:// git URL or local directory path as `repo`."
            .to_string()
    })?;
    cache.get(source, None).map_err(|err| format!("Failed to index {source:?}: {err}"))
}

// --- Argument helpers ---

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn int_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

/// Build a server that indexes real repositories on demand.
pub fn create_server(
    default_source: Option<String>,
    include_text_files: bool,
) -> Server<SemejaIndex> {
    let cache = IndexCache::new(Box::new(default_builder), include_text_files);
    Server::new(cache, default_source)
}

/// Placeholder entry point for stdio serving (no protocol transport bundled).
pub fn serve(_path: Option<&str>) -> Result<()> {
    Err(anyhow!("The semeja MCP stdio transport is not bundled in this build."))
}
