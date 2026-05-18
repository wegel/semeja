//! Tests for language detection and extension selection.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use semeja::lang::{detect_language, get_extensions, DOC_LANGUAGES};
use semeja::lang_table::EXTENSION_TO_LANGUAGE;

fn extension_map() -> HashMap<&'static str, &'static str> {
    EXTENSION_TO_LANGUAGE.iter().copied().collect()
}

#[test]
fn detect_language_maps_known_extensions() {
    assert_eq!(detect_language(Path::new("a.py")).as_deref(), Some("python"));
    assert_eq!(detect_language(Path::new("b.js")).as_deref(), Some("javascript"));
    assert_eq!(detect_language(Path::new("c.txt")), None);
}

#[test]
fn get_extensions_splits_doc_and_code_languages() {
    let all: HashSet<String> = get_extensions(true, None).into_iter().collect();
    let without_doc: HashSet<String> = get_extensions(false, None).into_iter().collect();
    let doc_languages: HashSet<&str> = DOC_LANGUAGES.iter().copied().collect();
    let map = extension_map();

    for extension in all.difference(&without_doc) {
        assert!(doc_languages.contains(map[extension.as_str()]));
    }
    for extension in &without_doc {
        assert!(!doc_languages.contains(map[extension.as_str()]));
    }
}

#[test]
fn get_extensions_includes_additional_extensions() {
    let all: HashSet<String> = get_extensions(true, None).into_iter().collect();
    let kjs = vec![".kjs".to_string()];
    let all_extra: HashSet<String> = get_extensions(true, Some(&kjs)).into_iter().collect();
    let mut expected = all.clone();
    expected.insert(".kjs".to_string());
    assert_eq!(all_extra, expected);

    let without: HashSet<String> = get_extensions(false, None).into_iter().collect();
    let without_extra: HashSet<String> = get_extensions(false, Some(&kjs)).into_iter().collect();
    let mut expected = without.clone();
    expected.insert(".kjs".to_string());
    assert_eq!(without_extra, expected);

    // Adding an already-present extension is a no-op.
    let py = vec![".py".to_string()];
    let without_py: HashSet<String> = get_extensions(false, Some(&py)).into_iter().collect();
    assert_eq!(without_py, without);
}
