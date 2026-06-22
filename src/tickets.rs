use crate::agent_backend::ContainerBackend;
use crate::auth;
use crate::ci;
use crate::paths::Paths;
use crate::store::{Store, User};
use clap::{Parser, Subcommand};
use myque::{CreateTaskInput, Status, StoreError as MyqueStoreError, StoredTask, TaskStore};
use std::{fs, sync::Arc};

fn task_store(paths: &Paths, owner: &str, name: &str) -> TaskStore {
    TaskStore::new(paths.myque_root(owner, name))
}

fn ensure_initialized(store: &TaskStore) -> Result<(), MyqueStoreError> {
    if !store.base_dir().exists() {
        store.init(false)?;
    }
    Ok(())
}

pub fn list_tasks(
    paths: &Paths,
    owner: &str,
    name: &str,
) -> Result<Vec<StoredTask>, MyqueStoreError> {
    task_store(paths, owner, name).load_tasks()
}

pub fn get_task(
    paths: &Paths,
    owner: &str,
    name: &str,
    id: &str,
) -> Result<StoredTask, MyqueStoreError> {
    task_store(paths, owner, name).get_task(id)
}

pub fn create_titled(
    paths: &Paths,
    owner: &str,
    name: &str,
    title: String,
    status: Status,
) -> Result<StoredTask, MyqueStoreError> {
    create_with_body(paths, owner, name, title, status, None)
}

pub fn create_with_body(
    paths: &Paths,
    owner: &str,
    name: &str,
    title: String,
    status: Status,
    body: Option<String>,
) -> Result<StoredTask, MyqueStoreError> {
    let store = task_store(paths, owner, name);
    ensure_initialized(&store)?;
    let mut input = CreateTaskInput::new(title);
    input.status = status;
    input.body = body;
    store.create_task(input)
}

pub fn set_body(
    paths: &Paths,
    owner: &str,
    name: &str,
    id: &str,
    body: String,
) -> Result<StoredTask, MyqueStoreError> {
    let store = task_store(paths, owner, name);
    ensure_initialized(&store)?;
    let mut stored = store.get_task(id)?;
    stored.body = body;
    stored.task.updated_at = chrono::Utc::now().to_rfc3339();
    stored.frontmatter.updated_at = Some(stored.task.updated_at.clone());
    store.write_task(&stored)?;
    store.get_task(id)
}

pub fn set_status(
    paths: &Paths,
    owner: &str,
    name: &str,
    id: &str,
    status: Status,
) -> Result<StoredTask, MyqueStoreError> {
    let store = task_store(paths, owner, name);
    ensure_initialized(&store)?;
    store.update_status(id, status)
}

#[derive(Parser)]
#[command(name = "issues", no_binary_name = true)]
struct IssuesCli {
    #[command(subcommand)]
    cmd: IssuesCmd,
}

#[derive(Subcommand)]
enum IssuesCmd {
    List {
        repo: String,
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        repo: String,
        id: String,
    },
    New {
        repo: String,
        #[arg(long)]
        title: String,
        #[arg(long = "label")]
        labels: Vec<String>,
        #[arg(long, default_value = "backlog")]
        status: String,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
    },
    Edit {
        repo: String,
        id: String,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<String>,
    },
    Move {
        repo: String,
        id: String,
        status: String,
    },
    Board {
        repo: String,
    },
    Dispatch {
        repo: String,
    },
    Runs {
        repo: String,
    },
    Logs {
        repo: String,
        id: i64,
    },
}

impl IssuesCmd {
    fn repo(&self) -> &str {
        match self {
            Self::List { repo, .. }
            | Self::Show { repo, .. }
            | Self::New { repo, .. }
            | Self::Edit { repo, .. }
            | Self::Move { repo, .. }
            | Self::Board { repo }
            | Self::Dispatch { repo }
            | Self::Runs { repo }
            | Self::Logs { repo, .. } => repo,
        }
    }
}

pub fn run_issues(
    store: &Store,
    paths: &Paths,
    agent_db: &Arc<chilin::Db>,
    user: Option<&User>,
    argv: &[String],
) -> (String, i32) {
    let parse_args = if argv.first().is_some_and(|arg| arg == "issues") {
        &argv[1..]
    } else {
        argv
    };
    let parse_args = normalize_new_title(parse_args);
    let cli = match IssuesCli::try_parse_from(&parse_args) {
        Ok(cli) => cli,
        Err(err) => return (err.to_string(), 2),
    };

    let Some((owner, name)) = auth::parse_repo(cli.cmd.repo()) else {
        return ("invalid repository path\n".to_owned(), 1);
    };

    match cli.cmd {
        IssuesCmd::List { status, .. } => {
            if !auth::can_read(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let Some(status) = parse_optional_status(status) else {
                return (invalid_status_message(), 2);
            };
            let ticket_store = task_store(paths, &owner, &name);
            match ticket_store.load_tasks() {
                Ok(tasks) => {
                    let out = myque::render_task_list(
                        tasks
                            .iter()
                            .filter(|stored| {
                                status.as_ref().is_none_or(|s| stored.task.status == *s)
                            })
                            .map(|stored| &stored.task),
                    );
                    (out, 0)
                }
                Err(err) => store_error(err),
            }
        }
        IssuesCmd::Show { id, .. } => {
            if !auth::can_read(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let ticket_store = task_store(paths, &owner, &name);
            match ticket_store.get_task(&id) {
                Ok(stored) => {
                    let mut out = String::new();
                    out.push_str(&format!("id: {}\n", stored.task.id));
                    out.push_str(&format!("title: {}\n", stored.task.title));
                    out.push_str(&format!("status: {}\n", stored.task.status));
                    out.push_str(&format!("labels: {}\n", stored.task.labels.join(", ")));
                    out.push_str(&format!("created_at: {}\n", stored.task.created_at));
                    out.push_str(&format!("updated_at: {}\n\n", stored.task.updated_at));
                    out.push_str(&stored.body);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    (out, 0)
                }
                Err(MyqueStoreError::TaskNotFound(_)) => (format!("no such ticket: {id}\n"), 1),
                Err(err) => store_error(err),
            }
        }
        IssuesCmd::Board { .. } => {
            if !auth::can_read(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let ticket_store = task_store(paths, &owner, &name);
            match ticket_store.load_tasks() {
                Ok(tasks) => (
                    myque::render_board(tasks.iter().map(|stored| &stored.task)),
                    0,
                ),
                Err(err) => store_error(err),
            }
        }
        IssuesCmd::Dispatch { .. } => {
            if !auth::can_write(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let ticket_store = task_store(paths, &owner, &name);
            let config = match ticket_store.load_config() {
                Ok(config) => config,
                Err(err) => return store_error(err),
            };
            let mut reg = myque::BackendRegistry::with_builtins();
            reg.register(Box::new(ContainerBackend {
                agent_db: agent_db.clone(),
                paths: paths.clone(),
                owner: owner.clone(),
                name: name.clone(),
            }));
            let outcome = match myque::dispatch_with(&ticket_store, &config, false, &reg) {
                Ok(outcome) => outcome,
                Err(err) => return (format!("{err}\n"), 1),
            };
            let mut out = String::new();
            for r in outcome.started {
                out.push_str(&format!(
                    "started {} run={} backend={}\n",
                    r.task_id, r.id, r.backend
                ));
            }
            for (task_id, reason) in outcome.rejected {
                out.push_str(&format!(
                    "skipped {}: {}\n",
                    task_id,
                    myque::skip_reason_text(&reason)
                ));
            }
            if out.is_empty() {
                out.push_str("nothing dispatched\n");
            }
            (out, 0)
        }
        IssuesCmd::Runs { .. } => {
            if !auth::can_read(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            match agent_db.list(&format!("{owner}/{name}"), 20) {
                Ok(jobs) => (ci::format_job_table(&jobs), 0),
                Err(err) => (format!("{err}\n"), 1),
            }
        }
        IssuesCmd::Logs { id, .. } => {
            if !auth::can_read(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let namespace = format!("{owner}/{name}");
            match agent_db.get(id) {
                Ok(Some(j)) if j.namespace == namespace => (ci::read_job_log(&j), 0),
                Ok(_) => ("no such run\n".to_owned(), 1),
                Err(err) => (format!("{err}\n"), 1),
            }
        }
        IssuesCmd::New {
            title,
            labels,
            status,
            agent,
            body,
            body_file,
            ..
        } => {
            if !auth::can_write(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let Some(status) = Status::parse_str(&status) else {
                return (invalid_status_message(), 2);
            };
            let body = match resolve_body_arg(body, body_file) {
                Ok(body) => body,
                Err(message) => return (message, 2),
            };
            let ticket_store = task_store(paths, &owner, &name);
            if let Err(err) = ensure_initialized(&ticket_store) {
                return store_error(err);
            }
            let mut input = CreateTaskInput::new(title);
            input.status = status;
            input.labels = labels;
            input.body = body;
            if let Some(agent) = agent {
                input.agent = agent;
            }
            match ticket_store.create_task(input) {
                Ok(stored) => (format!("{}\n", stored.task.id), 0),
                Err(err) => store_error(err),
            }
        }
        IssuesCmd::Edit {
            id,
            body,
            body_file,
            ..
        } => {
            if !auth::can_write(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let Some(body) = (match resolve_body_arg(body, body_file) {
                Ok(body) => body,
                Err(message) => return (message, 2),
            }) else {
                return (
                    "nothing to edit; pass --body or --body-file\n".to_owned(),
                    2,
                );
            };
            match set_body(paths, &owner, &name, &id, body) {
                Ok(_) => (format!("edited {id}\n"), 0),
                Err(MyqueStoreError::TaskNotFound(_)) => (format!("no such ticket: {id}\n"), 1),
                Err(err) => store_error(err),
            }
        }
        IssuesCmd::Move { id, status, .. } => {
            if !auth::can_write(store, user, &owner, &name) {
                return ("access denied\n".to_owned(), 1);
            }
            let Some(status) = Status::parse_str(&status) else {
                return (invalid_status_message(), 2);
            };
            let ticket_store = task_store(paths, &owner, &name);
            if let Err(err) = ensure_initialized(&ticket_store) {
                return store_error(err);
            }
            match ticket_store.update_status(&id, status.clone()) {
                Ok(_) => (format!("moved {id} -> {status}\n"), 0),
                Err(MyqueStoreError::TaskNotFound(_)) => (format!("no such ticket: {id}\n"), 1),
                Err(err) => store_error(err),
            }
        }
    }
}

fn resolve_body_arg(
    body: Option<String>,
    body_file: Option<String>,
) -> Result<Option<String>, String> {
    match (body, body_file) {
        (Some(_), Some(_)) => Err("pass only one of --body or --body-file\n".to_owned()),
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) if path == "-" => {
            let mut body = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut body)
                .map_err(|err| format!("failed to read body from stdin: {err}\n"))?;
            Ok(Some(body))
        }
        (None, Some(path)) => fs::read_to_string(&path)
            .map(Some)
            .map_err(|err| format!("failed to read body file {path}: {err}\n")),
        (None, None) => Ok(None),
    }
}

fn normalize_new_title(args: &[String]) -> Vec<String> {
    if args.first().is_none_or(|cmd| cmd != "new") {
        return args.to_vec();
    }

    let mut normalized = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--title" {
            normalized.push(args[i].clone());
            i += 1;
            let mut title = Vec::new();
            while i < args.len() && !args[i].starts_with("--") {
                title.push(args[i].clone());
                i += 1;
            }
            if !title.is_empty() {
                normalized.push(title.join(" "));
            }
            continue;
        }
        normalized.push(args[i].clone());
        i += 1;
    }
    normalized
}

fn parse_optional_status(status: Option<String>) -> Option<Option<Status>> {
    status
        .map(|status| Status::parse_str(&status).map(Some))
        .unwrap_or(Some(None))
}

fn invalid_status_message() -> String {
    format!(
        "unknown status; expected one of: {}\n",
        myque::Status::all().join(", ")
    )
}

fn store_error(err: MyqueStoreError) -> (String, i32) {
    (format!("{err}\n"), 1)
}
