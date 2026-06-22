use kohiro::paths::Paths;
use myque::{Status, TaskStore};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;

fn git(args: &[&str], cwd: Option<&Path>) {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_bare_with_push_template(bare: &Path, template: Option<&str>) {
    kohiro::git::ensure_bare(bare).unwrap();
    let work = tempdir().unwrap();
    git(&["init", "-q", "-b", "master"], Some(work.path()));
    git(&["config", "user.email", "t@example"], Some(work.path()));
    git(&["config", "user.name", "Tester"], Some(work.path()));
    std::fs::write(work.path().join("README.md"), "hello\n").unwrap();
    if let Some(template) = template {
        std::fs::create_dir_all(work.path().join(".ci")).unwrap();
        std::fs::write(work.path().join(".ci/push"), template).unwrap();
    }
    git(&["add", "."], Some(work.path()));
    git(&["commit", "-q", "-m", "seed"], Some(work.path()));
    git(
        &["remote", "add", "origin", bare.to_str().unwrap()],
        Some(work.path()),
    );
    git(&["push", "-q", "origin", "master"], Some(work.path()));
}

fn ci_template() -> String {
    r#"+++
title = "CI push"
priority = 1
order = 100
labels = ["safe-auto", "ci", "ci:push"]
agent = "ci"
backend = "chilin"
depends_on = []
max_attempts = 2
+++

## Goal

Run push CI for {repo}.

## Context

Commit {sha} was pushed by {pusher}.

## Constraints

Run in the checked out pushed commit.

## Acceptance

The configured command exits successfully.

## Chilin

```toml
command = ["sh", "-c", "echo ci-marker-$CI_SHA > ci.out"]
env = [["CI_REPO", "{repo}"], ["CI_SHA", "{sha}"], ["CI_PUSHER", "{pusher}"]]
log_path = "{log_path}"

[mount]
source = "{workdir}"
target = "/repo"
readonly = false
```
"#
    .to_owned()
}

#[test]
fn enqueue_push_creates_and_dispatches_ci_ticket() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let bare = paths.repo_path("o", "r");
    seed_bare_with_push_template(&bare, Some(&ci_template()));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let task = rt
        .block_on(kohiro::ci::enqueue_push(
            &paths,
            "o",
            "r",
            &bare,
            Some("alice"),
        ))
        .unwrap()
        .expect("enqueued");

    assert_eq!(task.task.status, Status::Ready);
    assert_eq!(task.task.backend, "chilin");
    assert!(task.body.contains("## Chilin"));
    assert!(task.body.contains("CI_PUSHER"));
    assert!(task.body.contains("alice"));

    let ordinary_store = TaskStore::new(paths.myque_root("o", "r"));
    let mut ordinary = myque::CreateTaskInput::new("ordinary ready task");
    ordinary.status = Status::Ready;
    ordinary.labels = vec!["safe-auto".to_owned()];
    ordinary.allowed_auto_dispatch = true;
    ordinary.body = Some(
        "## Goal\nDo it.\n\n## Context\nFixture.\n\n## Constraints\nNone.\n\n## Acceptance\nDone.\n"
            .to_owned(),
    );
    let ordinary = ordinary_store.create_task(ordinary).unwrap();

    let outcome =
        kohiro::ci::dispatch_ready(&paths, "o", "r", Arc::new(chilin::ShellRunner)).unwrap();
    assert_eq!(outcome.started.len(), 1, "{outcome:?}");
    assert_eq!(outcome.started[0].task_id, task.task.id);

    let ci_task = ordinary_store.get_task(&task.task.id).unwrap();
    assert_eq!(ci_task.task.status, Status::Done);
    let ordinary = ordinary_store.get_task(&ordinary.task.id).unwrap();
    assert_eq!(ordinary.task.status, Status::Ready);

    let jobs = kohiro::ci::list_jobs(&paths, "o", "r", 20).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, chilin::JobStatus::Succeeded);
    assert!(kohiro::ci::read_job_log(&jobs[0]).is_empty());
    let out = std::fs::read_to_string(
        jobs[0]
            .mount
            .as_ref()
            .expect("mount exists")
            .source
            .join("ci.out"),
    )
    .unwrap();
    assert!(out.contains("ci-marker-"), "{out}");
}

#[tokio::test]
async fn enqueue_push_skips_repos_without_push_script() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let bare = paths.repo_path("o", "no_ci");
    seed_bare_with_push_template(&bare, None);

    let enqueued = kohiro::ci::enqueue_push(&paths, "o", "no_ci", &bare, Some("alice"))
        .await
        .unwrap();
    assert!(enqueued.is_none());
}
