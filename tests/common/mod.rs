//! Shared test helpers.

#![allow(dead_code)]

use semeja::types::Chunk;

/// Create a minimal Python chunk for use in tests.
pub fn make_chunk(content: &str, file_path: &str) -> Chunk {
    Chunk {
        content: content.to_string(),
        file_path: file_path.to_string(),
        start_line: 1,
        end_line: content.matches('\n').count() + 1,
        language: Some("python".to_string()),
    }
}
