//! Resolution of the hybrid-search blending weight (`alpha`).

use crate::rank::boost::is_symbol_query;

/// Blend weight leaning toward BM25 for exact keyword matching.
const ALPHA_SYMBOL: f32 = 0.3;
/// Balanced semantic + BM25 blend weight for natural-language queries.
const ALPHA_NL: f32 = 0.5;

/// Return the semantic-score blend weight, auto-detecting from query type.
pub fn resolve_alpha(query: &str, alpha: Option<f32>) -> f32 {
    if let Some(alpha) = alpha {
        return alpha;
    }
    if is_symbol_query(query) {
        ALPHA_SYMBOL
    } else {
        ALPHA_NL
    }
}
