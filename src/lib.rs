//! Fast and accurate code search for agents: hybrid BM25 + semantic retrieval.

pub mod bm25;
pub mod chunk;
pub mod cli;
pub mod embed;
pub mod index;
pub mod lang;
pub mod lang_table;
pub mod mcp;
pub mod rank;
pub mod search;
pub mod stats;
pub mod tokenize;
pub mod types;
pub mod utils;
pub mod walk;

/// Crate version, matching the package version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use index::SemejaIndex;
pub use types::{Chunk, IndexStats, SearchMode, SearchResult};
