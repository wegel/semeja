//! Tests for cloning and indexing git repositories.

use std::path::Path;
use std::process::Command;

use semeja::embed::MockEncoder;
use semeja::SemejaIndex;
use tempfile::tempdir;

fn model() -> Box<MockEncoder> {
    Box::new(MockEncoder::new())
}

fn git(args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "t@t.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "t@t.com")
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) {
    std::fs::write(repo.join(name), content).expect("write file");
    let repo = repo.to_str().expect("utf-8 path");
    git(&["-C", repo, "add", name]);
    git(&["-C", repo, "commit", "-m", message]);
}

#[test]
fn from_git_indexes_local_repo_with_relative_paths() {
    let dir = tempdir().expect("temp dir");
    git(&["init", dir.path().to_str().unwrap()]);
    commit_file(dir.path(), "main.py", "def hello():\n    return 'hello'\n", "add file");

    let index = SemejaIndex::from_git(dir.path().to_str().unwrap(), None, Some(model()), None, false)
        .expect("from_git");
    assert!(index.stats().indexed_files >= 1);
    assert!(index.stats().total_chunks > 0);
    assert!(index.chunks.iter().any(|c| c.file_path.contains("main.py")));
    assert!(index.chunks.iter().all(|c| !Path::new(&c.file_path).is_absolute()));
}

#[test]
fn from_git_checks_out_requested_branch() {
    let dir = tempdir().expect("temp dir");
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).expect("create repo dir");
    git(&["init", repo.to_str().unwrap()]);
    commit_file(&repo, "main.py", "def on_main(): pass\n", "main");
    git(&["-C", repo.to_str().unwrap(), "checkout", "-b", "feature"]);
    commit_file(&repo, "feature.py", "def on_feature(): pass\n", "feature");

    let index =
        SemejaIndex::from_git(repo.to_str().unwrap(), Some("feature"), Some(model()), None, false)
            .expect("from_git");
    assert!(index.chunks.iter().any(|c| c.file_path.ends_with("feature.py")));
}

#[test]
fn from_git_raises_on_clone_failure() {
    let result = SemejaIndex::from_git(
        "/nonexistent/path/that/does/not/exist",
        None,
        Some(model()),
        None,
        false,
    );
    let err = result.err().expect("clone should fail");
    assert!(err.to_string().contains("git clone failed"));
}
