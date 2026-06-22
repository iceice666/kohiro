use crate::git;
use crate::paths::Paths;
use anyhow::{Context, Result, anyhow};
use myque::{AgentConfig, BackendConfig, CreateTaskInput, Status, StoredTask, TaskStore};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const CI_AGENT: &str = "ci";
const CHILIN_BACKEND: &str = "chilin";

/// Check out HEAD and create a runnable `.ci/push` MyQue ticket. Returns the
/// created task, or None when the repo has no `.ci/push` at HEAD (or HEAD does
/// not resolve).
pub async fn enqueue_push(
    paths: &Paths,
    owner: &str,
    name: &str,
    bare_repo: &Path,
    pusher: Option<&str>,
) -> Result<Option<StoredTask>> {
    if !git::path_exists_at_head(bare_repo, ".ci/push").await {
        return Ok(None);
    }
    let Some(sha) = git::resolve_head(bare_repo).await? else {
        return Ok(None);
    };

    let id = next_ci_task_id();
    let workdir = paths.ci_work_dir(owner, name).join(&id);
    let log_path = paths.ci_log_dir(owner, name).join(format!("{id}.log"));
    let _ = tokio::fs::remove_dir_all(&workdir).await;
    git::checkout_head(bare_repo, &workdir).await?;

    let template = git::read_blob(bare_repo, ".ci/push")
        .await
        .context("read .ci/push template")?;
    if let Some(note) = template.note {
        return Err(anyhow!(".ci/push is not a text ticket template: {note}"));
    }

    let store = task_store(paths, owner, name);
    store.init(false)?;
    ensure_chilin_config(&store)?;

    let mut input = ci_input_from_template(&template.text, &id)?;
    input.body = Some(apply_push_context(
        input.body.as_deref().unwrap_or(""),
        PushContext {
            owner,
            name,
            sha: &sha,
            pusher: pusher.unwrap_or(""),
            workdir: &workdir,
            log_path: &log_path,
        },
    ));
    Ok(Some(store.create_task(input)?))
}

pub fn dispatch_ready(
    paths: &Paths,
    owner: &str,
    name: &str,
    runner: Arc<dyn chilin::Runner>,
) -> Result<myque::DispatchOutcome> {
    let store = task_store(paths, owner, name);
    let mut config = store.load_config()?;
    config.policy.require_allowed_label = true;
    config.policy.allowed_labels = vec!["ci:push".to_owned()];
    let mut registry = myque::BackendRegistry::with_builtins();
    registry.register(Box::new(chilin::ChilinRunner::new(
        runner,
        paths.ci_log_dir(owner, name),
    )));
    Ok(myque::dispatch_with(&store, &config, false, &registry)?)
}

pub fn list_jobs(paths: &Paths, owner: &str, name: &str, limit: usize) -> Result<Vec<chilin::Job>> {
    let mut jobs = task_store(paths, owner, name)
        .load_tasks()?
        .into_iter()
        .filter(is_ci_task)
        .map(|stored| task_to_job(paths, owner, name, stored))
        .collect::<Result<Vec<_>>>()?;
    jobs.sort_by_key(|job| std::cmp::Reverse(job.id));
    jobs.truncate(limit);
    Ok(jobs)
}

pub fn get_job(paths: &Paths, owner: &str, name: &str, id: i64) -> Result<Option<chilin::Job>> {
    Ok(list_jobs(paths, owner, name, usize::MAX)?
        .into_iter()
        .find(|job| job.id == id))
}

pub fn format_command(command: &[String]) -> String {
    command.join(" ")
}

pub fn format_job_table(jobs: &[chilin::Job]) -> String {
    let mut out = String::new();
    for j in jobs {
        out.push_str(&format!(
            "{:>6}  {:<10}  {:<16}  {}  {}\n",
            j.id,
            j.status,
            j.label,
            j.enqueued_at,
            format_command(&j.command)
        ));
    }
    if out.is_empty() {
        out.push_str("no jobs\n");
    }
    out
}

pub fn format_job_detail(j: &chilin::Job) -> String {
    let mut out = String::new();
    out.push_str(&format!("id          {}\n", j.id));
    out.push_str(&format!("namespace   {}\n", j.namespace));
    out.push_str(&format!("label       {}\n", j.label));
    out.push_str(&format!("status      {}\n", j.status));
    out.push_str(&format!(
        "exit_code   {}\n",
        j.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".into())
    ));
    out.push_str(&format!("command     {}\n", j.command.join(" ")));
    out.push_str(&format!("enqueued_at {}\n", j.enqueued_at));
    out.push_str(&format!(
        "started_at  {}\n",
        j.started_at.clone().unwrap_or_else(|| "-".into())
    ));
    out.push_str(&format!(
        "ended_at    {}\n",
        j.ended_at.clone().unwrap_or_else(|| "-".into())
    ));
    out
}

pub fn read_job_log(j: &chilin::Job) -> String {
    match std::fs::read_to_string(&j.log_path) {
        Ok(s) => s,
        Err(_) => format!("no log yet at {}\n", j.log_path.display()),
    }
}

fn task_store(paths: &Paths, owner: &str, name: &str) -> TaskStore {
    TaskStore::new(paths.myque_root(owner, name))
}

fn ensure_chilin_config(store: &TaskStore) -> Result<()> {
    let mut config = store.load_config()?;
    let had_backend = config.backends.contains_key(CHILIN_BACKEND);
    config
        .backends
        .entry(CHILIN_BACKEND.to_owned())
        .or_insert_with(|| BackendConfig {
            kind: CHILIN_BACKEND.to_owned(),
        });
    let had_agent = config.agents.contains_key(CI_AGENT);
    config
        .agents
        .entry(CI_AGENT.to_owned())
        .or_insert_with(|| AgentConfig {
            backend: CHILIN_BACKEND.to_owned(),
            command: None,
        });
    if !had_backend || !had_agent {
        store.write_config(&config)?;
    }
    Ok(())
}

fn ci_input_from_template(raw: &str, id: &str) -> Result<CreateTaskInput> {
    let (frontmatter, body) = if raw.trim_start().starts_with("+++") {
        let (frontmatter, body) = myque::parse_task_file(raw).context("parse .ci/push ticket")?;
        (Some(frontmatter), body.to_owned())
    } else {
        (None, raw.to_owned())
    };

    let mut input = CreateTaskInput::new(
        frontmatter
            .as_ref()
            .and_then(|fm| fm.title.clone())
            .unwrap_or_else(|| "CI push".to_owned()),
    );
    input.id = Some(id.to_owned());
    input.status = Status::Ready;
    input.priority = frontmatter.as_ref().and_then(|fm| fm.priority).unwrap_or(1);
    input.order = frontmatter.as_ref().and_then(|fm| fm.order).unwrap_or(100);
    input.labels = frontmatter
        .as_ref()
        .and_then(|fm| fm.labels.clone())
        .unwrap_or_default();
    ensure_label(&mut input.labels, "safe-auto");
    ensure_label(&mut input.labels, "ci");
    ensure_label(&mut input.labels, "ci:push");
    input.agent = frontmatter
        .as_ref()
        .and_then(|fm| fm.agent.clone())
        .unwrap_or_else(|| CI_AGENT.to_owned());
    input.backend = frontmatter
        .as_ref()
        .and_then(|fm| fm.backend.clone())
        .unwrap_or_else(|| CHILIN_BACKEND.to_owned());
    input.depends_on = frontmatter
        .as_ref()
        .and_then(|fm| fm.depends_on.clone())
        .unwrap_or_default();
    input.allowed_auto_dispatch = true;
    input.max_attempts = frontmatter
        .as_ref()
        .and_then(|fm| fm.max_attempts)
        .unwrap_or(2);
    input.body = Some(body);
    Ok(input)
}

fn ensure_label(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|existing| existing == label) {
        labels.push(label.to_owned());
    }
}

struct PushContext<'a> {
    owner: &'a str,
    name: &'a str,
    sha: &'a str,
    pusher: &'a str,
    workdir: &'a Path,
    log_path: &'a Path,
}

fn apply_push_context(body: &str, ctx: PushContext<'_>) -> String {
    body.replace("{repo}", &format!("{}/{}", ctx.owner, ctx.name))
        .replace("{owner}", ctx.owner)
        .replace("{name}", ctx.name)
        .replace("{sha}", ctx.sha)
        .replace("{pusher}", ctx.pusher)
        .replace("{workdir}", &ctx.workdir.display().to_string())
        .replace("{log_path}", &ctx.log_path.display().to_string())
}

fn task_to_job(paths: &Paths, owner: &str, name: &str, stored: StoredTask) -> Result<chilin::Job> {
    let parsed = chilin::parse_task_body(&stored.body).ok();
    let run =
        stored.task.last_run_id.as_deref().and_then(|run_id| {
            myque::read_run_record(&task_store(paths, owner, name), run_id).ok()
        });
    let fallback_log = stored
        .task
        .last_run_id
        .as_ref()
        .map(|run_id| paths.ci_log_dir(owner, name).join(format!("{run_id}.log")))
        .unwrap_or_else(|| {
            paths
                .ci_log_dir(owner, name)
                .join(format!("{}.log", stored.task.id))
        });
    Ok(chilin::Job {
        id: numeric_job_id(&stored.task.id),
        namespace: format!("{owner}/{name}"),
        label: "push".to_owned(),
        command: parsed
            .as_ref()
            .map(|task| task.command.clone())
            .unwrap_or_default(),
        env: parsed
            .as_ref()
            .map(|task| task.env.clone())
            .unwrap_or_default(),
        mount: parsed.as_ref().and_then(|task| task.mount.clone()),
        status: job_status_from_task_status(&stored.task.status),
        exit_code: run.as_ref().and_then(|run| run.exit_code),
        log_path: parsed
            .as_ref()
            .filter(|task| !task.log_path.as_os_str().is_empty())
            .map(|task| task.log_path.clone())
            .unwrap_or(fallback_log),
        enqueued_at: stored.task.created_at,
        started_at: stored.task.assigned_at,
        ended_at: stored.task.completed_at,
    })
}

fn is_ci_task(stored: &StoredTask) -> bool {
    stored.task.agent == CI_AGENT && stored.task.labels.iter().any(|label| label == "ci")
}

fn job_status_from_task_status(status: &Status) -> chilin::JobStatus {
    match status {
        Status::Ready => chilin::JobStatus::Pending,
        Status::Running => chilin::JobStatus::Running,
        Status::Done => chilin::JobStatus::Succeeded,
        Status::Failed => chilin::JobStatus::Failed,
        Status::Cancelled => chilin::JobStatus::Cancelled,
        _ => chilin::JobStatus::Pending,
    }
}

fn numeric_job_id(task_id: &str) -> i64 {
    task_id
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn next_ci_task_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch");
    format!("ci-{}-{:06}", now.as_millis(), now.subsec_micros())
}
