use anyhow::Context;
use std::path::Path;
use std::process::Stdio;

pub fn ensure_bare(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create repo parent {}", parent.display()))?;
    }

    let output = std::process::Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(path)
        .output()
        .with_context(|| format!("run git init --bare {}", path.display()))?;

    if !output.status.success() {
        anyhow::bail!(
            "git init --bare {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("symbolic-ref")
        .arg("HEAD")
        .arg("refs/heads/master")
        .output()
        .with_context(|| format!("set default branch for {}", path.display()))?;

    if !output.status.success() {
        anyhow::bail!(
            "git symbolic-ref HEAD refs/heads/master in {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

pub fn git_service_command(service: &str, dir: &Path) -> tokio::process::Command {
    assert!(matches!(service, "upload-pack" | "receive-pack"));

    let mut command = tokio::process::Command::new("git");
    command
        .arg(service)
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}
