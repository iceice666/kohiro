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

pub fn delete(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(path).with_context(|| format!("remove repo dir {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct BlobView {
    pub text: String,
    pub note: Option<String>,
}

pub async fn commit_log(dir: &Path, n: usize) -> anyhow::Result<Vec<CommitEntry>> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("log")
        .arg("-n")
        .arg(n.to_string())
        .arg("--format=%H%x1f%an%x1f%aI%x1f%s")
        .output()
        .await
        .with_context(|| format!("run git log in {}", dir.display()))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split('\u{1f}');
        let hash = fields.next().unwrap_or("");
        let author = fields.next().unwrap_or("");
        let date = fields.next().unwrap_or("");
        let subject = fields.next().unwrap_or("");
        commits.push(CommitEntry {
            short_hash: hash.chars().take(7).collect(),
            author: author.to_owned(),
            date: date.chars().take(10).collect(),
            subject: subject.to_owned(),
        });
    }
    Ok(commits)
}

pub async fn list_tree(dir: &Path, path: &str) -> anyhow::Result<Vec<TreeEntry>> {
    let treeish = if path.is_empty() {
        "HEAD".to_owned()
    } else {
        format!("HEAD:{path}")
    };
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("ls-tree")
        .arg(&treeish)
        .output()
        .await
        .with_context(|| format!("run git ls-tree in {}", dir.display()))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        // <mode> <type> <sha>\t<name>
        let Some((meta, name)) = line.split_once('\t') else {
            continue;
        };
        let kind = meta.split_whitespace().nth(1).unwrap_or("");
        entries.push(TreeEntry {
            name: name.to_owned(),
            is_dir: kind == "tree",
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(entries)
}

pub async fn read_blob(dir: &Path, path: &str) -> anyhow::Result<BlobView> {
    let spec = format!("HEAD:{path}");
    let size_out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("cat-file")
        .arg("-s")
        .arg(&spec)
        .output()
        .await
        .with_context(|| format!("run git cat-file -s in {}", dir.display()))?;
    if !size_out.status.success() {
        anyhow::bail!(
            "git cat-file -s {} failed: {}",
            spec,
            String::from_utf8_lossy(&size_out.stderr).trim()
        );
    }
    let size: usize = String::from_utf8_lossy(&size_out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    if size > (1 << 20) {
        return Ok(BlobView {
            text: String::new(),
            note: Some("<file too large (> 1 MiB)>".to_owned()),
        });
    }
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("cat-file")
        .arg("-p")
        .arg(&spec)
        .output()
        .await
        .with_context(|| format!("run git cat-file -p in {}", dir.display()))?;
    if !out.status.success() {
        anyhow::bail!(
            "git cat-file -p {} failed: {}",
            spec,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let bytes = out.stdout;
    let scan_len = bytes.len().min(8 * 1024);
    if bytes[..scan_len].contains(&0) {
        return Ok(BlobView {
            text: String::new(),
            note: Some(format!("<binary file, {} bytes>", bytes.len())),
        });
    }
    const MAX_TEXT: usize = 256 * 1024;
    if bytes.len() > MAX_TEXT {
        Ok(BlobView {
            text: String::from_utf8_lossy(&bytes[..MAX_TEXT]).into_owned(),
            note: Some("<truncated — showing first 256 KiB>".to_owned()),
        })
    } else {
        Ok(BlobView {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            note: None,
        })
    }
}
