//! Tests for `SemejaIndex` construction and search.

mod common;

use std::collections::HashSet;

use common::make_chunk;
use semeja::index::{create_index_from_path, MAX_FILE_BYTES};
use semeja::embed::MockEncoder;
use semeja::index::{compute_file_sizes, SemejaIndex};
use tempfile::{tempdir, TempDir};

fn model() -> Box<MockEncoder> {
    Box::new(MockEncoder::new())
}

/// A small project with two Python files and a README.
fn tmp_project() -> TempDir {
    let dir = tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("auth.py"),
        "def authenticate(token):\n    \"\"\"Verify an auth token.\"\"\"\n    return token == \"secret\"\n\ndef login(username, password):\n    return authenticate(password)\n",
    )
    .expect("write auth.py");
    std::fs::write(
        dir.path().join("utils.py"),
        "def format_name(first, last):\n    return f\"{first} {last}\"\n\nclass Config:\n    debug = False\n    host = \"localhost\"\n",
    )
    .expect("write utils.py");
    std::fs::write(dir.path().join("README.md"), "# Test project\n").expect("write README.md");
    dir
}

fn indexed() -> (TempDir, SemejaIndex) {
    let dir = tmp_project();
    let index = SemejaIndex::from_path(dir.path(), Some(model()), None, false).expect("index");
    (dir, index)
}

#[test]
fn markdown_is_excluded_by_default_and_included_on_request() {
    for (include_text_files, expect_md) in [(false, false), (true, true)] {
        let dir = tmp_project();
        let (_, _, chunks) =
            create_index_from_path(dir.path(), &MockEncoder::new(), None, include_text_files, None)
                .expect("index");
        let has_md = chunks.iter().any(|c| c.file_path.ends_with(".md"));
        assert_eq!(has_md, expect_md);
    }
}

#[test]
fn indexing_empty_directory_errors() {
    let dir = tempdir().expect("temp dir");
    assert!(create_index_from_path(dir.path(), &MockEncoder::new(), None, false, None).is_err());
}

#[test]
fn oversized_files_are_skipped() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("big.py"), vec![b'x'; (MAX_FILE_BYTES + 1) as usize])
        .expect("write big.py");
    assert!(create_index_from_path(dir.path(), &MockEncoder::new(), None, false, None).is_err());
}

#[test]
fn stats_report_language_counts() {
    let (_dir, index) = indexed();
    let stats = index.stats();
    assert!(stats.languages.get("python").copied().unwrap_or(0) > 0);
}

#[test]
fn each_search_mode_returns_bounded_results() {
    let (_dir, index) = indexed();
    for (query, mode) in
        [("authenticate token", "hybrid"), ("authenticate", "bm25"), ("authentication", "semantic")]
    {
        let results = index.search(query, 3, mode, None, &[], &[]).expect("search");
        assert!(results.len() <= 3);
    }
}

#[test]
fn unrecognised_search_mode_errors() {
    let (_dir, index) = indexed();
    assert!(index.search("query", 10, "invalid", None, &[], &[]).is_err());
}

#[test]
fn search_respects_top_k_and_returns_no_duplicates() {
    let (_dir, index) = indexed();
    assert!(index.search("function", 1, "bm25", None, &[], &[]).expect("search").len() <= 1);

    let results = index.search("authenticate", 5, "hybrid", None, &[], &[]).expect("search");
    let unique: HashSet<_> = results.iter().map(|r| &r.chunk).collect();
    assert_eq!(unique.len(), results.len());
}

#[test]
fn filtered_search_restricts_to_selected_paths() {
    let (_dir, index) = indexed();
    let target = index.chunks.last().expect("a chunk").file_path.clone();
    for mode in ["bm25", "hybrid", "semantic"] {
        let results =
            index.search("function", 3, mode, None, &[], &[target.clone()]).expect("search");
        assert!(results.iter().all(|r| r.chunk.file_path == target));
    }
}

#[test]
fn empty_query_returns_empty_across_modes() {
    let (_dir, index) = indexed();
    for mode in ["bm25", "hybrid", "semantic"] {
        for query in ["", "   ", "\n\n"] {
            assert!(index.search(query, 10, mode, None, &[], &[]).expect("search").is_empty());
        }
    }
}

#[test]
fn compute_file_sizes_dedups_and_skips_missing() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(dir.path().join("foo.py"), "hello world").expect("write foo.py");
    let chunks = vec![make_chunk("c", "foo.py"), make_chunk("c", "foo.py")];
    let sizes = compute_file_sizes(&chunks, dir.path());
    assert_eq!(sizes.get("foo.py").copied(), Some(11));

    let missing = compute_file_sizes(&[make_chunk("c", "nonexistent.py")], dir.path());
    assert!(missing.is_empty());
}

#[test]
fn from_path_rejects_missing_and_non_directory_paths() {
    let dir = tempdir().expect("temp dir");
    let missing = dir.path().join("does_not_exist");
    let err = SemejaIndex::from_path(&missing, Some(model()), None, false).err().expect("missing");
    assert!(err.to_string().contains("does not exist"));

    let file = dir.path().join("not_a_dir.py");
    std::fs::write(&file, "x = 1\n").expect("write file");
    let err = SemejaIndex::from_path(&file, Some(model()), None, false).err().expect("not a dir");
    assert!(err.to_string().contains("not a directory"));
}

#[test]
fn find_related_returns_related_chunks() {
    let (_dir, index) = indexed();
    let chunk = index.chunks[0].clone();
    let related = index.find_related(&chunk, 3).expect("find_related");
    assert!(related.len() <= 3);
    assert!(related.iter().all(|r| r.chunk != chunk));

    // find_related is deterministic for a given seed.
    let first: Vec<_> = index.find_related(&chunk, 3).expect("a").into_iter().map(|r| r.chunk).collect();
    let second: Vec<_> = index.find_related(&chunk, 3).expect("b").into_iter().map(|r| r.chunk).collect();
    assert_eq!(first, second);
}
