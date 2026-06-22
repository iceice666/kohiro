use crate::auth;
use crate::paths::Paths;
use crate::store::{Store, User};
use clap::{Parser, Subcommand};
use myque::{CreateTaskInput, Status, StoreError as MyqueStoreError, TaskStore};

fn task_store(paths: &Paths, owner: &str, name: &str) -> TaskStore {
    TaskStore::new(paths.myque_root(owner, name))
}

fn ensure_initialized(store: &TaskStore) -> Result<(), MyqueStoreError> {
    if !store.base_dir().exists() {
        store.init(false)?;
    }
    Ok(())
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
    },
    Move {
        repo: String,
        id: String,
        status: String,
    },
    Board {
        repo: String,
    },
}

impl IssuesCmd {
    fn repo(&self) -> &str {
        match self {
            Self::List { repo, .. }
            | Self::Show { repo, .. }
            | Self::New { repo, .. }
            | Self::Move { repo, .. }
            | Self::Board { repo } => repo,
        }
    }
}

pub fn run_issues(
    store: &Store,
    paths: &Paths,
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
        IssuesCmd::New {
            title,
            labels,
            status,
            agent,
            ..
        } => {
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
            let mut input = CreateTaskInput::new(title);
            input.status = status;
            input.labels = labels;
            if let Some(agent) = agent {
                input.agent = agent;
            }
            match ticket_store.create_task(input) {
                Ok(stored) => (format!("{}\n", stored.task.id), 0),
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
