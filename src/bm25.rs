//! Sparse (BM25) indexing, with file-path enrichment for path-aware queries.

use std::collections::HashMap;
use std::path::Path;

use crate::types::Chunk;

/// BM25 term-frequency saturation parameter.
const K1: f32 = 1.5;
/// BM25 length-normalisation parameter.
const B: f32 = 0.75;

/// Convert a selector of chunk indices into a boolean mask of length `size`.
pub fn selector_to_mask(selector: Option<&[usize]>, size: usize) -> Option<Vec<bool>> {
    let selector = selector?;
    let mut mask = vec![false; size];
    for &index in selector {
        if index < size {
            mask[index] = true;
        }
    }
    Some(mask)
}

/// Append file-path components to chunk content to boost path-based queries.
pub fn enrich_for_bm25(chunk: &Chunk) -> String {
    let path = Path::new(&chunk.file_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir_parts: Vec<&str> = path
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| c.as_os_str().to_str())
                .filter(|s| *s != "." && *s != "/")
                .collect()
        })
        .unwrap_or_default();
    let start = dir_parts.len().saturating_sub(3);
    let dir_text = dir_parts[start..].join(" ");
    // Repeat the stem twice to up-weight file-path matches in BM25.
    format!("{} {stem} {stem} {dir_text}", chunk.content)
}

// --- BM25 index ---

/// A BM25 lexical index over a corpus of pre-tokenized documents.
pub struct Bm25Index {
    doc_count: usize,
    avg_doc_len: f32,
    doc_lengths: Vec<f32>,
    postings: HashMap<String, Vec<(usize, u32)>>,
    idf: HashMap<String, f32>,
}

impl Bm25Index {
    /// Build a BM25 index from a corpus of token lists.
    pub fn build(corpus: &[Vec<String>]) -> Bm25Index {
        let doc_count = corpus.len();
        let doc_lengths: Vec<f32> = corpus.iter().map(|doc| doc.len() as f32).collect();
        let total: f32 = doc_lengths.iter().sum();
        let avg_doc_len = if doc_count > 0 { total / doc_count as f32 } else { 0.0 };

        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        for (doc_idx, doc) in corpus.iter().enumerate() {
            let mut term_counts: HashMap<&str, u32> = HashMap::new();
            for token in doc {
                *term_counts.entry(token.as_str()).or_insert(0) += 1;
            }
            for (term, count) in term_counts {
                postings.entry(term.to_string()).or_default().push((doc_idx, count));
            }
        }

        let idf = postings
            .iter()
            .map(|(term, posts)| {
                let df = posts.len() as f32;
                let n = doc_count as f32;
                (term.clone(), (1.0 + (n - df + 0.5) / (df + 0.5)).ln())
            })
            .collect();

        Bm25Index { doc_count, avg_doc_len, doc_lengths, postings, idf }
    }

    /// Number of indexed documents.
    pub fn doc_count(&self) -> usize {
        self.doc_count
    }

    /// Score every document against the query tokens.
    ///
    /// When `mask` is given, documents masked `false` are zeroed out.
    pub fn get_scores(&self, query: &[String], mask: Option<&[bool]>) -> Vec<f32> {
        let mut scores = vec![0.0_f32; self.doc_count];
        if self.avg_doc_len == 0.0 {
            return scores;
        }
        for token in query {
            let posts = match self.postings.get(token) {
                Some(posts) => posts,
                None => continue,
            };
            let idf = self.idf[token];
            for &(doc, freq) in posts {
                let freq = freq as f32;
                let norm = K1 * (1.0 - B + B * self.doc_lengths[doc] / self.avg_doc_len);
                scores[doc] += idf * freq * (K1 + 1.0) / (freq + norm);
            }
        }
        if let Some(mask) = mask {
            for (score, keep) in scores.iter_mut().zip(mask) {
                if !keep {
                    *score = 0.0;
                }
            }
        }
        scores
    }
}
