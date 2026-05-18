//! Recursive file discovery honouring `.gitignore` and `.semejaignore` rules.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Directory names always skipped during a walk.
const DEFAULT_IGNORED_DIRS: [&str; 17] = [
    ".git/",
    ".hg/",
    ".svn/",
    "__pycache__/",
    "node_modules/",
    ".venv/",
    "venv/",
    ".tox/",
    ".mypy_cache/",
    ".pytest_cache/",
    ".ruff_cache/",
    ".cache/",
    ".semeja/",
    ".next/",
    "dist/",
    "build/",
    ".eggs/",
];

// --- Public types ---

/// A compiled set of gitignore-style patterns.
pub struct GitIgnoreSpec {
    patterns: Vec<Pattern>,
}

/// A [`GitIgnoreSpec`] together with the directory it is rooted at.
pub struct IgnoreSpec {
    /// The directory the spec's patterns are relative to.
    pub base: PathBuf,
    /// The compiled patterns.
    pub spec: GitIgnoreSpec,
}

// --- Public API ---

impl GitIgnoreSpec {
    /// Compile a spec from raw gitignore lines.
    pub fn from_lines<I, S>(lines: I) -> GitIgnoreSpec
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let patterns = lines.into_iter().filter_map(|l| compile_pattern(l.as_ref())).collect();
        GitIgnoreSpec { patterns }
    }
}

/// Yield files under `root` matching `extensions`, skipping ignored paths.
pub fn walk_files(root: &Path, extensions: &[String], ignore: Option<&[String]>) -> Vec<PathBuf> {
    let extension_set: HashSet<String> = extensions.iter().cloned().collect();

    let mut dir_patterns: Vec<String> = DEFAULT_IGNORED_DIRS.iter().map(|s| s.to_string()).collect();
    dir_patterns.sort();
    if let Some(extra) = ignore {
        dir_patterns.extend(extra.iter().cloned());
    }
    let base = IgnoreSpec {
        base: root.to_path_buf(),
        spec: GitIgnoreSpec::from_lines(dir_patterns),
    };

    let mut out = Vec::new();
    walk(root, &[base], &extension_set, &mut out);
    out
}

/// Check whether `path` is ignored by any spec; returns `(ignored, found)`.
///
/// `found` marks a negated file pattern with an extension suffix, which is
/// allowed to bypass the extension filter.
pub fn is_ignored(path: &Path, specs: &[IgnoreSpec]) -> (bool, bool) {
    let is_dir = path.is_dir();
    let mut ignored = false;
    let mut found = false;

    for spec in specs {
        let relative = match path.strip_prefix(&spec.base) {
            Ok(rel) => rel,
            Err(_) => continue,
        };
        let mut relative_str = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if is_dir {
            relative_str.push('/');
        }

        for pattern in &spec.spec.patterns {
            if pattern.regex.is_match(&relative_str) {
                ignored = pattern.include;
                found = !ignored && pattern.has_suffix;
            }
        }
    }
    (ignored, found)
}

// --- Private types and helpers ---

struct Pattern {
    /// True for a normal ignore pattern, false for a `!` negation.
    include: bool,
    regex: Regex,
    /// True when the pattern's filename carries an extension suffix.
    has_suffix: bool,
}

/// Walk one directory, recursing into non-ignored subdirectories.
fn walk(directory: &Path, inherited: &[IgnoreSpec], extensions: &HashSet<String>, out: &mut Vec<PathBuf>) {
    let mut specs: Vec<IgnoreSpec> = inherited.iter().map(clone_spec).collect();
    if let Some(spec) = load_ignore_for_dir(directory) {
        specs.push(IgnoreSpec { base: directory.to_path_buf(), spec });
    }
    walk_entries(directory, &specs, extensions, out);
}

fn clone_spec(spec: &IgnoreSpec) -> IgnoreSpec {
    IgnoreSpec {
        base: spec.base.clone(),
        spec: GitIgnoreSpec {
            patterns: spec
                .spec
                .patterns
                .iter()
                .map(|p| Pattern {
                    include: p.include,
                    regex: p.regex.clone(),
                    has_suffix: p.has_suffix,
                })
                .collect(),
        },
    }
}

/// List, filter, and recurse over the entries of one directory.
fn walk_entries(directory: &Path, specs: &[IgnoreSpec], extensions: &HashSet<String>, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = match fs::read_dir(directory) {
        Ok(reader) => reader.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => return,
    };
    entries.sort();

    for item in entries {
        if fs::symlink_metadata(&item).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
            continue;
        }
        let (ignored, found) = is_ignored(&item, specs);
        if ignored {
            continue;
        }
        if item.is_dir() {
            walk(&item, specs, extensions, out);
        } else if item.is_file() && (found || has_indexed_extension(&item, extensions)) {
            out.push(item);
        }
    }
}

/// Return true if the file's lowercased extension is in the indexed set.
fn has_indexed_extension(path: &Path, extensions: &HashSet<String>) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => extensions.contains(&format!(".{}", ext.to_lowercase())),
        None => false,
    }
}

/// Load and combine `.gitignore` and `.semejaignore` for a directory.
fn load_ignore_for_dir(directory: &Path) -> Option<GitIgnoreSpec> {
    let mut lines: Vec<String> = Vec::new();
    for name in [".gitignore", ".semejaignore"] {
        if let Ok(text) = fs::read_to_string(directory.join(name)) {
            lines.extend(text.lines().map(str::to_string));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(GitIgnoreSpec::from_lines(lines))
    }
}

/// Compile one gitignore line into a [`Pattern`], or `None` for comments.
fn compile_pattern(raw: &str) -> Option<Pattern> {
    let line = raw.trim_end();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let mut include = true;
    let mut body = line;
    if let Some(rest) = body.strip_prefix('!') {
        include = false;
        body = rest;
    }
    if body.is_empty() {
        return None;
    }

    let dir_only = body.ends_with('/');
    let core = body.trim_end_matches('/');
    let anchored_lead = core.starts_with('/');
    let core = core.trim_start_matches('/');
    if core.is_empty() {
        return None;
    }
    let anchored = anchored_lead || core.contains('/');

    let regex_body: String =
        core.split('/').map(translate_segment).collect::<Vec<_>>().join("/");
    let prefix = if anchored { "" } else { "(?:.*/)?" };
    let suffix = if dir_only { "/" } else { "(?:/|$)" };
    let regex = Regex::new(&format!("^{prefix}{regex_body}{suffix}")).ok()?;

    Some(Pattern { include, regex, has_suffix: name_has_suffix(line.trim_end_matches('/')) })
}

/// Translate one glob path segment into a regex fragment.
fn translate_segment(segment: &str) -> String {
    if segment == "*" {
        // A whole-segment `*` matches a non-empty path component.
        return "[^/]+".to_string();
    }
    let mut out = String::new();
    for ch in segment.chars() {
        match ch {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            _ => out.push_str(&regex::escape(&ch.to_string())),
        }
    }
    out
}

/// Return true if the final path component of `pattern` has an extension.
fn name_has_suffix(pattern: &str) -> bool {
    let name = pattern.rsplit('/').next().unwrap_or(pattern);
    match name.rfind('.') {
        Some(dot) => dot > 0 && dot < name.len() - 1,
        None => false,
    }
}
