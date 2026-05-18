//! Result ranking: query-aware boosts, path penalties, and alpha weighting.

pub mod boost;
pub mod penalty;
pub mod weight;

pub use boost::{apply_query_boost, boost_multi_chunk_files, is_symbol_query, ScoreMap};
pub use penalty::rerank_topk;
pub use weight::resolve_alpha;
