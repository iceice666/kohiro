use kohiro::agent_backend::ContainerBackend;
use kohiro::paths::Paths;
use myque::{CreateTaskInput, Status, TaskStore};
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

fn seed_bare(bare: &Path) {
    kohiro::git::ensure_bare(bare).unwrap();
    let work = tempdir().unwrap();
    git(&["init", "-q", "-b", "master"], Some(work.path()));
    git(&["config", "user.email", "t@example"], Some(work.path()));
    git(&["config", "user.name", "Tester"], Some(work.path()));
    std::fs::write(work.path().join("README.md"), "hello\n").unwrap();
    git(&["add", "."], Some(work.path()));
    git(&["commit", "-q", "-m", "seed"], Some(work.path()));
    git(
        &["remote", "add", "origin", bare.to_str().unwrap()],
        Some(work.path()),
    );
    git(&["push", "-q", "origin", "master"], Some(work.path()));
}

#[tokio::test]
async fn container_backend_dispatches_myque_task_to_chilin() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    seed_bare(&paths.repo_path("o", "r"));

    let store = TaskStore::new(paths.myque_root("o", "r"));
    store.init(false).unwrap();
    std::fs::write(
        store.config_path(),
        r#"default_backend = "container"

[policy]
require_allowed_label = false
require_acceptance_section = false
require_allowed_auto_dispatch = false

[backends.container]
kind = "container"

[agents.coder]
backend = "container"
command = "sh -c 'cat task.md > seen.txt'"
"#,
    )
    .unwrap();

    let mut input = CreateTaskInput::new("do work");
    input.status = Status::Ready;
    input.agent = "coder".into();
    input.backend = "container".into();
    input.body = Some(
        "## Goal\nDo work.\n\n## Context\nFixture.\n\n## Constraints\nNone.\n\n## Acceptance\nTask file is visible.\n".into(),
    );
    let task = store.create_task(input).unwrap();
    let config = store.load_config().unwrap();

    let mut reg = myque::BackendRegistry::with_builtins();
    reg.register(Box::new(ContainerBackend {
        runner: Arc::new(chilin::ShellRunner),
        paths: paths.clone(),
        owner: "o".into(),
        name: "r".into(),
    }));
    let outcome = myque::dispatch_with(&store, &config, false, &reg).unwrap();
    assert_eq!(outcome.started.len(), 1, "{outcome:?}");
    assert!(outcome.rejected.is_empty(), "{outcome:?}");
    assert_eq!(outcome.started[0].status, "done");

    let seen = std::fs::read_to_string(
        paths
            .agent_work_dir("o", "r")
            .join(&outcome.started[0].id)
            .join("seen.txt"),
    )
    .unwrap();
    assert!(seen.contains(&task.task.id), "{seen}");
    let updated = store.get_task(&task.task.id).unwrap();
    assert_eq!(updated.task.status, Status::Done);
    assert!(updated.task.completed_at.is_some());
}
