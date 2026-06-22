use kohiro::paths::Paths;
use kohiro::store::Store;
use kohiro::tickets::run_issues;
use std::sync::Arc;
use tempfile::tempdir;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn issues_commands_manage_myque_tasks() {
    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));
    let store = Store::open(&paths.db_path()).unwrap();
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    let agent_db = Arc::new(chilin::Db::open(&paths.chilin_agent_db_path()).unwrap());
    agent_db.migrate().unwrap();
    let owner = store.add_user("o", false).unwrap();
    store.ensure_repo(owner.id, "r").unwrap();
    let outsider = store.add_user("x", false).unwrap();

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "new", "o/r", "--title", "hello"]),
    );
    assert_eq!(code, 0, "{out}");
    let id = out.trim().to_owned();
    assert!(id.starts_with("task-"), "{id}");

    let tasks_dir = paths.myque_root("o", "r").join(".myque/tasks");
    let has_task_file = std::fs::read_dir(&tasks_dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "md")
    });
    assert!(has_task_file, "no task file in {}", tasks_dir.display());

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "list", "o/r"]),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("hello"), "{out}");

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "move", "o/r", &id, "ready"]),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(&format!("moved {id} -> ready")), "{out}");

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "show", "o/r", &id]),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("status: ready"), "{out}");
    assert!(out.contains("## Goal"), "{out}");

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "edit", "o/r", &id, "--body", "edited body"]),
    );
    assert_eq!(code, 0, "{out}");
    assert_eq!(out, format!("edited {id}\n"));

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "show", "o/r", &id]),
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("edited body"), "{out}");

    let (_out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&owner),
        &argv(&["issues", "move", "o/r", &id, "bogus"]),
    );
    assert_ne!(code, 0);

    let (out, code) = run_issues(
        &store,
        &paths,
        &agent_db,
        Some(&outsider),
        &argv(&["issues", "list", "o/r"]),
    );
    assert_eq!(code, 1);
    assert_eq!(out, "access denied\n");
}

#[test]
fn typed_helpers_create_and_update_tasks() {
    use kohiro::tickets::{create_with_body, get_task, list_tasks, set_body, set_status};
    use myque::Status;

    let dir = tempdir().unwrap();
    let paths = Paths::new(dir.path().join("data"));

    let created = create_with_body(
        &paths,
        "o",
        "r",
        "from tui".into(),
        Status::Backlog,
        Some("first body".into()),
    )
    .unwrap();
    let id = created.task.id.clone();

    let tasks = list_tasks(&paths, "o", "r").unwrap();
    assert!(
        tasks
            .iter()
            .any(|t| t.task.id == id && t.task.title == "from tui")
    );

    set_status(&paths, "o", "r", &id, Status::Ready).unwrap();
    let fetched = get_task(&paths, "o", "r", &id).unwrap();
    assert_eq!(fetched.task.status, Status::Ready);
    assert_eq!(fetched.body.trim(), "first body");

    let edited = set_body(&paths, "o", "r", &id, "edited\nbody".into()).unwrap();
    assert_eq!(edited.body.trim(), "edited\nbody");
}
