use crate::paths::Paths;
use myque::{
    AgentBackend, BackendDecision, BackendError, Config, DispatchResult, RunStatus, StoredTask,
    Task,
};
use std::sync::Arc;

pub struct ContainerBackend {
    pub runner: Arc<dyn chilin::Runner>,
    pub paths: Paths,
    pub owner: String,
    pub name: String,
}

impl ContainerBackend {
    fn command_template<'a>(&self, task: &Task, config: &'a Config) -> Option<&'a str> {
        config
            .agents
            .get(&task.agent)
            .and_then(|a| a.command.as_deref())
            .filter(|c| !c.trim().is_empty())
    }
}

impl AgentBackend for ContainerBackend {
    fn name(&self) -> &'static str {
        "container"
    }

    fn can_run(&self, task: &Task, config: &Config) -> BackendDecision {
        match self.command_template(task, config) {
            Some(_) => BackendDecision::allowed(),
            None => BackendDecision::rejected(format!("agent `{}` has no command", task.agent)),
        }
    }

    fn dispatch(&self, task: &StoredTask, config: &Config, run_id: String) -> DispatchResult {
        let reject = |msg: String| DispatchResult {
            run_id: run_id.clone(),
            started: false,
            message: msg,
            ended_at: None,
            exit_code: None,
        };
        let Some(template) = self.command_template(&task.task, config) else {
            return reject(format!("agent `{}` has no command", task.task.agent));
        };
        let Some(parts) = shlex::split(template) else {
            return reject(format!("invalid command: {template}"));
        };
        if parts.is_empty() {
            return reject("empty command".into());
        }
        let command: Vec<String> = parts
            .iter()
            .map(|p| {
                p.replace("{workspace}", "repo")
                    .replace("{task_file}", "task.md")
                    .replace("{task_id}", &task.task.id)
            })
            .collect();

        let workdir = self
            .paths
            .agent_work_dir(&self.owner, &self.name)
            .join(&run_id);
        let _ = std::fs::remove_dir_all(&workdir);
        if let Err(e) = std::fs::create_dir_all(&workdir) {
            return reject(format!("mkdir workdir: {e}"));
        }
        let bare = self.paths.repo_path(&self.owner, &self.name);
        let clone = std::process::Command::new("git")
            .arg("clone")
            .arg("--quiet")
            .arg(&bare)
            .arg(workdir.join("repo"))
            .output();
        match clone {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return reject(format!(
                    "git clone failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Err(e) => return reject(format!("git clone: {e}")),
        }
        if let Err(e) = std::fs::copy(&task.path, workdir.join("task.md")) {
            return reject(format!("write task.md: {e}"));
        }

        let log_dir = self.paths.agent_log_dir(&self.owner, &self.name);
        let body = format!(
            "{}\n\n## Chilin\n\n```toml\n{}\n```\n",
            task.body.trim_end(),
            chilin_block(
                &command,
                &[
                    ("MYQUE_TASK_ID".to_owned(), task.task.id.clone()),
                    (
                        "MYQUE_REPO".to_owned(),
                        format!("{}/{}", self.owner, self.name)
                    ),
                ],
                &chilin::Mount {
                    source: workdir.clone(),
                    target: "/work".into(),
                    readonly: false,
                },
                log_dir.join(format!("{run_id}.log"))
            )
        );
        let mut dispatched = task.clone();
        dispatched.body = body;
        let backend = chilin::ChilinRunner::new(self.runner.clone(), log_dir);
        backend.dispatch(&dispatched, config, run_id)
    }

    fn status(&self, run_id: &str, _: &Config) -> RunStatus {
        RunStatus {
            run_id: run_id.to_owned(),
            status: "unknown".into(),
            message: None,
        }
    }

    fn cancel(&self, _: &str, _: &Config) -> Result<(), BackendError> {
        Ok(())
    }
}

fn chilin_block(
    command: &[String],
    env: &[(String, String)],
    mount: &chilin::Mount,
    log_path: std::path::PathBuf,
) -> String {
    format!(
        "command = {}\nenv = {}\nlog_path = {}\n\n[mount]\nsource = {}\ntarget = {}\nreadonly = {}",
        toml_string_array(command),
        toml_pairs(env),
        toml_string(&log_path.display().to_string()),
        toml_string(&mount.source.display().to_string()),
        toml_string(&mount.target),
        mount.readonly
    )
}

fn toml_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| toml_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn toml_pairs(values: &[(String, String)]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|(key, value)| format!("[{}, {}]", toml_string(key), toml_string(value)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}
