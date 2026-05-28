//! Tests for semantic, BM25, and hybrid search.

mod common;

use common::make_chunk;
use semeja::embed::{
    embed_chunks, load_model, resolve_model_name, CosineBackend, MockEncoder, CODE_MODEL_NAME,
    DEFAULT_MODEL_NAME, TEXT_MODEL_NAME,
};
use semeja::bm25::Bm25Index;
use semeja::search::{search_bm25, search_hybrid, search_semantic, sort_top_k};
use semeja::tokenize::tokenize;
use semeja::types::{Chunk, SearchMode};

fn chunks() -> Vec<Chunk> {
    vec![
        make_chunk("def authenticate(token):\n    return token == 'secret'", "auth.py"),
        make_chunk("def login(username, password):\n    pass", "auth.py"),
        make_chunk("class UserService:\n    pass", "users.py"),
        make_chunk("def format_date(dt):\n    return str(dt)", "utils.py"),
    ]
}

fn bm25_index(chunks: &[Chunk]) -> Bm25Index {
    let corpus: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(&c.content)).collect();
    Bm25Index::build(&corpus)
}

fn semantic_index(chunks: &[Chunk]) -> CosineBackend {
    CosineBackend::new(embed_chunks(&MockEncoder::new(), chunks))
}

#[test]
fn search_bm25_ranks_relevant_first_and_honours_selector() {
    let chunks = chunks();
    let bm25 = bm25_index(&chunks);

    let results = search_bm25("authenticate token", &bm25, &chunks, 4, None);
    assert!(!results.is_empty());
    assert!(results[0].chunk.content.contains("authenticate"));

    let selector = [chunks.len() - 1];
    let filtered = search_bm25("format", &bm25, &chunks, 4, Some(&selector));
    assert!(filtered.iter().all(|r| r.chunk == chunks[chunks.len() - 1]));
}

#[test]
fn search_bm25_returns_empty_for_no_match() {
    let chunks = chunks();
    let bm25 = bm25_index(&chunks);
    for query in ["", "   ", "\n\n", "zzzznonexistentterm"] {
        assert!(search_bm25(query, &bm25, &chunks, 3, None).is_empty(), "query {query:?}");
    }
}

#[test]
fn semantic_search_returns_scores_in_range() {
    let chunks = chunks();
    let semantic = semantic_index(&chunks);
    let model = MockEncoder::new();
    let results = search_semantic("login", &model, &semantic, &chunks, 3, None).expect("search");
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| (-1.0..=1.0).contains(&r.score)));
}

#[test]
fn search_hybrid_combines_and_keeps_distinct_files() {
    let chunks = chunks();
    let model = MockEncoder::new();
    let results = search_hybrid(
        "authenticate token",
        &model,
        &semantic_index(&chunks),
        &bm25_index(&chunks),
        &chunks,
        3,
        None,
        None,
    )
    .expect("search");
    assert!(!results.is_empty());

    let shared = "def helper():\n    pass";
    let all_chunks = vec![make_chunk(shared, "module_a.py"), make_chunk(shared, "module_b.py")];
    let deduped = search_hybrid(
        "helper",
        &model,
        &semantic_index(&all_chunks),
        &bm25_index(&all_chunks),
        &all_chunks,
        5,
        None,
        None,
    )
    .expect("search");
    let locations: Vec<&str> = deduped.iter().map(|r| r.chunk.file_path.as_str()).collect();
    assert!(locations.contains(&"module_a.py"));
    assert!(locations.contains(&"module_b.py"));
}

#[test]
fn search_results_carry_matching_source_labels() {
    let chunks = chunks();
    let model = MockEncoder::new();
    let semantic = semantic_index(&chunks);
    let bm25 = bm25_index(&chunks);

    let bm25_results = search_bm25("authenticate", &bm25, &chunks, 3, None);
    assert!(!bm25_results.is_empty());
    assert!(bm25_results.iter().all(|r| r.source == SearchMode::Bm25));

    let semantic_results =
        search_semantic("query", &model, &semantic, &chunks, 4, None).expect("search");
    assert!(!semantic_results.is_empty());
    assert!(semantic_results.iter().all(|r| r.source == SearchMode::Semantic));

    let hybrid_results =
        search_hybrid("login", &model, &semantic, &bm25, &chunks, 4, None, None).expect("search");
    assert!(!hybrid_results.is_empty());
    assert!(hybrid_results.iter().all(|r| r.source == SearchMode::Hybrid));
}

#[test]
fn sort_top_k_matches_descending_argsort() {
    let values: Vec<f32> = (0..1000).map(|i| ((i * 7919) % 1000) as f32 * 0.5 - 200.0).collect();
    let top_k = 100;
    let got = sort_top_k(&values, top_k);

    let mut expected: Vec<usize> = (0..values.len()).collect();
    expected.sort_by(|&a, &b| values[b].total_cmp(&values[a]));
    expected.truncate(top_k);
    assert_eq!(got, expected);
}

#[test]
fn model_selectors_resolve_to_preset_names() {
    assert_eq!(DEFAULT_MODEL_NAME, "minishlab/potion-code-16M");
    assert_eq!(CODE_MODEL_NAME, "minishlab/potion-code-16M");
    assert_eq!(TEXT_MODEL_NAME, "minishlab/potion-retrieval-32M");
    assert_eq!(resolve_model_name(None), CODE_MODEL_NAME);
    assert_eq!(resolve_model_name(Some("code")), CODE_MODEL_NAME);
    assert_eq!(resolve_model_name(Some("text")), TEXT_MODEL_NAME);
    assert_eq!(resolve_model_name(Some("org/custom-model")), "org/custom-model");
}

/// Verifies the real model loads and embeds; ignored by default as it
/// downloads the model from the Hugging Face hub on first run.
#[test]
#[ignore = "downloads the embedding model from the network"]
fn load_model_produces_real_embeddings() {
    let model = load_model(None).expect("load default model");
    let embeddings = model.encode(&["def authenticate(token): pass".to_string()]);
    assert_eq!(embeddings.len(), 1);
    assert!(!embeddings[0].is_empty());
}

#[test]
fn embed_chunks_empty_returns_empty_matrix() {
    let result = embed_chunks(&MockEncoder::new(), &[]);
    assert!(result.is_empty());
}

#[test]
fn cosine_backend_rejects_k_below_one() {
    let chunks = chunks();
    let embeddings = embed_chunks(&MockEncoder::new(), &chunks);
    let backend = CosineBackend::new(embeddings.clone());
    let err = backend.query(&embeddings[..1], 0, None).expect_err("k=0 must error");
    assert!(err.to_string().contains("k should be >= 1"));
}
