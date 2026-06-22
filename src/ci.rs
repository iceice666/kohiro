use crate::git;
use crate::paths::Paths;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Check out HEAD and enqueue a `.ci/push` job. Returns the chilin job id, or
/// None when the repo has no `.ci/push` at HEAD (or HEAD does not resolve).
pub async fn enqueue_push(
    ci_db: &chilin::Db,
    paths: &Paths,
    owner: &str,
    name: &str,
    bare_repo: &Path,
    pusher: Option<&str>,
) -> anyhow::Result<Option<i64>> {
    if !git::path_exists_at_head(bare_repo, ".ci/push").await {
        return Ok(None);
    }
    let Some(sha) = git::resolve_head(bare_repo).await? else {
        return Ok(None);
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let workdir = paths.ci_work_dir(owner, name).join(format!("{sha}-{ts}"));
    let _ = tokio::fs::remove_dir_all(&workdir).await;
    git::checkout_head(bare_repo, &workdir).await?;
    let id = ci_db.enqueue(chilin::JobSpec {
        namespace: format!("{owner}/{name}"),
        label: "push".into(),
        command: vec!["sh".into(), ".ci/push".into()],
        env: vec![
            ("CI_REPO".into(), format!("{owner}/{name}")),
            ("CI_SHA".into(), sha),
            ("CI_PUSHER".into(), pusher.unwrap_or("").to_owned()),
        ],
        mount: Some(chilin::Mount {
            source: workdir,
            target: "/repo".into(),
            readonly: false,
        }),
        log_dir: paths.ci_log_dir(owner, name),
    })?;
    Ok(Some(id))
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
            j.command.join(" ")
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
