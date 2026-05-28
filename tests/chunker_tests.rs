//! Tests for source-code chunking.

use semeja::chunk::chunk_source;
use semeja::chunk::{chunk, chunk_lines, has_grammar, ChunkBoundary};
use semeja::types::Chunk;

/// Every language with a bundled tree-sitter grammar.
const BUNDLED_LANGUAGES: &[&str] = &[
    "python", "javascript", "typescript", "tsx", "rust", "go", "java", "c", "cpp", "csharp",
    "ruby", "php", "bash", "html", "css", "json", "scala", "haskell", "ocaml", "ocaml_interface",
    "elixir", "lua", "kotlin", "swift", "r", "sql", "dart", "perl", "objc", "julia", "groovy",
    "powershell", "markdown", "yaml", "toml", "erlang", "zig", "nix", "solidity", "svelte", "ada",
    "cmake", "make", "elm", "hcl", "gleam", "scheme", "commonlisp", "fsharp", "fsharp_signature",
    "verilog", "systemverilog", "cuda", "graphql", "proto",
];

#[test]
fn chunk_lines_empty_input_yields_nothing() {
    assert!(chunk_lines("", 23).is_empty());

    let content: String = (0..10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let chunks = chunk_lines(&content, 10);
    assert!(chunks.len() >= 2);
    assert_eq!(chunks[0].start, 0);
}

#[test]
fn chunk_source_returns_nothing_for_whitespace() {
    assert!(chunk_source("   \n\n", "foo.py", Some("python")).is_empty());
}

#[test]
fn chunk_source_falls_back_to_lines_for_unknown_language() {
    assert_eq!(
        chunk_source("hello", "foo.loki", Some("loki")),
        vec![Chunk {
            content: "hello".to_string(),
            file_path: "foo.loki".to_string(),
            start_line: 1,
            end_line: 1,
            language: Some("loki".to_string()),
        }]
    );
    assert_eq!(
        chunk_source("1+1=3", "foo.json", None),
        vec![Chunk {
            content: "1+1=3".to_string(),
            file_path: "foo.json".to_string(),
            start_line: 1,
            end_line: 1,
            language: None,
        }]
    );
}

#[test]
fn core_chunk_returns_nothing_for_whitespace() {
    assert!(chunk("   \n", "python", 100).is_empty());
}

#[test]
fn core_chunk_lines_merges_small_lines() {
    // Each line is 2 chars ("a\n"); desired_length=10 allows merging up to 5 lines.
    let chunks = chunk_lines("a\nb\nc\nd\ne\nf\n", 10);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0], ChunkBoundary { start: 0, end: 10 });
    assert_eq!(chunks[1], ChunkBoundary { start: 10, end: 12 });
}

#[test]
fn core_chunk_recursively_splits_and_breaks() {
    let code = "x = 1\ndef foo():\n    a = 1\n    b = 2\n    c = 3\ny = 2\n";
    let chunks = chunk(code, "python", 10);
    assert!(chunks.len() >= 3);
    assert_eq!(chunks[0].start, 0);
    for (i, c) in chunks.iter().enumerate() {
        assert!(c.start < c.end && c.end <= code.len());
        if i > 0 {
            assert!(c.start >= chunks[i - 1].end);
        }
    }
}

#[test]
fn bundled_grammars_load_for_typical_languages() {
    for language in BUNDLED_LANGUAGES {
        assert!(has_grammar(language), "grammar for {language} should load");
    }
    assert!(!has_grammar("brainfuck"));
}

#[test]
fn non_python_languages_chunk_along_syntax() {
    let code = "fn alpha() {\n    let a = 1;\n}\n\nfn beta() {\n    let b = 2;\n}\n";
    let chunks = chunk(code, "rust", 10);
    assert!(chunks.len() >= 2);
    for (i, c) in chunks.iter().enumerate() {
        assert!(c.start < c.end && c.end <= code.len());
        if i > 0 {
            assert!(c.start >= chunks[i - 1].end);
        }
    }
}

#[test]
fn markdown_chunks_lead_with_their_heading() {
    let md = "# Top\n\n## Alpha\n\nalpha body text\n\n## Beta\n\nbeta body text\n";
    let boundaries = chunk(md, "markdown", 24);
    assert!(boundaries.len() >= 2, "small budget should split into sections");

    let chars: Vec<char> = md.chars().collect();
    let texts: Vec<String> =
        boundaries.iter().map(|b| chars[b.start..b.end].iter().collect()).collect();

    // Every chunk begins at a heading — none drift onto the previous section.
    for text in &texts {
        assert!(text.trim_start().starts_with('#'), "chunk must lead with a heading: {text:?}");
    }
    // The second section's heading leads its own chunk, with its body.
    assert!(texts.iter().any(|t| t.trim_start().starts_with("## Beta") && t.contains("beta body")));
}

#[test]
fn core_chunk_handles_leaf_node_exceeding_desired_length() {
    let long_var = "x".repeat(100);
    let code = format!("{long_var} = 1\n");
    let chunks = chunk(&code, "python", 50);
    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].start, 0);
    for c in &chunks {
        assert!(c.start < c.end && c.end <= code.len());
    }
}
