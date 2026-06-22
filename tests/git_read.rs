use kohiro::git;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

#[tokio::test]
async fn reads_commits_tree_and_blob() {
    let dir = tempdir().unwrap();
    let repo = dir.path();
    run(repo, &["init", "-q"]);
    run(repo, &["config", "user.email", "t@example"]);
    run(repo, &["config", "user.name", "Tester"]);
    std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
    std::fs::create_dir(repo.join("dir")).unwrap();
    std::fs::write(repo.join("dir/b.txt"), "world\n").unwrap();
    run(repo, &["add", "."]);
    run(repo, &["commit", "-q", "-m", "initial commit"]);

    let commits = git::commit_log(repo, 50).await.unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "initial commit");
    assert_eq!(commits[0].short_hash.len(), 7);
    assert_eq!(commits[0].date.len(), 10);

    // Root tree: directory first, then file.
    let tree = git::list_tree(repo, "").await.unwrap();
    let entries: Vec<(&str, bool)> = tree.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
    assert_eq!(entries, vec![("dir", true), ("a.txt", false)]);

    // Subdirectory tree.
    let sub = git::list_tree(repo, "dir").await.unwrap();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].name, "b.txt");
    assert!(!sub[0].is_dir);

    // Blob content.
    let blob = git::read_blob(repo, "a.txt").await.unwrap();
    assert_eq!(blob.text, "hello\n");
    assert!(blob.note.is_none());
}

#[tokio::test]
async fn empty_repo_reads_are_empty_not_errors() {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("bare.git");
    let status = Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&repo)
        .status()
        .unwrap();
    assert!(status.success());

    assert!(git::commit_log(&repo, 50).await.unwrap().is_empty());
    assert!(git::list_tree(&repo, "").await.unwrap().is_empty());
}
