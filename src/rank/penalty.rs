//! File-path penalties and diversity reranking for the top-k result selection.
//!
//! The test-file and path-pattern regexes are copied from semble
//! (https://github.com/MinishLab/semble), Copyright (c) 2026 Thomas van Dongen,
//! MIT License.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::rank::boost::ScoreMap;
use crate::types::Chunk;

/// Strong penalty: test files, compat shims, example/doc code.
const STRONG_PENALTY: f32 = 0.3;
/// Moderate penalty: re-export / metadata files.
const MODERATE_PENALTY: f32 = 0.5;
/// Mild penalty: `.d.ts` declaration stubs.
const MILD_PENALTY: f32 = 0.7;

/// Chunks from the same file allowed before the saturation penalty applies.
const FILE_SATURATION_THRESHOLD: usize = 1;
/// Multiplicative decay per extra chunk from the same file.
const FILE_SATURATION_DECAY: f32 = 0.5;

/// Filenames that are re-export barrels or package metadata.
const REEXPORT_FILENAMES: &[&str] = &["__init__.py", "package-info.java"];

static TEST_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?:^|/)(?:",
        r"test_[^/]*\.py|[^/]*_test\.py",
        r"|[^/]*_test\.go",
        r"|[^/]*Tests?\.java",
        r"|[^/]*Test\.php",
        r"|[^/]*_spec\.rb|[^/]*_test\.rb",
        r"|[^/]*\.test\.[jt]sx?|[^/]*\.spec\.[jt]sx?",
        r"|[^/]*Tests?\.kt|[^/]*Spec\.kt",
        r"|[^/]*Tests?\.swift|[^/]*Spec\.swift",
        r"|[^/]*Tests?\.cs",
        r"|test_[^/]*\.cpp|[^/]*_test\.cpp",
        r"|test_[^/]*\.c|[^/]*_test\.c",
        r"|[^/]*Spec\.scala|[^/]*Suite\.scala|[^/]*Test\.scala",
        r"|[^/]*_test\.dart|test_[^/]*\.dart",
        r"|[^/]*_spec\.lua|[^/]*_test\.lua|test_[^/]*\.lua",
        r"|test_helpers?[^/]*\.\w+",
        r")$",
    ))
    .expect("valid test-file regex")
});

static TEST_DIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|/)(?:tests?|__tests__|spec|testing)(?:/|$)").expect("valid test-dir regex")
});

static COMPAT_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)(?:compat|_compat|legacy)(?:/|$)").expect("valid compat regex"));

static EXAMPLES_DIR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|/)(?:_?examples?|docs?_src)(?:/|$)").expect("valid examples regex")
});

static TYPE_DEFS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.d\.ts$").expect("valid type-defs regex"));

/// Select the top-k results, applying path penalties and file-saturation decay.
///
/// When `penalise_paths` is set, test/compat/example/re-export paths are
/// down-weighted before ranking; saturation decay reduces repeated hits from
/// the same file.
pub fn rerank_topk(scores: &ScoreMap, top_k: usize, penalise_paths: bool) -> Vec<(Chunk, f32)> {
    if scores.is_empty() {
        return Vec::new();
    }

    // Apply file-path penalties.
    let mut penalty_cache: HashMap<String, f32> = HashMap::new();
    let penalised: Vec<(Chunk, f32)> = scores
        .iter()
        .map(|(chunk, &score)| {
            let value = if penalise_paths {
                let penalty = *penalty_cache
                    .entry(chunk.file_path.clone())
                    .or_insert_with(|| file_path_penalty(&chunk.file_path));
                score * penalty
            } else {
                score
            };
            (chunk.clone(), value)
        })
        .collect();

    // Sort by penalised score, highest first (stable for ties).
    let mut ranked = penalised.clone();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut file_selected: HashMap<String, usize> = HashMap::new();
    let mut selected: Vec<(f32, Chunk)> = Vec::new();
    let mut min_selected = f32::INFINITY;

    for (chunk, pen_score) in ranked {
        if selected.len() >= top_k && pen_score <= min_selected {
            break;
        }
        let already = *file_selected.get(&chunk.file_path).unwrap_or(&0);
        let mut eff_score = pen_score;
        if already >= FILE_SATURATION_THRESHOLD {
            let excess = already - FILE_SATURATION_THRESHOLD + 1;
            eff_score *= FILE_SATURATION_DECAY.powi(excess as i32);
        }
        selected.push((eff_score, chunk.clone()));
        file_selected.insert(chunk.file_path.clone(), already + 1);
        if selected.len() >= top_k {
            min_selected = selected.iter().map(|s| s.0).fold(f32::INFINITY, f32::min);
        }
    }

    selected.sort_by(|a, b| b.0.total_cmp(&a.0));
    selected.into_iter().take(top_k).map(|(score, chunk)| (chunk, score)).collect()
}

/// Return a combined multiplicative penalty for all applicable path patterns.
fn file_path_penalty(file_path: &str) -> f32 {
    let normalised = file_path.replace('\\', "/");
    let mut penalty = 1.0;
    if TEST_FILE_RE.is_match(&normalised) || TEST_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    let name = Path::new(file_path).file_name().and_then(|n| n.to_str()).unwrap_or("");
    if REEXPORT_FILENAMES.contains(&name) {
        penalty *= MODERATE_PENALTY;
    }
    if COMPAT_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if EXAMPLES_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if TYPE_DEFS_RE.is_match(&normalised) {
        penalty *= MILD_PENALTY;
    }
    penalty
}
