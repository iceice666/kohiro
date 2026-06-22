use kohiro::auth;
use kohiro::paths::Paths;
use kohiro::server::KohiroServer;
use kohiro::store::Store;
use myque::{CreateTaskInput, Status, TaskStore};
use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::PrivateKey;
use russh::server::Server as _;
use std::net::ToSocketAddrs;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::net::TcpListener;

struct TestClient;

#[async_trait::async_trait]
impl client::Handler for TestClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn contains(buf: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(buf).contains(needle)
}

async fn read_until(
    channel: &mut russh::Channel<client::Msg>,
    buf: &mut Vec<u8>,
    needle: &str,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if contains(buf, needle) {
            return true;
        }
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            return false;
        }
        let Some(msg) = tokio::time::timeout(timeout, channel.wait())
            .await
            .ok()
            .flatten()
        else {
            return false;
        };
        if let ChannelMsg::Data { ref data } = msg {
            buf.extend_from_slice(data);
            if contains(buf, needle) {
                return true;
            }
        }
    }
}

#[tokio::test]
async fn pty_shell_drives_tui_over_ssh() {
    // --- temp data dir + store + a seeded repo with one commit ---
    let dir = tempdir().unwrap();
    let paths = Arc::new(Paths::new(dir.path().join("data")));
    std::fs::create_dir_all(paths.repos_dir()).unwrap();
    let store = Arc::new(Store::open(&paths.db_path()).unwrap());

    let ticket_store = TaskStore::new(paths.myque_root("admin", "demo"));
    ticket_store.init(false).unwrap();
    let log_path = paths.ci_log_dir("admin", "demo").join("ci-1.log");
    let mut input = CreateTaskInput::new("CI push");
    input.id = Some("ci-1".to_owned());
    input.status = Status::Done;
    input.labels = vec![
        "safe-auto".to_owned(),
        "ci".to_owned(),
        "ci:push".to_owned(),
    ];
    input.agent = "ci".to_owned();
    input.backend = "chilin".to_owned();
    input.allowed_auto_dispatch = true;
    input.body = Some(format!(
        r#"## Goal

Run push CI.

## Context

Fixture.

## Constraints

None.

## Acceptance

Done.

## Chilin

```toml
command = ["sh", ".ci/push"]
log_path = "{}"
```
"#,
        log_path.display()
    ));
    let mut task = ticket_store.create_task(input).unwrap();
    task.task.completed_at = Some("2026-06-22T00:00:00Z".to_owned());
    task.frontmatter.completed_at = task.task.completed_at.clone();
    ticket_store.write_task(&task).unwrap();
    std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
    std::fs::write(&log_path, "demo-log-line\nsecond-line\n").unwrap();

    let client_key =
        PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519).unwrap();
    let fp = auth::fingerprint_of(client_key.public_key());
    store.bootstrap("admin", &fp, "admin@test").unwrap();

    let admin = store.user_by_username("admin").unwrap().unwrap();
    store.ensure_repo(admin.id, "demo").unwrap();
    let bare = paths.repo_path("admin", "demo");
    kohiro::git::ensure_bare(&bare).unwrap();

    // Push one commit into the bare repo so Files/Commits are non-empty.
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    run_git(&work, &["init", "-q", "-b", "master"]);
    run_git(&work, &["config", "user.email", "a@example"]);
    run_git(&work, &["config", "user.name", "Admin"]);
    std::fs::write(work.join("hello.txt"), "hi there\n").unwrap();
    run_git(&work, &["add", "."]);
    run_git(&work, &["commit", "-q", "-m", "seed commit"]);
    run_git(&work, &["push", "-q", bare.to_str().unwrap(), "master"]);

    // --- start the server on an ephemeral port ---
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let host_key =
        PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519).unwrap();
    let server_config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        inactivity_timeout: Some(Duration::from_secs(15)),
        methods: russh::MethodSet::PUBLICKEY,
        ..Default::default()
    });

    let mut srv = KohiroServer {
        store: store.clone(),
        paths: paths.clone(),
        ci_runner: Arc::new(chilin::ShellRunner),
        agent_runner: Arc::new(chilin::ShellRunner),
    };
    let server_task = tokio::spawn(async move {
        let _ = srv.run_on_socket(server_config, &listener).await;
    });

    // --- connect, authenticate, request a PTY shell ---
    let client_config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(15)),
        ..Default::default()
    });
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    let mut handle: Handle<TestClient> = client::connect(client_config, addr, TestClient)
        .await
        .unwrap();
    let auth = handle
        .authenticate_publickey("admin", Arc::new(client_key))
        .await
        .unwrap();
    assert!(auth);

    let mut channel = handle.channel_open_session().await.unwrap();
    channel
        .request_pty(true, "xterm", 100, 30, 0, 0, &[])
        .await
        .unwrap();
    channel.request_shell(true).await.unwrap();

    let mut buf = Vec::new();
    assert!(
        read_until(&mut channel, &mut buf, "Repositories").await,
        "initial TUI did not render repositories; got {}",
        String::from_utf8_lossy(&buf)
    );
    assert!(
        read_until(&mut channel, &mut buf, "admin/demo").await,
        "repo list missing admin/demo"
    );

    // Open repo detail with Enter.
    channel.data(&b"\r"[..]).await.unwrap();
    assert!(
        read_until(&mut channel, &mut buf, "hello.txt").await,
        "Files browser missing seeded file"
    );
    assert!(
        read_until(&mut channel, &mut buf, "Files").await,
        "Files panel title missing"
    );

    // Tab cycles Files -> Commits -> Kanban. Shift+Tab goes back.
    channel.data(&b"\t"[..]).await.unwrap();
    buf.clear();
    assert!(
        read_until(&mut channel, &mut buf, "seed").await,
        "Commits view missing commit subject"
    );
    assert!(
        read_until(&mut channel, &mut buf, "Admin").await,
        "Commits view missing commit author"
    );

    channel.data(&b"\t"[..]).await.unwrap();
    buf.clear();
    assert!(
        read_until(&mut channel, &mut buf, "Kanban · [all]").await,
        "Kanban view missing flattened status-tab title"
    );
    assert!(
        read_until(&mut channel, &mut buf, "CI push").await,
        "Kanban view missing seeded task title"
    );

    // Status filters are flattened into left/right Kanban tabs; all -> done is
    // six right-arrow steps for the seeded done task.
    channel
        .data(&b"\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C"[..])
        .await
        .unwrap();
    buf.clear();
    assert!(
        read_until(&mut channel, &mut buf, "[done]").await,
        "Kanban did not switch to the done status tab"
    );
    assert!(
        read_until(&mut channel, &mut buf, "CI push").await,
        "Done Kanban tab missing seeded done task"
    );

    // Shift+Tab returns from Kanban to Commits.
    channel.data(&b"\x1b[Z"[..]).await.unwrap();
    buf.clear();
    assert!(
        read_until(&mut channel, &mut buf, "Commits").await,
        "Shift+Tab did not move back to Commits"
    );

    channel.data(&b"\x03"[..]).await.unwrap();
    server_task.abort();
}
