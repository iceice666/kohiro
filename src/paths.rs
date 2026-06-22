use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("kohiro.db")
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.data_dir.join("repos")
    }

    pub fn ssh_dir(&self) -> PathBuf {
        self.data_dir.join(".ssh")
    }

    pub fn host_key_path(&self) -> PathBuf {
        self.ssh_dir().join("host_key")
    }

    pub fn repo_path(&self, owner: &str, name: &str) -> PathBuf {
        self.repos_dir().join(owner).join(format!("{name}.git"))
    }

    pub fn myque_root(&self, owner: &str, name: &str) -> PathBuf {
        self.data_dir.join("myque").join(owner).join(name)
    }

    pub fn ci_log_dir(&self, owner: &str, name: &str) -> PathBuf {
        self.data_dir.join("ci").join("logs").join(owner).join(name)
    }

    pub fn agent_log_dir(&self, owner: &str, name: &str) -> PathBuf {
        self.data_dir
            .join("ci")
            .join("agent-logs")
            .join(owner)
            .join(name)
    }

    pub fn ci_work_dir(&self, owner: &str, name: &str) -> PathBuf {
        self.data_dir.join("ci").join("work").join(owner).join(name)
    }

    pub fn agent_work_dir(&self, owner: &str, name: &str) -> PathBuf {
        self.data_dir
            .join("ci")
            .join("agent-work")
            .join(owner)
            .join(name)
    }
}
