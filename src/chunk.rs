//! Source-code chunking: split files into indexable units along syntax boundaries.

use tree_sitter::{Node, Parser};
use tree_sitter_language::LanguageFn;

use crate::lang::all_languages;
use crate::types::Chunk;

/// Desired chunk length in characters for source code.
const DESIRED_CHUNK_LENGTH_CHARS: usize = 1500;
/// Desired chunk length in characters for prose/Markdown — larger so whole
/// document sections (heading plus body) usually stay in one chunk.
const MARKDOWN_CHUNK_LENGTH_CHARS: usize = 3000;

/// Bundled tree-sitter grammars, keyed by language name from [`crate::lang`].
///
/// Languages absent from this table fall back to line-based chunking.
const GRAMMARS: &[(&str, LanguageFn)] = &[
    ("python", tree_sitter_python::LANGUAGE),
    ("javascript", tree_sitter_javascript::LANGUAGE),
    ("typescript", tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
    ("tsx", tree_sitter_typescript::LANGUAGE_TSX),
    ("rust", tree_sitter_rust::LANGUAGE),
    ("go", tree_sitter_go::LANGUAGE),
    ("java", tree_sitter_java::LANGUAGE),
    ("c", tree_sitter_c::LANGUAGE),
    ("cpp", tree_sitter_cpp::LANGUAGE),
    ("csharp", tree_sitter_c_sharp::LANGUAGE),
    ("ruby", tree_sitter_ruby::LANGUAGE),
    ("php", tree_sitter_php::LANGUAGE_PHP),
    ("bash", tree_sitter_bash::LANGUAGE),
    ("html", tree_sitter_html::LANGUAGE),
    ("css", tree_sitter_css::LANGUAGE),
    ("json", tree_sitter_json::LANGUAGE),
    ("scala", tree_sitter_scala::LANGUAGE),
    ("haskell", tree_sitter_haskell::LANGUAGE),
    ("ocaml", tree_sitter_ocaml::LANGUAGE_OCAML),
    ("ocaml_interface", tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE),
    ("elixir", tree_sitter_elixir::LANGUAGE),
    ("lua", tree_sitter_lua::LANGUAGE),
    ("kotlin", tree_sitter_kotlin_ng::LANGUAGE),
    ("swift", tree_sitter_swift::LANGUAGE),
    ("r", tree_sitter_r::LANGUAGE),
    ("sql", tree_sitter_sequel::LANGUAGE),
    ("dart", tree_sitter_dart::LANGUAGE),
    ("perl", tree_sitter_perl::LANGUAGE),
    ("objc", tree_sitter_objc::LANGUAGE),
    ("julia", tree_sitter_julia::LANGUAGE),
    ("groovy", tree_sitter_groovy::LANGUAGE),
    ("powershell", tree_sitter_powershell::LANGUAGE),
    ("markdown", tree_sitter_md::LANGUAGE),
    ("yaml", tree_sitter_yaml::LANGUAGE),
    ("toml", tree_sitter_toml_ng::LANGUAGE),
    ("erlang", tree_sitter_erlang::LANGUAGE),
    ("zig", tree_sitter_zig::LANGUAGE),
    ("nix", tree_sitter_nix::LANGUAGE),
    ("solidity", tree_sitter_solidity::LANGUAGE),
    ("svelte", tree_sitter_svelte_ng::LANGUAGE),
    ("ada", tree_sitter_ada::LANGUAGE),
    ("cmake", tree_sitter_cmake::LANGUAGE),
    ("make", tree_sitter_make::LANGUAGE),
    ("elm", tree_sitter_elm::LANGUAGE),
    ("hcl", tree_sitter_hcl::LANGUAGE),
    ("gleam", tree_sitter_gleam::LANGUAGE),
    ("scheme", tree_sitter_scheme::LANGUAGE),
    ("commonlisp", tree_sitter_commonlisp::LANGUAGE_COMMONLISP),
    ("fsharp", tree_sitter_fsharp::LANGUAGE_FSHARP),
    ("fsharp_signature", tree_sitter_fsharp::LANGUAGE_SIGNATURE),
    ("verilog", tree_sitter_verilog::LANGUAGE),
    ("systemverilog", tree_sitter_verilog::LANGUAGE),
    ("cuda", tree_sitter_cuda::LANGUAGE),
    ("graphql", tree_sitter_graphql::LANGUAGE),
    ("proto", tree_sitter_proto::LANGUAGE),
];

/// A half-open `[start, end)` boundary produced by the chunking algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBoundary {
    /// Inclusive start offset.
    pub start: usize,
    /// Exclusive end offset.
    pub end: usize,
}

impl ChunkBoundary {
    fn len(&self) -> usize {
        self.end - self.start
    }
}

// --- Public API ---

/// Return true if the language is one of the known code languages.
pub fn is_supported_language(language: &str) -> bool {
    all_languages().contains(language)
}

/// Return true if a tree-sitter grammar is bundled for the language.
///
/// Languages with a grammar are chunked along syntax boundaries; all others
/// fall back to line-based chunking.
pub fn has_grammar(language: &str) -> bool {
    parser_for(language).is_some()
}

/// Chunk pre-read source text into [`Chunk`] values with line ranges.
pub fn chunk_source(source: &str, file_path: &str, language: Option<&str>) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }

    let desired = match language {
        Some(lang) if is_markdown(lang) => MARKDOWN_CHUNK_LENGTH_CHARS,
        _ => DESIRED_CHUNK_LENGTH_CHARS,
    };
    let boundaries = match language {
        Some(lang) if is_supported_language(lang) => chunk(source, lang, desired),
        _ => chunk_lines(source, desired),
    };

    let chars: Vec<char> = source.chars().collect();
    boundaries
        .into_iter()
        .map(|boundary| {
            // Clamp to start so zero-length chunks don't produce an off-by-one.
            let end_index = boundary.end.saturating_sub(1).max(boundary.start);
            Chunk {
                content: chars[boundary.start..=end_index].iter().collect(),
                file_path: file_path.to_string(),
                start_line: count_newlines(&chars[..boundary.start]) + 1,
                end_line: count_newlines(&chars[..end_index]) + 1,
                language: language.map(str::to_string),
            }
        })
        .collect()
}

/// Chunk source code by line, merging adjacent lines up to `desired_length`.
pub fn chunk_lines(text: &str, desired_length: usize) -> Vec<ChunkBoundary> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<ChunkBoundary> = Vec::new();
    let mut index = 0;
    for line in text.split_inclusive('\n') {
        let len = line.chars().count();
        lines.push(ChunkBoundary { start: index, end: index + len });
        index += len;
    }
    merge_adjacent_chunks(&lines, desired_length)
}

/// Chunk source code via its tree-sitter syntax tree, falling back to lines.
pub fn chunk(text: &str, language: &str, desired_length: usize) -> Vec<ChunkBoundary> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut parser = match parser_for(language) {
        Some(parser) => parser,
        None => return chunk_lines(text, desired_length),
    };
    let tree = match parser.parse(text, None) {
        Some(tree) => tree,
        None => return chunk_lines(text, desired_length),
    };

    // The algorithm works in byte offsets; convert to char offsets for callers.
    let boundaries = if is_markdown(language) {
        chunk_markdown(tree.root_node(), desired_length)
    } else {
        chunk_node(tree.root_node(), desired_length)
    };
    boundaries
        .into_iter()
        .map(|boundary| ChunkBoundary {
            start: text[..boundary.start].chars().count(),
            end: text[..boundary.end].chars().count(),
        })
        .collect()
}

// --- Private helpers ---

/// Build a tree-sitter parser for the given language, if a grammar is bundled.
fn parser_for(language: &str) -> Option<Parser> {
    let lang = GRAMMARS.iter().find(|(name, _)| *name == language)?.1;
    let mut parser = Parser::new();
    parser.set_language(&lang.into()).ok()?;
    Some(parser)
}

/// Turn a syntax node into chunks, then merge adjacent chunks.
fn chunk_node(node: Node, desired_length: usize) -> Vec<ChunkBoundary> {
    let raw = merge_node_inner(node, desired_length);
    merge_adjacent_chunks(&raw, desired_length)
}

/// Return true if the language is Markdown (chunked section-by-section).
fn is_markdown(language: &str) -> bool {
    language == "markdown" || language == "markdown_inline"
}

/// Return true if a node is a Markdown heading.
fn is_heading(node: Node) -> bool {
    node.kind() == "atx_heading" || node.kind() == "setext_heading"
}

/// Chunk Markdown so each chunk begins at a heading and carries its section.
///
/// tree-sitter-md nests `section` nodes (heading, body, nested sections). We
/// flatten them into document order, then group greedily up to
/// `desired_length`, starting a new chunk at each heading once the current one
/// holds body text. This keeps a heading attached to the content it
/// introduces instead of drifting onto the previous section.
fn chunk_markdown(root: Node, desired_length: usize) -> Vec<ChunkBoundary> {
    let mut units: Vec<Node> = Vec::new();
    collect_markdown_units(root, &mut units);
    if units.is_empty() {
        return vec![ChunkBoundary { start: root.start_byte(), end: root.end_byte() }];
    }
    merge_markdown_units(&units, desired_length)
}

/// Flatten Markdown `section` nodes into an ordered list of headings and blocks.
fn collect_markdown_units<'a>(node: Node<'a>, units: &mut Vec<Node<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "section" {
            collect_markdown_units(child, units);
        } else {
            units.push(child);
        }
    }
}

/// Group heading/block units into heading-led chunks up to `desired_length`.
fn merge_markdown_units(units: &[Node], desired_length: usize) -> Vec<ChunkBoundary> {
    let mut groups: Vec<ChunkBoundary> = Vec::new();
    let mut index = 0;
    while index < units.len() {
        let start = units[index].start_byte();
        let mut end = units[index].end_byte();
        let mut length = end - start;
        let mut has_body = !is_heading(units[index]);
        index += 1;

        // An oversized leading block is split on its own syntax boundaries.
        if length > desired_length {
            groups.extend(merge_node_inner(units[index - 1], desired_length));
            continue;
        }

        let mut emitted = false;
        while index < units.len() {
            let next = units[index];
            let next_len = next.end_byte() - next.start_byte();
            // A heading starts a new section once the current chunk has content.
            if has_body && is_heading(next) {
                break;
            }
            if length + next_len > desired_length {
                // A chunk holding only heading(s) must still capture the content
                // they introduce, even when it overflows the budget.
                if !has_body {
                    if next_len > desired_length {
                        // Oversized block: split it; the heading leads its first piece.
                        let mut pieces = merge_node_inner(next, desired_length);
                        if let Some(first) = pieces.first_mut() {
                            first.start = start;
                        }
                        groups.extend(pieces);
                        emitted = true;
                    } else {
                        // Keep the first body block with its heading.
                        end = next.end_byte();
                    }
                    index += 1;
                }
                break;
            }
            end = next.end_byte();
            length += next_len;
            has_body |= !is_heading(next);
            index += 1;
        }
        if !emitted {
            groups.push(ChunkBoundary { start, end });
        }
    }
    groups
}

/// Recursively merge and split syntax-tree nodes into byte-offset chunks.
fn merge_node_inner(node: Node, desired_length: usize) -> Vec<ChunkBoundary> {
    let children: Vec<Node> = {
        let mut cursor = node.walk();
        node.children(&mut cursor).collect()
    };
    if children.is_empty() {
        return vec![ChunkBoundary { start: node.start_byte(), end: node.end_byte() }];
    }

    let mut groups: Vec<ChunkBoundary> = Vec::new();
    let mut index = 0;
    while index < children.len() {
        let child = children[index];
        let start = child.start_byte();
        let mut end = child.end_byte();
        let mut length = end - start;
        index += 1;

        // A single oversized node is split further by recursion.
        if length > desired_length {
            groups.extend(merge_node_inner(child, desired_length));
            continue;
        }
        // Extend the current group with following siblings that still fit.
        while index < children.len() {
            let next = children[index];
            let next_len = next.end_byte() - next.start_byte();
            if length + next_len > desired_length {
                break;
            }
            end = next.end_byte();
            length += next_len;
            index += 1;
        }
        groups.push(ChunkBoundary { start, end });
    }
    groups
}

/// Merge a sequence of adjacent chunks up to the desired length.
fn merge_adjacent_chunks(chunks: &[ChunkBoundary], desired_length: usize) -> Vec<ChunkBoundary> {
    let mut merged: Vec<ChunkBoundary> = Vec::new();
    let mut current = chunks[0];
    let mut current_length = current.len();

    for group in &chunks[1..] {
        let length = group.len();
        if current_length + length > desired_length {
            merged.push(current);
            current = *group;
            current_length = length;
            continue;
        }
        current.end = group.end;
        current_length += length;
    }
    merged.push(current);
    merged
}

/// Count newline characters in a slice of chars.
fn count_newlines(chars: &[char]) -> usize {
    chars.iter().filter(|c| **c == '\n').count()
}
