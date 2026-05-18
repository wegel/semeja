//! Tests for gitignore-aware file discovery.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use semeja::walk::{is_ignored, walk_files, GitIgnoreSpec, IgnoreSpec};
use tempfile::tempdir;

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().expect("path has a parent")).expect("create parent dirs");
    fs::write(path, "x = 1\n").expect("write file");
}

fn walk_relative(root: &Path) -> HashSet<String> {
    walk_files(root, &[".py".to_string()], None)
        .iter()
        .map(|p| p.strip_prefix(root).expect("under root").to_string_lossy().replace('\\', "/"))
        .collect()
}

fn run_case(files: &[&str], gitignore: Option<&str>, semejaignore: Option<&str>, expected: &[&str]) {
    let dir = tempdir().expect("temp dir");
    for rel in files {
        touch(&dir.path().join(rel));
    }
    if let Some(text) = gitignore {
        fs::write(dir.path().join(".gitignore"), text).expect("write gitignore");
    }
    if let Some(text) = semejaignore {
        fs::write(dir.path().join(".semejaignore"), text).expect("write semejaignore");
    }
    let expected: HashSet<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(walk_relative(dir.path()), expected);
}

#[test]
fn default_ignored_directories_are_skipped() {
    run_case(
        &["src/a.py", ".venv/lib/b.py", "node_modules/pkg/c.py", ".cache/uv/d.py"],
        None,
        None,
        &["src/a.py"],
    );
}

#[test]
fn root_gitignore_excludes_directories_and_files() {
    run_case(
        &["src/keep.py", "local/ignored.py", "generated.py"],
        Some("local/\ngenerated.py\n# comment"),
        None,
        &["src/keep.py"],
    );
}

#[test]
fn negation_re_includes_ignored_files() {
    run_case(&["out/a.py", "out/keep.py"], Some("out/*\n!out/keep.py\n"), None, &["out/keep.py"]);
}

#[test]
fn allow_list_gitignore_keeps_subdirs() {
    run_case(
        &["main.py", "internal/pkg/foo.py", "internal/pkg/bar.py"],
        Some("*\n!*/\n!*.py\n"),
        None,
        &["main.py", "internal/pkg/foo.py", "internal/pkg/bar.py"],
    );
}

#[test]
fn ignored_parent_negation_does_not_leak_gitignore() {
    run_case(&["out/deep/keep.py"], Some("out/*\n!out/deep/keep.py\n"), None, &[]);
}

#[test]
fn ignored_parent_negation_does_not_leak_semejaignore() {
    run_case(&["out/deep/keep.py"], None, Some("out/*\n!out/deep/keep.py\n"), &[]);
}

#[test]
fn explicit_file_negation_bypasses_extension_filter() {
    run_case(
        &["special.kjs", "other.kjs", "main.py"],
        None,
        Some("*.kjs\n!special.kjs\n"),
        &["main.py", "special.kjs"],
    );
}

#[test]
fn glob_negation_without_suffix_does_not_bypass_extension_filter() {
    run_case(&[".github/workflows/ci.yaml", "src/main.py"], None, Some("!.github/*\n"), &["src/main.py"]);
}

#[test]
fn directory_negation_does_not_bypass_extension_filter() {
    run_case(&["vendor/special.kjs", "vendor/main.py"], None, Some("*\n!vendor/\n"), &["vendor/main.py"]);
}

#[test]
fn ignored_directories_are_pruned() {
    let dir = tempdir().expect("temp dir");
    touch(&dir.path().join("src/a.py"));
    touch(&dir.path().join("node_modules/deep/deeper/b.js"));
    let visited = walk_files(dir.path(), &[".py".to_string(), ".js".to_string()], None);
    assert!(!visited.iter().any(|p| p.to_string_lossy().contains("node_modules")));
}

#[test]
fn is_ignored_skips_spec_with_unrelated_base() {
    let dir = tempdir().expect("temp dir");
    let project_a = dir.path().join("project_a");
    let project_b = dir.path().join("project_b");
    fs::create_dir(&project_a).expect("create project_a");
    fs::create_dir(&project_b).expect("create project_b");
    let target = project_a.join("keep.py");
    fs::write(&target, "x = 1\n").expect("write target");

    let unrelated = IgnoreSpec { base: project_b, spec: GitIgnoreSpec::from_lines(["*.py"]) };
    let (ignored, _) = is_ignored(&target, std::slice::from_ref(&unrelated));
    assert!(!ignored);

    let matching = IgnoreSpec { base: project_a, spec: GitIgnoreSpec::from_lines(["*.py"]) };
    let (ignored, _) = is_ignored(&target, &[unrelated, matching]);
    assert!(ignored);
}

#[test]
fn symlinks_are_skipped() {
    let dir = tempdir().expect("temp dir");
    let real_dir = dir.path().join("real_pkg/src");
    touch(&real_dir.join("mod.py"));

    let link_parent = dir.path().join("wrapper/src");
    fs::create_dir_all(&link_parent).expect("create link parent");
    std::os::unix::fs::symlink(&real_dir, link_parent.join("linked")).expect("dir symlink");

    touch(&dir.path().join("original.py"));
    std::os::unix::fs::symlink(dir.path().join("original.py"), dir.path().join("link_to_original.py"))
        .expect("file symlink");

    let found = walk_relative(dir.path());
    assert!(found.contains("real_pkg/src/mod.py"));
    assert!(found.contains("original.py"));
    assert!(!found.contains("wrapper/src/linked/mod.py"));
    assert!(!found.contains("link_to_original.py"));
}
