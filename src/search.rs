//! Semantic, BM25, and hybrid search over an indexed corpus.

use anyhow::Result;

use crate::embed::{CosineBackend, Encoder};
use crate::bm25::{selector_to_mask, Bm25Index};
use crate::rank::{apply_query_boost, boost_multi_chunk_files, rerank_topk, resolve_alpha, ScoreMap};
use crate::tokenize::tokenize;
use crate::types::{Chunk, SearchMode, SearchResult};

/// Reciprocal-rank-fusion constant.
const RRF_K: f32 = 60.0;

/// Run semantic (vector) search for a query.
pub fn search_semantic(
    query: &str,
    model: &dyn Encoder,
    semantic_index: &CosineBackend,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
) -> Result<Vec<SearchResult>> {
    let query_embedding = model.encode(&[query.to_string()]);
    let mut results = semantic_index.query(&query_embedding, top_k, selector)?;
    let (indices, distances) = results.drain(..).next().unwrap_or_default();
    Ok(indices
        .into_iter()
        .zip(distances)
        .map(|(index, distance)| SearchResult {
            chunk: chunks[index].clone(),
            // Vicinity-style cosine distance; convert so higher = better.
            score: 1.0 - distance,
            source: SearchMode::Semantic,
        })
        .collect())
}

/// Return chunks ranked by BM25 score, excluding zero-score results.
pub fn search_bm25(
    query: &str,
    bm25_index: &Bm25Index,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[usize]>,
) -> Vec<SearchResult> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mask = selector_to_mask(selector, chunks.len());
    let scores = bm25_index.get_scores(&tokens, mask.as_deref());

    sort_top_k(&scores, top_k)
        .into_iter()
        .filter(|&i| scores[i] > 0.0)
        .map(|i| SearchResult { chunk: chunks[i].clone(), score: scores[i], source: SearchMode::Bm25 })
        .collect()
}

/// Hybrid search: an alpha-weighted blend of semantic and BM25 rankings.
///
/// Both score sets are converted to reciprocal-rank-fusion scores before
/// combining, so `alpha` has a consistent meaning regardless of magnitude.
pub fn search_hybrid(
    query: &str,
    model: &dyn Encoder,
    semantic_index: &CosineBackend,
    bm25_index: &Bm25Index,
    chunks: &[Chunk],
    top_k: usize,
    alpha: Option<f32>,
    selector: Option<&[usize]>,
    max_per_file: usize,
) -> Result<Vec<SearchResult>> {
    let alpha_weight = resolve_alpha(query, alpha);

    // Over-fetch candidates so the merged pool stays large after the union.
    let candidate_count = top_k * 5;

    let semantic = search_semantic(query, model, semantic_index, chunks, candidate_count, selector)?;
    let mut semantic_scores: ScoreMap = ScoreMap::new();
    for result in semantic {
        semantic_scores.insert(result.chunk, result.score);
    }
    let mut bm25_scores: ScoreMap = ScoreMap::new();
    for result in search_bm25(query, bm25_index, chunks, candidate_count, selector) {
        if result.score != 0.0 {
            bm25_scores.insert(result.chunk, result.score);
        }
    }

    let normalized_semantic = rrf_scores(&semantic_scores);
    let normalized_bm25 = rrf_scores(&bm25_scores);

    // Union the candidate sets, then sort by start line for stable ordering.
    let mut all_candidates: Vec<Chunk> = Vec::new();
    for chunk in normalized_semantic.keys().chain(normalized_bm25.keys()) {
        if !all_candidates.contains(chunk) {
            all_candidates.push(chunk.clone());
        }
    }
    all_candidates.sort_by_key(|c| c.start_line);

    let mut combined: ScoreMap = ScoreMap::new();
    for chunk in all_candidates {
        let semantic_part = alpha_weight * normalized_semantic.get(&chunk).copied().unwrap_or(0.0);
        let bm25_part = (1.0 - alpha_weight) * normalized_bm25.get(&chunk).copied().unwrap_or(0.0);
        combined.insert(chunk, semantic_part + bm25_part);
    }

    // Boost multi-chunk files, then query-type boosts, then rerank with penalties.
    boost_multi_chunk_files(&mut combined);
    let combined = apply_query_boost(&combined, query, chunks);
    let ranked = rerank_topk(&combined, top_k, alpha_weight < 1.0, max_per_file);

    Ok(ranked
        .into_iter()
        .map(|(chunk, score)| SearchResult { chunk, score, source: SearchMode::Hybrid })
        .collect())
}

// --- Private helpers ---

/// Convert raw scores to RRF scores `1/(k + rank)`; higher raw score → rank 1.
fn rrf_scores(scores: &ScoreMap) -> ScoreMap {
    if scores.is_empty() {
        return ScoreMap::new();
    }
    let mut ranked: Vec<&Chunk> = scores.keys().collect();
    ranked.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    ranked
        .into_iter()
        .enumerate()
        .map(|(rank, chunk)| (chunk.clone(), 1.0 / (RRF_K + (rank + 1) as f32)))
        .collect()
}

/// Return the indices of the `top_k` highest scores, in descending order.
pub fn sort_top_k(scores: &[f32], top_k: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..scores.len()).collect();
    indices.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    indices.truncate(top_k);
    indices
}
