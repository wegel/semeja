//! Text embedding and cosine-similarity vector search.

use anyhow::{Context, Result};
use model2vec_rs::model::StaticModel;
use rayon::prelude::*;

use crate::types::Chunk;

/// Texts per parallel embedding batch.
const EMBED_BATCH: usize = 64;

/// Default embedding model, a distilled `model2vec` static model.
pub const DEFAULT_MODEL_NAME: &str = "minishlab/potion-code-16M";

/// Dimensionality of the [`MockEncoder`].
const MOCK_DIM: usize = 256;

// --- Encoder ---

/// An embedding model that turns text into fixed-width float vectors.
pub trait Encoder {
    /// Encode texts into embedding vectors, one per input.
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>>;
}

/// A `model2vec` static embedding model: real, CPU-only semantic embeddings.
pub struct StaticModelEncoder {
    model: StaticModel,
}

impl Encoder for StaticModelEncoder {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        if texts.is_empty() {
            return Vec::new();
        }
        // Embed batches in parallel; output order matches input order.
        texts.par_chunks(EMBED_BATCH).flat_map_iter(|batch| self.model.encode(batch)).collect()
    }
}

/// Load the embedding model, fetching it from the Hugging Face hub if needed.
///
/// Downloaded models are cached locally by `hf-hub`, so only the first call
/// for a given model touches the network.
pub fn load_model(model_path: Option<&str>) -> Result<Box<dyn Encoder>> {
    let name = model_path.unwrap_or(DEFAULT_MODEL_NAME);
    let model = StaticModel::from_pretrained(name, None, None, None)
        .with_context(|| format!("load embedding model {name:?}"))?;
    Ok(Box::new(StaticModelEncoder { model }))
}

/// Embed chunk contents into a matrix of embedding vectors.
pub fn embed_chunks(model: &dyn Encoder, chunks: &[Chunk]) -> Vec<Vec<f32>> {
    if chunks.is_empty() {
        return Vec::new();
    }
    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
    model.encode(&texts)
}

/// A deterministic, dependency-free encoder for tests and benchmarks.
///
/// It hashes text into a reproducible unit vector. It carries no semantic
/// signal and must never be used for real search.
pub struct MockEncoder;

impl MockEncoder {
    /// Create a mock encoder.
    pub fn new() -> MockEncoder {
        MockEncoder
    }
}

impl Default for MockEncoder {
    fn default() -> Self {
        MockEncoder::new()
    }
}

impl Encoder for MockEncoder {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|text| hash_embed(text)).collect()
    }
}

// --- Vector index ---

/// A cosine-similarity vector index supporting selector-restricted queries.
pub struct CosineBackend {
    vectors: Vec<Vec<f32>>,
}

impl CosineBackend {
    /// Build an index over the given embedding vectors (normalised on entry).
    pub fn new(vectors: Vec<Vec<f32>>) -> CosineBackend {
        let vectors = vectors.into_iter().map(|v| normalize(&v)).collect();
        CosineBackend { vectors }
    }

    /// The number of indexed vectors.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// True when the index holds no vectors.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Return the `k` nearest neighbours (by cosine distance) for each query.
    ///
    /// Each result is `(indices, distances)` sorted by ascending distance.
    /// `selector` restricts retrieval to the given chunk indices.
    pub fn query(
        &self,
        vectors: &[Vec<f32>],
        k: usize,
        selector: Option<&[usize]>,
    ) -> Result<Vec<(Vec<usize>, Vec<f32>)>> {
        anyhow::ensure!(k >= 1, "k should be >= 1, is now {k}");
        let mut effective_k = k.min(self.vectors.len());
        if let Some(sel) = selector {
            effective_k = effective_k.min(sel.len());
        }

        let mut out = Vec::with_capacity(vectors.len());
        for query in vectors {
            let query = normalize(query);
            let candidates: Vec<usize> = match selector {
                Some(sel) => sel.to_vec(),
                None => (0..self.vectors.len()).collect(),
            };
            let mut scored: Vec<(usize, f32)> = candidates
                .into_par_iter()
                .map(|idx| (idx, 1.0 - dot(&query, &self.vectors[idx])))
                .collect();
            scored.sort_by(|a, b| a.1.total_cmp(&b.1));
            scored.truncate(effective_k);
            out.push((scored.iter().map(|s| s.0).collect(), scored.iter().map(|s| s.1).collect()));
        }
        Ok(out)
    }
}

// --- Private helpers ---

/// Hash `text` into a deterministic pseudo-random unit vector.
fn hash_embed(text: &str) -> Vec<f32> {
    // FNV-1a hash seeds an xorshift generator for reproducible vectors.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut state = hash | 1;
    let mut vector = Vec::with_capacity(MOCK_DIM);
    for _ in 0..MOCK_DIM {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        vector.push((state as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32);
    }
    normalize(&vector)
}

/// Return the L2-normalised copy of a vector.
fn normalize(vector: &[f32]) -> Vec<f32> {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    vector.iter().map(|v| v / (norm + 1e-8)).collect()
}

/// Dot product of two equal-length vectors.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
