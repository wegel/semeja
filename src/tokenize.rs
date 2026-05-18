//! Identifier tokenization for BM25 indexing, with camelCase/snake_case expansion.

use std::sync::LazyLock;

use regex::Regex;

static TOKEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z_][a-zA-Z0-9_]*").expect("valid token regex"));

/// Split a single identifier into sub-tokens via camelCase/snake_case.
///
/// Returns the original token (lowered) plus any sub-tokens, e.g.
/// `"HandlerStack"` becomes `["handlerstack", "handler", "stack"]`.
pub fn split_identifier(token: &str) -> Vec<String> {
    let lower = token.to_lowercase();

    let parts: Vec<String> = if token.contains('_') {
        lower.split('_').filter(|p| !p.is_empty()).map(str::to_string).collect()
    } else {
        camel_parts(token).iter().map(|p| p.to_lowercase()).collect()
    };

    if parts.len() >= 2 {
        let mut out = Vec::with_capacity(parts.len() + 1);
        out.push(lower);
        out.extend(parts);
        out
    } else {
        vec![lower]
    }
}

/// Split text into lowercase identifier-like tokens for BM25 indexing.
///
/// Compound identifiers are expanded into sub-tokens so partial matches work;
/// the original compound token is preserved for exact-match boosting.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for m in TOKEN_RE.find_iter(text) {
        result.extend(split_identifier(m.as_str()));
    }
    result
}

// --- Private helpers ---

/// Split an identifier on camelCase/PascalCase boundaries.
///
/// Mirrors the regex `[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+`:
/// `"getHTTPResponse"` becomes `["get", "HTTP", "Response"]`.
fn camel_parts(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    let n = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c.is_ascii_uppercase() {
            let mut j = i;
            while j < n && chars[j].is_ascii_uppercase() {
                j += 1;
            }
            let trailing_lower = j < n && chars[j].is_ascii_lowercase();
            if trailing_lower && j - i > 1 {
                // Acronym run: last uppercase letter starts the next word.
                parts.push(chars[i..j - 1].iter().collect());
                i = j - 1;
                let mut k = i + 1;
                while k < n && chars[k].is_ascii_lowercase() {
                    k += 1;
                }
                parts.push(chars[i..k].iter().collect());
                i = k;
            } else if trailing_lower {
                let mut k = j;
                while k < n && chars[k].is_ascii_lowercase() {
                    k += 1;
                }
                parts.push(chars[i..k].iter().collect());
                i = k;
            } else {
                parts.push(chars[i..j].iter().collect());
                i = j;
            }
        } else if c.is_ascii_lowercase() {
            let mut j = i;
            while j < n && chars[j].is_ascii_lowercase() {
                j += 1;
            }
            parts.push(chars[i..j].iter().collect());
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            parts.push(chars[i..j].iter().collect());
            i = j;
        } else {
            i += 1;
        }
    }
    parts
}
