//! Core data types shared across indexing, search, and ranking.

// --- Search mode ---

/// Search strategy for [`crate::index::SemejaIndex::search`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchMode {
    /// Alpha-weighted blend of semantic and BM25 scores.
    Hybrid,
    /// Pure vector-similarity search.
    Semantic,
    /// Pure lexical BM25 search.
    Bm25,
}

impl SearchMode {
    /// Parse a mode from its lowercase string label.
    pub fn parse(value: &str) -> Option<SearchMode> {
        match value {
            "hybrid" => Some(SearchMode::Hybrid),
            "semantic" => Some(SearchMode::Semantic),
            "bm25" => Some(SearchMode::Bm25),
            _ => None,
        }
    }

    /// The lowercase string label for this mode.
    pub fn as_str(self) -> &'static str {
        match self {
            SearchMode::Hybrid => "hybrid",
            SearchMode::Semantic => "semantic",
            SearchMode::Bm25 => "bm25",
        }
    }
}

/// Call type recorded for token-savings tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallType {
    /// A `search` call.
    Search,
    /// A `find_related` call.
    FindRelated,
}

impl CallType {
    /// The string label stored in the stats file.
    pub fn as_str(self) -> &'static str {
        match self {
            CallType::Search => "search",
            CallType::FindRelated => "find_related",
        }
    }
}

// --- Chunk ---

/// A single indexable unit of code.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chunk {
    /// The raw source text of the chunk.
    pub content: String,
    /// Repo-relative path of the file the chunk came from.
    pub file_path: String,
    /// 1-indexed first line of the chunk.
    pub start_line: usize,
    /// 1-indexed last line of the chunk.
    pub end_line: usize,
    /// Detected language, if any.
    pub language: Option<String>,
}

impl Chunk {
    /// File path and line range as a `path:start-end` string.
    pub fn location(&self) -> String {
        format!("{}:{}-{}", self.file_path, self.start_line, self.end_line)
    }
}

// --- Search result ---

/// A single search result with its score and originating mode.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// The matched chunk.
    pub chunk: Chunk,
    /// The relevance score; comparable only within one result set.
    pub score: f32,
    /// The search mode that produced this result.
    pub source: SearchMode,
}

// --- Index stats ---

/// Statistics about the current index state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexStats {
    /// Number of distinct files indexed.
    pub indexed_files: usize,
    /// Total number of chunks across all files.
    pub total_chunks: usize,
    /// Per-language chunk counts.
    pub languages: std::collections::HashMap<String, usize>,
}
