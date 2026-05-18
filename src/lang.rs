//! File-extension to language detection and the supported-extension set.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use crate::lang_table::EXTENSION_TO_LANGUAGE;

static EXT_MAP: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| EXTENSION_TO_LANGUAGE.iter().copied().collect());

static ALL_LANGUAGES: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| EXTENSION_TO_LANGUAGE.iter().map(|(_, lang)| *lang).collect());

static DOC_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| DOC_LANGUAGES.iter().copied().collect());

static LANGUAGE_TO_EXTENSIONS: LazyLock<HashMap<&'static str, Vec<&'static str>>> =
    LazyLock::new(|| {
        let mut inv: HashMap<&str, Vec<&str>> = HashMap::new();
        for (ext, lang) in EXTENSION_TO_LANGUAGE {
            inv.entry(lang).or_default().push(ext);
        }
        inv
    });

/// The set of every known code/doc language.
pub fn all_languages() -> &'static HashSet<&'static str> {
    &ALL_LANGUAGES
}

/// Detect the language of a file from its extension.
pub fn detect_language(file_name: &Path) -> Option<String> {
    let ext = file_name.extension()?.to_str()?.to_lowercase();
    EXT_MAP.get(format!(".{ext}").as_str()).map(|lang| lang.to_string())
}

/// Return the sorted set of file extensions to index.
///
/// Documentation languages are excluded unless `include_text_files` is set;
/// `extra` extensions are always added.
pub fn get_extensions(include_text_files: bool, extra: Option<&[String]>) -> Vec<String> {
    let mut extensions: HashSet<String> = HashSet::new();
    for lang in ALL_LANGUAGES.iter() {
        if !include_text_files && DOC_SET.contains(lang) {
            continue;
        }
        if let Some(exts) = LANGUAGE_TO_EXTENSIONS.get(lang) {
            extensions.extend(exts.iter().map(|e| e.to_string()));
        }
    }
    if let Some(extra) = extra {
        extensions.extend(extra.iter().cloned());
    }
    let mut sorted: Vec<String> = extensions.into_iter().collect();
    sorted.sort();
    sorted
}

/// Languages treated as prose/documentation rather than code.
pub const DOC_LANGUAGES: &[&str] = &[
    "asciidoc",
    "beancount",
    "bibtex",
    "capnp",
    "cedarschema",
    "comment",
    "cooklang",
    "cpon",
    "csv",
    "desktop",
    "devicetree",
    "diff",
    "djot",
    "doxygen",
    "dtd",
    "editorconfig",
    "ebnf",
    "git_config",
    "gitattributes",
    "gitcommit",
    "gitignore",
    "godot_resource",
    "gomod",
    "gosum",
    "gowork",
    "gpg",
    "hjson",
    "hocon",
    "html",
    "ini",
    "javadoc",
    "jsdoc",
    "json",
    "json5",
    "kdl",
    "latex",
    "ledger",
    "luadoc",
    "markdown",
    "markdown_inline",
    "mermaid",
    "norg",
    "norg_meta",
    "org",
    "pem",
    "pgn",
    "phpdoc",
    "po",
    "properties",
    "proto",
    "psv",
    "requirements",
    "ron",
    "rst",
    "rtf",
    "smithy",
    "ssh_config",
    "textproto",
    "thrift",
    "todotxt",
    "toml",
    "tsv",
    "turtle",
    "typespec",
    "vimdoc",
    "wit",
    "xcompose",
    "xml",
    "yaml",
    "ziggy_schema",
];
