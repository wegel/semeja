//! Query-type-aware score boosts: symbol definitions, embedded symbols, file stems.
//!
//! The query/symbol regexes and the keyword and stopword lists are copied from
//! semble (https://github.com/MinishLab/semble), Copyright (c) 2026
//! Thomas van Dongen, MIT License.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::tokenize::split_identifier;
use crate::types::Chunk;

/// Scores keyed by chunk, preserving insertion order.
pub type ScoreMap = IndexMap<Chunk, f32>;

// --- Constants ---

/// Keywords that introduce a definition (case-sensitive).
const DEFINITION_KEYWORDS: &[&str] = &[
    "class", "module", "defmodule", "def", "interface", "struct", "enum", "trait", "type", "func",
    "function", "object", "abstract class", "data class", "fn", "fun", "package", "namespace",
    "protocol", "record", "typedef",
];

/// SQL DDL keywords (matched case-insensitively).
const SQL_DEFINITION_KEYWORDS: &[&str] =
    &["CREATE TABLE", "CREATE VIEW", "CREATE PROCEDURE", "CREATE FUNCTION"];

/// Additive boost multiplier for chunks that define a queried symbol.
const DEFINITION_BOOST_MULTIPLIER: f32 = 3.0;
/// Additive boost multiplier for NL queries when file stems match query words.
const STEM_BOOST_MULTIPLIER: f32 = 1.0;
/// Half-strength scale for symbols only incidentally present in an NL query.
const EMBEDDED_SYMBOL_BOOST_SCALE: f32 = 0.5;
/// Minimum stem length for prefix-based non-candidate scans.
const EMBEDDED_STEM_MIN_LEN: usize = 4;
/// Fraction of max score redistributed to multi-chunk files.
const FILE_COHERENCE_BOOST_FRAC: f32 = 0.2;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "does", "for", "from", "has", "have",
    "how", "if", "in", "is", "it", "not", "of", "on", "or", "the", "to", "was", "what", "when",
    "where", "which", "who", "why", "with",
];

static SYMBOL_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:",
        r"[A-Za-z_][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Za-z_][A-Za-z0-9_]*)+",
        r"|_[A-Za-z0-9_]*",
        r"|[A-Za-z][A-Za-z0-9]*[A-Z_][A-Za-z0-9_]*",
        r"|[A-Z][A-Za-z0-9]*",
        r")$",
    ))
    .expect("valid symbol query regex")
});

static EMBEDDED_SYMBOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Z][a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*|[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]+)\b")
        .expect("valid embedded symbol regex")
});

static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").expect("valid word regex"));

// --- Public API ---

/// Return true if the query looks like a bare or namespace-qualified symbol.
pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

/// Apply query-type boosts to candidate scores, returning a new score map.
pub fn apply_query_boost(combined_scores: &ScoreMap, query: &str, all_chunks: &[Chunk]) -> ScoreMap {
    let mut boosted = combined_scores.clone();
    let max_score = match max_value(combined_scores) {
        Some(max) => max,
        None => return boosted,
    };

    if is_symbol_query(query) {
        boost_symbol_definitions(&mut boosted, query, max_score, all_chunks);
    } else {
        boost_stem_matches(&mut boosted, query, max_score);
        boost_embedded_symbols(&mut boosted, query, max_score, all_chunks);
    }
    boosted
}

/// Promote files with multiple high-scoring chunks by boosting their top chunk.
pub fn boost_multi_chunk_files(scores: &mut ScoreMap) {
    let max_score = match max_value(scores) {
        Some(max) if max != 0.0 => max,
        _ => return,
    };

    let mut file_sum: IndexMap<String, f32> = IndexMap::new();
    let mut best_chunk: IndexMap<String, Chunk> = IndexMap::new();
    for (chunk, &score) in scores.iter() {
        *file_sum.entry(chunk.file_path.clone()).or_insert(0.0) += score;
        let is_better = match best_chunk.get(&chunk.file_path) {
            Some(current) => score > scores[current],
            None => true,
        };
        if is_better {
            best_chunk.insert(chunk.file_path.clone(), chunk.clone());
        }
    }

    let max_file_sum = file_sum.values().cloned().fold(f32::MIN, f32::max);
    let boost_unit = max_score * FILE_COHERENCE_BOOST_FRAC;
    for (file_path, chunk) in best_chunk {
        let delta = boost_unit * file_sum[&file_path] / max_file_sum;
        *scores.get_mut(&chunk).expect("best chunk present") += delta;
    }
}

// --- Symbol-definition boosting ---

struct DefPattern {
    general: Regex,
    sql: Regex,
}

/// Boost chunks defining a queried symbol, scanning candidates and non-candidates.
fn boost_symbol_definitions(boosted: &mut ScoreMap, query: &str, max_score: f32, all_chunks: &[Chunk]) {
    let symbol_name = extract_symbol_name(query);
    let mut names = vec![symbol_name.clone()];
    if symbol_name != query.trim() {
        names.push(query.trim().to_string());
    }
    let patterns = build_def_patterns(&names);
    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER;

    let candidates: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in candidates {
        let tier = definition_tier(&chunk, &patterns, &names, boost_unit);
        if tier != 0.0 {
            *boosted.get_mut(&chunk).expect("candidate present") += tier;
        }
    }

    let symbol_lower = symbol_name.to_lowercase();
    for chunk in all_chunks {
        if boosted.contains_key(chunk) {
            continue;
        }
        if !stem_matches(&file_stem_lower(&chunk.file_path), &symbol_lower) {
            continue;
        }
        let tier = definition_tier(chunk, &patterns, &names, boost_unit);
        if tier != 0.0 {
            boosted.insert(chunk.clone(), tier);
        }
    }
}

/// Boost chunks defining CamelCase symbols embedded in a natural-language query.
fn boost_embedded_symbols(boosted: &mut ScoreMap, query: &str, max_score: f32, all_chunks: &[Chunk]) {
    let names: Vec<String> = EMBEDDED_SYMBOL_RE
        .find_iter(query)
        .map(|m| m.as_str().to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if names.is_empty() {
        return;
    }
    let patterns = build_def_patterns(&names);
    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE;

    let candidates: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in candidates {
        let tier = definition_tier(&chunk, &patterns, &names, boost_unit);
        if tier != 0.0 {
            *boosted.get_mut(&chunk).expect("candidate present") += tier;
        }
    }

    let symbols_lower: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
    for chunk in all_chunks {
        if boosted.contains_key(chunk) {
            continue;
        }
        let stem = file_stem_lower(&chunk.file_path);
        let stem_norm = stem.replace('_', "");
        let matched = symbols_lower.iter().any(|sym| {
            stem == *sym
                || stem_norm == *sym
                || (stem.len() >= EMBEDDED_STEM_MIN_LEN && sym.starts_with(&stem))
                || (stem_norm.len() >= EMBEDDED_STEM_MIN_LEN && sym.starts_with(&stem_norm))
        });
        if !matched {
            continue;
        }
        let tier = definition_tier(chunk, &patterns, &names, boost_unit);
        if tier != 0.0 {
            boosted.insert(chunk.clone(), tier);
        }
    }
}

/// Return the boost amount for a chunk that defines one of `names` (0.0 if none).
fn definition_tier(chunk: &Chunk, patterns: &[DefPattern], names: &[String], boost_unit: f32) -> f32 {
    let defines = patterns
        .iter()
        .any(|p| p.general.is_match(&chunk.content) || p.sql.is_match(&chunk.content));
    if !defines {
        return 0.0;
    }
    let stem = file_stem_lower(&chunk.file_path);
    let stem_match = names.iter().any(|n| stem_matches(&stem, &n.to_lowercase()));
    boost_unit * if stem_match { 1.5 } else { 1.0 }
}

/// Build the general + SQL definition regexes for each symbol name.
fn build_def_patterns(names: &[String]) -> Vec<DefPattern> {
    let general_body = DEFINITION_KEYWORDS.iter().map(|k| regex::escape(k)).collect::<Vec<_>>().join("|");
    let sql_body = SQL_DEFINITION_KEYWORDS.iter().map(|k| regex::escape(k)).collect::<Vec<_>>().join("|");
    let ns_prefix = r"(?:[A-Za-z_][A-Za-z0-9_]*(?:\.|::))*";

    names
        .iter()
        .map(|name| {
            let escaped = regex::escape(name);
            let suffix = format!(r")\s+{ns_prefix}{escaped}(?:\s|[<({{:\[;]|$)");
            let general = Regex::new(&format!(r"(?m)(?:^|\s)(?:{general_body}{suffix}"))
                .expect("valid definition regex");
            let sql = Regex::new(&format!(r"(?mi)(?:^|\s)(?:{sql_body}{suffix}"))
                .expect("valid sql definition regex");
            DefPattern { general, sql }
        })
        .collect()
}

/// Extract the final identifier from a possibly namespace-qualified query.
fn extract_symbol_name(query: &str) -> String {
    for separator in ["::", "\\", "->", "."] {
        if let Some(idx) = query.rfind(separator) {
            return query[idx + separator.len()..].to_string();
        }
    }
    query.trim().to_string()
}

/// Return true if `stem` matches `name` (exact, snake-normalised, or plural).
fn stem_matches(stem: &str, name: &str) -> bool {
    let stem_norm = stem.replace('_', "");
    stem == name
        || stem_norm == name
        || stem.trim_end_matches('s') == name
        || stem_norm.trim_end_matches('s') == name
}

// --- File-stem boosting ---

/// Boost chunks whose file paths match natural-language query keywords.
fn boost_stem_matches(boosted: &mut ScoreMap, query: &str, max_score: f32) {
    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let keywords: HashSet<String> = WORD_RE
        .find_iter(query)
        .map(|m| m.as_str())
        .filter(|w| w.len() > 2 && !stopwords.contains(w.to_lowercase().as_str()))
        .map(|w| w.to_lowercase())
        .collect();
    if keywords.is_empty() {
        return;
    }

    let boost = max_score * STEM_BOOST_MULTIPLIER;
    let mut path_cache: HashMap<String, HashSet<String>> = HashMap::new();
    let chunks: Vec<Chunk> = boosted.keys().cloned().collect();
    for chunk in chunks {
        let parts = path_cache
            .entry(chunk.file_path.clone())
            .or_insert_with(|| path_parts(&chunk.file_path))
            .clone();
        let n_matches = count_keyword_matches(&keywords, &parts);
        if n_matches > 0 {
            let match_ratio = n_matches as f32 / keywords.len() as f32;
            if match_ratio >= 0.10 {
                *boosted.get_mut(&chunk).expect("candidate present") += boost * match_ratio;
            }
        }
    }
}

/// Collect the identifier parts of a file's stem and parent directory.
fn path_parts(file_path: &str) -> HashSet<String> {
    let path = Path::new(file_path);
    let mut parts: HashSet<String> = HashSet::new();
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        parts.extend(split_identifier(stem));
    }
    if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) {
        if !parent.is_empty() && parent != "." && parent != "/" && parent != ".." {
            parts.extend(split_identifier(parent));
        }
    }
    parts
}

/// Count query keywords matching path parts, allowing prefix overlap (min 3 chars).
fn count_keyword_matches(keywords: &HashSet<String>, parts: &HashSet<String>) -> usize {
    let exact: HashSet<&String> = keywords.intersection(parts).collect();
    if exact.len() == keywords.len() {
        return exact.len();
    }
    let mut n_matches = exact.len();
    for keyword in keywords.iter().filter(|k| !exact.contains(k)) {
        for part in parts {
            let (shorter, longer) =
                if keyword.len() <= part.len() { (keyword, part) } else { (part, keyword) };
            if shorter.len() >= 3 && longer.starts_with(shorter.as_str()) {
                n_matches += 1;
                break;
            }
        }
    }
    n_matches
}

// --- Small helpers ---

/// Lowercased file stem of a path.
fn file_stem_lower(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// Maximum value in a score map, if non-empty.
fn max_value(scores: &ScoreMap) -> Option<f32> {
    scores.values().copied().reduce(f32::max)
}
