//! End-to-end SSH test: a PTY shell launches the ratatui TUI over the channel,
//! renders the expected UI, navigates into a repo detail view (Files + Commits)
//! driven by real keystrokes over the wire, and Ctrl+C quits cleanly. This
//! exercises the live wiring (pty_request -> shell_request -> Tui::start -> draw,
//! and data -> Tui::on_input -> model dispatch -> redraw / quit) that the unit
//! tests cannot cover.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kohiro::auth;
use kohiro::paths::Paths;
use kohiro::server::KohiroServer;
use kohiro::store::Store;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::Server as _;
use russh::{Channel, ChannelMsg, client};
use tempfile::tempdir;
use tokio::net::TcpListener;

struct ClientHandler;

#[async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

/// Drain channel data into `buf` until `needle` appears or the timeout elapses.
async fn read_until(channel: &mut Channel<client::Msg>, buf: &mut Vec<u8>, needle: &str) -> bool {
    if contains(buf, needle) {
        return true;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let Ok(maybe) = tokio::time::timeout(remaining, channel.wait()).await else {
            return false;
        };
        let Some(msg) = maybe else {
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
    let ci_db = Arc::new(chilin::Db::open(&paths.chilin_ci_db_path()).unwrap());
    ci_db.migrate().unwrap();
    let agent_db = Arc::new(chilin::Db::open(&paths.chilin_agent_db_path()).unwrap());
    agent_db.migrate().unwrap();

    let client_key = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519).unwrap();
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

    let host_key = PrivateKey::random(&mut rand::rngs::OsRng, Algorithm::Ed25519).unwrap();
    let server_config = Arc::new(russh::server::Config {
        keys: vec![host_key],
        inactivity_timeout: Some(Duration::from_secs(15)),
        methods: russh::MethodSet::PUBLICKEY,
        ..Default::default()
    });

    let mut srv = KohiroServer {
        store: store.clone(),
        paths: paths.clone(),
        ci_db,
        agent_db,
    };
    let server_task = tokio::spawn(async move {
        let _ = srv.run_on_socket(server_config, &listener).await;
    });

    // --- connect, authenticate, request a PTY shell ---
    let client_config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(15)),
        ..Default::default()
    });
    let mut session = client::connect(client_config, ("127.0.0.1", port), ClientHandler)
        .await
        .unwrap();
    assert!(
        session
            .authenticate_publickey("admin", Arc::new(client_key))
            .await
            .unwrap(),
        "authentication failed"
    );

    let mut channel = session.channel_open_session().await.unwrap();
    channel
        .request_pty(true, "xterm", 80, 24, 0, 0, &[])
        .await
        .unwrap();
    channel.request_shell(true).await.unwrap();

    let mut buf = Vec::new();

    // Initial frame: header + tabs + seeded repo + username.
    assert!(
        read_until(&mut channel, &mut buf, "kohiro").await,
        "header not rendered: {:?}",
        String::from_utf8_lossy(&buf)
    );
    assert!(
        read_until(&mut channel, &mut buf, "Repos").await,
        "Repos tab missing"
    );
    assert!(
        read_until(&mut channel, &mut buf, "Keys").await,
        "Keys tab missing"
    );
    assert!(
        read_until(&mut channel, &mut buf, "admin/demo").await,
        "seeded repo missing"
    );
    assert!(
        read_until(&mut channel, &mut buf, "@admin").await,
        "username missing"
    );

    // Enter opens the repo detail view: the "Commits" sub-tab label and the
    // committed file only exist in the detail Files view.
    channel.data(&b"\r"[..]).await.unwrap();
    assert!(
        read_until(&mut channel, &mut buf, "Commits").await,
        "detail view not opened: {:?}",
        String::from_utf8_lossy(&buf)
    );
    assert!(
        read_until(&mut channel, &mut buf, "hello.txt").await,
        "Files browser missing seeded file"
    );

    // Tab cycles Files -> Commits, which shows the seeded commit's subject and
    // author. (The byte stream may split a multi-word subject across an
    // unchanged cell during ratatui's diff render, so assert on the contiguous
    // subject word and the author — both unique to the Commits view.)
    channel.data(&b"\t"[..]).await.unwrap();
    assert!(
        read_until(&mut channel, &mut buf, "seed").await,
        "Commits view missing commit subject"
    );
    assert!(
        read_until(&mut channel, &mut buf, "Admin").await,
        "Commits view missing commit author"
    );

    // Ctrl+C quits the TUI and closes the channel cleanly.
    channel.data(&b"\x03"[..]).await.unwrap();
    let mut closed = false;
    let mut exit_status = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(maybe) = tokio::time::timeout(remaining, channel.wait()).await else {
            break;
        };
        let Some(msg) = maybe else {
            closed = true;
            break;
        };
        match msg {
            ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
            ChannelMsg::Eof | ChannelMsg::Close => {
                closed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(
        closed || exit_status == Some(0),
        "TUI did not close on Ctrl+C (exit={exit_status:?})"
    );

    server_task.abort();
}
