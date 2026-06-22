use kohiro::paths::Paths;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
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

fn seed_bare_with_push_script(bare: &Path, script: Option<&str>) {
    kohiro::git::ensure_bare(bare).unwrap();
    let work = tempdir().unwrap();
    git(&["init", "-q", "-b", "master"], Some(work.path()));
    git(&["config", "user.email", "t@example"], Some(work.path()));
    git(&["config", "user.name", "Tester"], Some(work.path()));
    std::fs::write(work.path().join("README.md"), "hello\n").unwrap();
    if let Some(script) = script {
        std::fs::create_dir_all(work.path().join(".ci")).unwrap();
        std::fs::write(work.path().join(".ci/push"), script).unwrap();
    }
    git(&["add", "."], Some(work.path()));
    git(&["commit", "-q", "-m", "seed"], Some(work.path()));
    git(
        &["remote", "add", "origin", bare.to_str().unwrap()],
        Some(work.path()),
    );
    git(&["push", "-q", "origin", "master"], Some(work.path()));
}

async fn wait_succeeded(db: &chilin::Db, id: i64) -> chilin::Job {
    for _ in 0..200 {
        let job = db.get(id).unwrap().unwrap();
        match job.status {
            chilin::JobStatus::Succeeded => return job,
            chilin::JobStatus::Failed | chilin::JobStatus::Cancelled => {
                panic!("job ended as {}", job.status)
            }
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("job {id} did not finish")
}

#[tokio::test]
async fn enqueue_push_runs_ci_script() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let bare = paths.repo_path("o", "r");
    seed_bare_with_push_script(&bare, Some("#!/bin/sh\necho ci-marker-$CI_SHA\n"));

    let db = Arc::new(chilin::Db::open(&paths.chilin_ci_db_path()).unwrap());
    db.migrate().unwrap();
    let id = kohiro::ci::enqueue_push(&db, &paths, "o", "r", &bare, Some("alice"))
        .await
        .unwrap()
        .expect("enqueued");

    let worker = tokio::spawn(chilin::run_worker(
        db.clone(),
        Arc::new(chilin::ShellRunner),
        Duration::from_millis(50),
    ));
    let job = wait_succeeded(&db, id).await;
    worker.abort();
    assert!(kohiro::ci::read_job_log(&job).contains("ci-marker-"));
}

#[tokio::test]
async fn enqueue_push_skips_repos_without_push_script() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let bare = paths.repo_path("o", "no_ci");
    seed_bare_with_push_script(&bare, None);

    let db = chilin::Db::open(&paths.chilin_ci_db_path()).unwrap();
    db.migrate().unwrap();
    let enqueued = kohiro::ci::enqueue_push(&db, &paths, "o", "no_ci", &bare, Some("alice"))
        .await
        .unwrap();
    assert!(enqueued.is_none());
}
