//! The `SemejaIndex`: building and searching a local code index.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use rayon::prelude::*;

use crate::bm25::{enrich_for_bm25, Bm25Index};
use crate::chunk::chunk_source;
use crate::embed::{embed_chunks, load_model, CosineBackend, Encoder};
use crate::lang::{detect_language, get_extensions};
use crate::search::{search_bm25, search_hybrid, search_semantic};
use crate::stats::{default_stats_file, save_search_stats};
use crate::tokenize::tokenize;
use crate::types::{CallType, Chunk, IndexStats, SearchMode, SearchResult};
use crate::walk::walk_files;

/// Maximum file size to read and index (1 MB).
pub const MAX_FILE_BYTES: u64 = 1_000_000;

/// A fast local code index supporting semantic, BM25, and hybrid search.
pub struct SemejaIndex {
    model: Box<dyn Encoder>,
    /// All indexed chunks, in discovery order.
    pub chunks: Vec<Chunk>,
    bm25_index: Bm25Index,
    semantic_index: CosineBackend,
    file_sizes: HashMap<String, usize>,
    file_mapping: HashMap<String, Vec<usize>>,
    language_mapping: HashMap<String, Vec<usize>>,
    stats_file: PathBuf,
}

impl SemejaIndex {
    /// Create and index a `SemejaIndex` from a directory.
    pub fn from_path(
        path: &Path,
        model: Option<Box<dyn Encoder>>,
        extensions: Option<&[String]>,
        include_text_files: bool,
    ) -> Result<SemejaIndex> {
        let model = match model {
            Some(model) => model,
            None => load_model(None)?,
        };
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }
        if !path.is_dir() {
            bail!("Path is not a directory: {}", path.display());
        }
        let path = path.canonicalize()?;
        let (bm25, semantic, chunks) =
            create_index_from_path(&path, model.as_ref(), extensions, include_text_files, Some(&path))?;
        Ok(SemejaIndex::assemble(model, bm25, semantic, chunks, &path))
    }

    /// Clone a git repository into a temporary directory and index it.
    pub fn from_git(
        url: &str,
        git_ref: Option<&str>,
        model: Option<Box<dyn Encoder>>,
        extensions: Option<&[String]>,
        include_text_files: bool,
    ) -> Result<SemejaIndex> {
        let model = match model {
            Some(model) => model,
            None => load_model(None)?,
        };
        let tmp_dir = tempfile::tempdir()?;
        clone_repo(url, git_ref, tmp_dir.path())?;
        let resolved = tmp_dir.path().canonicalize()?;
        let (bm25, semantic, chunks) =
            create_index_from_path(&resolved, model.as_ref(), extensions, include_text_files, Some(&resolved))?;
        Ok(SemejaIndex::assemble(model, bm25, semantic, chunks, &resolved))
    }

    /// Statistics about the current index state.
    pub fn stats(&self) -> IndexStats {
        let mut languages: HashMap<String, usize> = HashMap::new();
        for chunk in &self.chunks {
            if let Some(language) = &chunk.language {
                *languages.entry(language.clone()).or_insert(0) += 1;
            }
        }
        IndexStats {
            indexed_files: self.file_mapping.len(),
            total_chunks: self.chunks.len(),
            languages,
        }
    }

    /// Return chunks semantically similar to the given chunk.
    pub fn find_related(&self, source: &Chunk, top_k: usize) -> Result<Vec<SearchResult>> {
        let selector = match &source.language {
            Some(language) => self.selector_vector(&[language.clone()], &[]),
            None => None,
        };
        let mut results = search_semantic(
            &source.content,
            self.model.as_ref(),
            &self.semantic_index,
            &self.chunks,
            top_k + 1,
            selector.as_deref(),
        )?;
        results.retain(|r| &r.chunk != source);
        results.truncate(top_k);
        save_search_stats(&results, CallType::FindRelated, &self.file_sizes, &self.stats_file);
        Ok(results)
    }

    /// Search the index and return the top-k most relevant chunks.
    pub fn search(
        &self,
        query: &str,
        top_k: usize,
        mode: &str,
        alpha: Option<f32>,
        filter_languages: &[String],
        filter_paths: &[String],
    ) -> Result<Vec<SearchResult>> {
        let mode = SearchMode::parse(mode).ok_or_else(|| anyhow!("Unknown search mode: {mode:?}"))?;
        if self.chunks.is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let selector = self.selector_vector(filter_languages, filter_paths);
        let selector = selector.as_deref();

        let results = match mode {
            SearchMode::Bm25 => search_bm25(query, &self.bm25_index, &self.chunks, top_k, selector),
            SearchMode::Semantic => search_semantic(
                query,
                self.model.as_ref(),
                &self.semantic_index,
                &self.chunks,
                top_k,
                selector,
            )?,
            SearchMode::Hybrid => search_hybrid(
                query,
                self.model.as_ref(),
                &self.semantic_index,
                &self.bm25_index,
                &self.chunks,
                top_k,
                alpha,
                selector,
            )?,
        };
        save_search_stats(&results, CallType::Search, &self.file_sizes, &self.stats_file);
        Ok(results)
    }

    /// Assemble an index from its built components.
    fn assemble(
        model: Box<dyn Encoder>,
        bm25_index: Bm25Index,
        semantic_index: CosineBackend,
        chunks: Vec<Chunk>,
        root: &Path,
    ) -> SemejaIndex {
        let (file_mapping, language_mapping) = populate_mapping(&chunks);
        SemejaIndex {
            model,
            file_sizes: compute_file_sizes(&chunks, root),
            chunks,
            bm25_index,
            semantic_index,
            file_mapping,
            language_mapping,
            stats_file: default_stats_file(),
        }
    }

    /// Build a sorted, de-duplicated selector of chunk indices for filters.
    fn selector_vector(&self, filter_languages: &[String], filter_paths: &[String]) -> Option<Vec<usize>> {
        let mut selector: Vec<usize> = Vec::new();
        for language in filter_languages {
            if let Some(indices) = self.language_mapping.get(language) {
                selector.extend(indices);
            }
        }
        for path in filter_paths {
            if let Some(indices) = self.file_mapping.get(path) {
                selector.extend(indices);
            }
        }
        if selector.is_empty() {
            return None;
        }
        selector.sort_unstable();
        selector.dedup();
        Some(selector)
    }
}

// --- Index construction ---

/// Build BM25 and vector indexes from a resolved directory.
///
/// When `display_root` is set, chunk file paths are stored relative to it.
pub fn create_index_from_path(
    path: &Path,
    model: &dyn Encoder,
    extensions: Option<&[String]>,
    include_text_files: bool,
    display_root: Option<&Path>,
) -> Result<(Bm25Index, CosineBackend, Vec<Chunk>)> {
    let extensions = get_extensions(include_text_files, extensions);

    // Read and chunk every file in parallel; output order matches file order.
    let chunks: Vec<Chunk> = walk_files(path, &extensions, None)
        .par_iter()
        .flat_map_iter(|file_path| chunk_file(file_path, display_root))
        .collect();

    if chunks.is_empty() {
        bail!("No supported files found under {}.", path.display());
    }

    let embeddings = embed_chunks(model, &chunks);
    let corpus: Vec<Vec<String>> =
        chunks.par_iter().map(|chunk| tokenize(&enrich_for_bm25(chunk))).collect();
    Ok((Bm25Index::build(&corpus), CosineBackend::new(embeddings), chunks))
}

/// Return a mapping of repo-relative file path to total character count.
pub fn compute_file_sizes(chunks: &[Chunk], root: &Path) -> HashMap<String, usize> {
    let mut sizes: HashMap<String, usize> = HashMap::new();
    for chunk in chunks {
        if sizes.contains_key(&chunk.file_path) {
            continue;
        }
        if let Ok(bytes) = fs::read(root.join(&chunk.file_path)) {
            sizes.insert(chunk.file_path.clone(), String::from_utf8_lossy(&bytes).chars().count());
        }
    }
    sizes
}

// --- Private helpers ---

/// Read one file and split it into chunks, skipping oversized or unreadable files.
fn chunk_file(file_path: &Path, display_root: Option<&Path>) -> Vec<Chunk> {
    match fs::metadata(file_path) {
        Ok(meta) if meta.len() <= MAX_FILE_BYTES => {}
        _ => return Vec::new(),
    }
    let bytes = match fs::read(file_path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let source = String::from_utf8_lossy(&bytes);
    let chunk_path = match display_root {
        Some(root) => file_path.strip_prefix(root).unwrap_or(file_path),
        None => file_path,
    };
    chunk_source(&source, &chunk_path.to_string_lossy(), detect_language(file_path).as_deref())
}

/// Build `(file → chunk indices, language → chunk indices)` mappings.
fn populate_mapping(chunks: &[Chunk]) -> (HashMap<String, Vec<usize>>, HashMap<String, Vec<usize>>) {
    let mut file_to_id: HashMap<String, Vec<usize>> = HashMap::new();
    let mut language_to_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if let Some(language) = &chunk.language {
            language_to_id.entry(language.clone()).or_default().push(i);
        }
        file_to_id.entry(chunk.file_path.clone()).or_default().push(i);
    }
    (file_to_id, language_to_id)
}

/// Clone a git repository, mapping failures to descriptive errors.
fn clone_repo(url: &str, git_ref: Option<&str>, dest: &Path) -> Result<()> {
    let mut command = Command::new("git");
    command.args(["clone", "--depth", "1"]);
    if let Some(reference) = git_ref {
        command.args(["--branch", reference]);
    }
    // `--` prevents `url` from being interpreted as a git option.
    command.args(["--", url]).arg(dest);

    let output = match command.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("git is not installed or not on PATH");
        }
        Err(err) => return Err(err.into()),
    };
    if !output.status.success() {
        bail!("git clone failed for {url:?}:\n{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}
