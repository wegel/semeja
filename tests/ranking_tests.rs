//! Tests for query-aware boosting, path penalties, and alpha weighting.

mod common;

use common::make_chunk;
use semeja::rank::{apply_query_boost, boost_multi_chunk_files, rerank_topk, resolve_alpha, ScoreMap};
use semeja::types::Chunk;

fn score_map(entries: &[(&Chunk, f32)]) -> ScoreMap {
    let mut map = ScoreMap::new();
    for (chunk, score) in entries {
        map.insert((*chunk).clone(), *score);
    }
    map
}

#[test]
fn rerank_topk_handles_empty_no_penalty_and_saturation() {
    assert!(rerank_topk(&ScoreMap::new(), 5, true).is_empty());

    let init_chunk = make_chunk("from .auth import authenticate", "src/semeja/__init__.py");
    let impl_chunk = make_chunk("def authenticate(token): ...", "src/semeja/auth.py");
    let ranked = rerank_topk(&score_map(&[(&init_chunk, 2.0), (&impl_chunk, 1.0)]), 2, false);
    assert_eq!(ranked[0].0, init_chunk);

    let saturated: Vec<Chunk> =
        (0..5).map(|i| make_chunk(&format!("def fn_{i}(): pass"), "big_file.py")).collect();
    let entries: Vec<(&Chunk, f32)> =
        saturated.iter().enumerate().map(|(i, c)| (c, (5 - i) as f32)).collect();
    let ranked = rerank_topk(&score_map(&entries), 5, true);
    let scores: Vec<f32> = ranked.iter().map(|(_, s)| *s).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.total_cmp(a));
    assert_eq!(scores, sorted);
}

#[test]
fn rerank_topk_demotes_penalised_paths() {
    for penalised_path in [
        "src/semeja/__init__.py",
        "tests/test_auth.py",
        "src/compat/old_api.py",
        "examples/demo.py",
        "src/types/index.d.ts",
    ] {
        let regular = make_chunk("def impl(): pass", "src/regular.py");
        let penalised = make_chunk("def impl(): pass", penalised_path);
        let ranked = rerank_topk(&score_map(&[(&regular, 1.0), (&penalised, 1.0)]), 2, true);
        assert_eq!(ranked[0].0, regular, "path {penalised_path} should rank below regular");
    }
}

#[test]
fn resolve_alpha_returns_explicit_or_auto_detected() {
    assert_eq!(resolve_alpha("MyService", Some(0.7)), 0.7);
    assert_eq!(resolve_alpha("MyService", None), 0.3);
    assert_eq!(resolve_alpha("how does routing work", None), 0.5);
}

#[test]
fn apply_query_boost_boosts_defining_chunk() {
    for query in ["MyService", "how does MyService work"] {
        let defining = make_chunk("class MyService:\n    pass", "src/my_service.py");
        let other = make_chunk("x = MyService()", "src/utils.py");
        let boosted = apply_query_boost(
            &score_map(&[(&defining, 0.5), (&other, 0.4)]),
            query,
            &[defining.clone(), other.clone()],
        );
        assert!(boosted[&defining] > boosted[&other], "query {query:?}");
    }
}

#[test]
fn apply_query_boost_scans_non_candidates() {
    for query in ["MyService", "how does MyService work"] {
        let defining = make_chunk("class MyService:\n    pass", "src/myservice.py");
        let candidate = make_chunk("x = 1", "src/other.py");
        let boosted = apply_query_boost(
            &score_map(&[(&candidate, 0.5)]),
            query,
            &[defining.clone(), candidate.clone()],
        );
        assert!(boosted.get(&defining).copied().unwrap_or(0.0) > 0.0, "query {query:?}");
    }
}

#[test]
fn apply_query_boost_skips_non_matching_stem() {
    for query in ["UserService", "how does UserService work"] {
        let defining = make_chunk("class UserService:\n    pass", "src/user_service.py");
        let unrelated = make_chunk("x = 1", "src/totally_unrelated_name.py");
        let boosted = apply_query_boost(
            &score_map(&[(&defining, 0.5)]),
            query,
            &[defining.clone(), unrelated.clone()],
        );
        assert!(!boosted.contains_key(&unrelated), "query {query:?}");
    }
}

#[test]
fn apply_query_boost_nl_stem_match_boosts() {
    for (query, file_path) in
        [("authenticate user session", "src/auth.py"), ("auth service", "src/auth_service.py")]
    {
        let chunk = make_chunk("def authenticate(): pass", file_path);
        let boosted = apply_query_boost(&score_map(&[(&chunk, 0.5)]), query, &[chunk.clone()]);
        assert!(boosted[&chunk] > 0.5, "query {query:?}");
    }
}

#[test]
fn apply_query_boost_edge_cases() {
    let chunk = make_chunk("def foo(): pass", "src/auth.py");
    let stopwords = apply_query_boost(&score_map(&[(&chunk, 0.5)]), "the and or", &[chunk.clone()]);
    assert!((stopwords[&chunk] - 0.5).abs() < 1e-6);

    let defining = make_chunk("class Base:\n    pass", "src/base.py");
    let qualified =
        apply_query_boost(&score_map(&[(&defining, 0.5)]), "Sinatra::Base", &[defining.clone()]);
    assert!(qualified[&defining] > 0.5);

    assert!(apply_query_boost(&ScoreMap::new(), "SomeQuery", &[]).is_empty());
}

#[test]
fn boost_multi_chunk_files_promotes_top_chunk() {
    let mut empty = ScoreMap::new();
    boost_multi_chunk_files(&mut empty);
    assert!(empty.is_empty());

    let zero_chunk = make_chunk("x = 1", "src/foo.py");
    let mut all_zero = score_map(&[(&zero_chunk, 0.0)]);
    boost_multi_chunk_files(&mut all_zero);
    assert_eq!(all_zero[&zero_chunk], 0.0);

    let c1 = make_chunk("def a(): pass", "src/big.py");
    let c2 = make_chunk("def b(): pass", "src/big.py");
    let c3 = make_chunk("def c(): pass", "src/small.py");
    let mut scores = score_map(&[(&c1, 1.0), (&c2, 0.8), (&c3, 1.0)]);
    boost_multi_chunk_files(&mut scores);
    assert!(scores[&c1] > 1.0);
}

#[test]
fn apply_query_boost_with_empty_scores() {
    assert!(apply_query_boost(&ScoreMap::new(), "query", &[]).is_empty());
}
