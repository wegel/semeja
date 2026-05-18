//! Tests for the in-process MCP-style server and index cache.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use anyhow::anyhow;
use common::make_chunk;
use semeja::mcp::{compute_cache_key, get_index, Index, IndexCache, Server, CACHE_MAX_SIZE};
use semeja::types::{Chunk, SearchMode, SearchResult};
use semeja::utils::{format_results, is_git_url, resolve_chunk};
use serde_json::json;
use tempfile::tempdir;

// --- Fake index ---

#[derive(Clone, Debug, Default)]
struct FakeIndex {
    search_results: Vec<SearchResult>,
    related_results: Vec<SearchResult>,
    chunks: Vec<Chunk>,
}

impl Index for FakeIndex {
    fn search(&self, _query: &str, _top_k: usize, _mode: &str) -> anyhow::Result<Vec<SearchResult>> {
        Ok(self.search_results.clone())
    }

    fn find_related(&self, _chunk: &Chunk, _top_k: usize) -> anyhow::Result<Vec<SearchResult>> {
        Ok(self.related_results.clone())
    }

    fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }
}

fn counting_cache(index: FakeIndex) -> (IndexCache<FakeIndex>, Rc<Cell<usize>>) {
    let calls = Rc::new(Cell::new(0));
    let calls_clone = calls.clone();
    let cache = IndexCache::new(
        Box::new(move |_, _, _| {
            calls_clone.set(calls_clone.get() + 1);
            Ok(index.clone())
        }),
        false,
    );
    (cache, calls)
}

// --- Utility tests ---

#[test]
fn resolve_chunk_handles_interior_boundary_and_misses() {
    let interior = make_chunk("line1\nline2\nline3", "src/a.py");
    let boundary = make_chunk("last line", "src/a.py");

    assert_eq!(resolve_chunk(std::slice::from_ref(&interior), "src/a.py", 2), Some(&interior));
    assert_eq!(resolve_chunk(std::slice::from_ref(&boundary), "src/a.py", 1), Some(&boundary));
    assert_eq!(resolve_chunk(std::slice::from_ref(&interior), "src/other.py", 1), None);
    assert_eq!(resolve_chunk(std::slice::from_ref(&interior), "src/a.py", 99), None);
}

#[test]
fn is_git_url_detects_remote_urls() {
    for (path, expected) in [
        ("https://github.com/org/repo", true),
        ("http://github.com/org/repo", true),
        ("git://github.com/org/repo", true),
        ("ssh://git@github.com/org/repo", true),
        ("git+ssh://git@github.com/org/repo", true),
        ("file:///tmp/repo", true),
        ("git@github.com:org/repo", true),
        ("/local/path/to/repo", false),
        ("./relative/path", false),
        ("repo_name", false),
    ] {
        assert_eq!(is_git_url(path), expected, "path {path}");
    }
}

#[test]
fn format_results_renders_header_and_fenced_blocks() {
    assert!(format_results("My header", &[]).contains("My header"));
    assert!(!format_results("My header", &[]).contains("```"));

    let results: Vec<SearchResult> = (0..3)
        .map(|i| SearchResult {
            chunk: make_chunk(&format!("def fn_{i}(): pass"), &format!("f{i}.py")),
            score: 0.1 * (i + 1) as f32,
            source: SearchMode::Hybrid,
        })
        .collect();
    let out = format_results("Results for: 'foo'", &results);
    assert!(out.contains("Results for: 'foo'"));
    assert!(out.matches("```").count() >= results.len() * 2);
    for (i, result) in results.iter().enumerate() {
        assert!(out.contains(&format!("## {}.", i + 1)));
        assert!(out.contains(&result.chunk.content));
    }
    assert!(out.contains("0.100") && out.contains("0.200") && out.contains("0.300"));
}

// --- Cache tests ---

#[test]
fn index_cache_builds_and_caches() {
    let dir = tempdir().expect("temp dir");
    for source in [dir.path().to_str().unwrap(), "https://github.com/org/repo"] {
        let (mut cache, calls) = counting_cache(FakeIndex::default());
        let first = cache.get(source, None).expect("first build");
        let second = cache.get(source, None).expect("cached build");
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn index_cache_evicts_on_failure() {
    let calls = Rc::new(Cell::new(0));
    let calls_clone = calls.clone();
    let mut cache: IndexCache<FakeIndex> = IndexCache::new(
        Box::new(move |_, _, _| {
            calls_clone.set(calls_clone.get() + 1);
            if calls_clone.get() == 1 {
                Err(anyhow!("build failed"))
            } else {
                Ok(FakeIndex::default())
            }
        }),
        false,
    );
    let dir = tempdir().expect("temp dir");
    let source = dir.path().to_str().unwrap();
    assert!(cache.get(source, None).is_err());
    assert!(cache.get(source, None).is_ok());
    assert_eq!(calls.get(), 2);
}

#[test]
fn index_cache_evicts_least_recently_used() {
    let parent = tempdir().expect("temp dir");
    let dirs: Vec<_> = (0..=CACHE_MAX_SIZE)
        .map(|i| {
            let path = parent.path().join(i.to_string());
            std::fs::create_dir(&path).expect("create dir");
            path
        })
        .collect();
    let (mut cache, _) = counting_cache(FakeIndex::default());

    for dir in &dirs[..CACHE_MAX_SIZE] {
        cache.get(dir.to_str().unwrap(), None).expect("build");
    }
    let first_key = compute_cache_key(dirs[0].to_str().unwrap(), None);
    assert!(cache.contains_key(&first_key));

    cache.get(dirs[CACHE_MAX_SIZE].to_str().unwrap(), None).expect("build");
    assert!(!cache.contains_key(&first_key));
    assert_eq!(cache.len(), CACHE_MAX_SIZE);
}

#[test]
fn cache_evict_removes_entry() {
    let dir = tempdir().expect("temp dir");
    let (mut cache, _) = counting_cache(FakeIndex::default());
    cache.get(dir.path().to_str().unwrap(), None).expect("build");
    let key = compute_cache_key(dir.path().to_str().unwrap(), None);
    assert!(cache.contains_key(&key));
    cache.evict(dir.path().to_str().unwrap());
    assert!(!cache.contains_key(&key));
}

#[test]
fn cache_evict_missing_is_noop() {
    let (mut cache, _) = counting_cache(FakeIndex::default());
    cache.evict("/no/such/path");
}

#[test]
fn get_index_requires_a_source() {
    let (mut cache, _) = counting_cache(FakeIndex::default());
    let err = get_index(None, None, &mut cache).expect_err("no source");
    assert!(err.contains("No repo specified"));
}

// --- Tool tests ---

fn fake_server(index: FakeIndex, default: Option<&str>) -> Server<FakeIndex> {
    let (cache, _) = counting_cache(index);
    Server::new(cache, default.map(str::to_string))
}

#[test]
fn tools_report_missing_repo_and_default() {
    let mut server = fake_server(FakeIndex::default(), None);
    assert!(server.call_tool("search", &json!({"query": "foo"})).contains("No repo specified"));
    assert!(server
        .call_tool("find_related", &json!({"file_path": "src/foo.py", "line": 10}))
        .contains("No repo specified"));
}

#[test]
fn tools_report_index_failure() {
    let mut cache: IndexCache<FakeIndex> =
        IndexCache::new(Box::new(|_, _, _| Err(anyhow!("clone failed"))), false);
    let _ = &mut cache;
    let mut server = Server::new(cache, None);
    let text = server.call_tool("search", &json!({"query": "foo", "repo": "https://github.com/x/y"}));
    assert!(text.contains("Failed to index"));
    assert!(text.contains("clone failed"));
}

#[test]
fn tools_reject_unsafe_repo_schemes() {
    for repo in ["file:///home/user/secret", "ssh://internal-host/repo", "git@github.com:org/repo"] {
        let mut server = fake_server(FakeIndex::default(), None);
        let text = server.call_tool("search", &json!({"query": "foo", "repo": repo}));
        assert!(text.contains("Only https://"), "repo {repo}");
    }
}

#[test]
fn search_tool_formats_results_and_empty_state() {
    let chunk = make_chunk("def bar(): pass", "src/bar.py");
    let with_results = FakeIndex {
        search_results: vec![SearchResult { chunk, score: 0.9, source: SearchMode::Hybrid }],
        ..FakeIndex::default()
    };
    let mut server = fake_server(with_results, Some("/some/path"));
    let text = server.call_tool("search", &json!({"query": "bar"}));
    assert!(text.contains("bar") && text.contains("0.900"));

    let mut empty = fake_server(FakeIndex::default(), Some("/some/path"));
    assert!(empty.call_tool("search", &json!({"query": "nothing"})).contains("No results found"));
}

#[test]
fn find_related_tool_formats_results_and_states() {
    let chunk = make_chunk("class Foo: pass", "src/foo.py");
    let with_results = FakeIndex {
        related_results: vec![SearchResult {
            chunk: chunk.clone(),
            score: 0.8,
            source: SearchMode::Semantic,
        }],
        chunks: vec![chunk.clone()],
        ..FakeIndex::default()
    };
    let mut server = fake_server(with_results, Some("/some/path"));
    let text = server.call_tool("find_related", &json!({"file_path": "src/foo.py", "line": 1}));
    assert!(text.contains("src/foo.py:1") && text.contains("0.800"));

    let no_results =
        FakeIndex { chunks: vec![chunk], ..FakeIndex::default() };
    let mut server = fake_server(no_results, Some("/some/path"));
    let text = server.call_tool("find_related", &json!({"file_path": "src/foo.py", "line": 1}));
    assert!(text.contains("No related chunks found"));

    let mut unknown = fake_server(FakeIndex::default(), Some("/some/path"));
    let text = unknown.call_tool("find_related", &json!({"file_path": "src/unknown.py", "line": 1}));
    assert!(text.contains("No chunk found"));
}
