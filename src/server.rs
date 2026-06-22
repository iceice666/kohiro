use crate::auth::{self, Access};
use crate::git;
use crate::paths::Paths;
use crate::store::Store;
use crate::tickets;
use anyhow::Context;
use async_trait::async_trait;
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStdin;

#[derive(Clone)]
pub struct KohiroServer {
    pub store: Arc<Store>,
    pub paths: Arc<Paths>,
}

impl server::Server for KohiroServer {
    type Handler = Conn;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> Conn {
        Conn::new(self.store.clone(), self.paths.clone())
    }

    fn handle_session_error(&mut self, error: <Self::Handler as server::Handler>::Error) {
        log::debug!("SSH session closed: {error:#}");
    }
}

pub struct Conn {
    store: Arc<Store>,
    paths: Arc<Paths>,
    fp: Option<String>,
    git_stdin: HashMap<ChannelId, ChildStdin>,
    pty: Option<(u16, u16)>,
    tui: Option<crate::tui::Tui>,
}

impl Conn {
    fn new(store: Arc<Store>, paths: Arc<Paths>) -> Self {
        Self {
            store,
            paths,
            fp: None,
            git_stdin: HashMap::new(),
            pty: None,
            tui: None,
        }
    }

    fn current_user(&self) -> Option<crate::store::User> {
        self.fp
            .as_deref()
            .and_then(|fp| auth::user_from_fingerprint(&self.store, fp))
    }

    async fn handle_git(
        &mut self,
        channel: ChannelId,
        service: &'static str,
        argv: &[String],
        session: &mut Session,
    ) -> Result<(), anyhow::Error> {
        let Some(repo_arg) = argv.last() else {
            session.channel_success(channel)?;
            return finish_with(session, channel, 128, "", "invalid path\n");
        };
        let repo_arg = repo_arg.strip_prefix('/').unwrap_or(repo_arg);
        let Some((owner, name)) = auth::parse_repo(repo_arg) else {
            session.channel_success(channel)?;
            return finish_with(session, channel, 128, "", "invalid path\n");
        };

        let user = self.current_user();
        let access = auth::git_access(&self.store, user.as_ref(), &owner, &name);
        let repo_path = self.paths.repo_path(&owner, &name);

        match service {
            "upload-pack" => {
                if access == Access::None {
                    session.channel_success(channel)?;
                    return finish_with(session, channel, 128, "", "access denied\n");
                }
                if !repo_path.exists() {
                    if access != Access::ReadWrite {
                        session.channel_success(channel)?;
                        return finish_with(session, channel, 128, "", "repository not found\n");
                    }
                    let Some(user) = user.as_ref() else {
                        session.channel_success(channel)?;
                        return finish_with(session, channel, 128, "", "access denied\n");
                    };
                    let owner_user = if user.username == owner {
                        user.clone()
                    } else {
                        match self.store.user_by_username(&owner)? {
                            Some(owner_user) => owner_user,
                            None => {
                                session.channel_success(channel)?;
                                return finish_with(
                                    session,
                                    channel,
                                    128,
                                    "",
                                    "unknown namespace\n",
                                );
                            }
                        }
                    };
                    git::ensure_bare(&repo_path)?;
                    self.store.ensure_repo(owner_user.id, &name)?;
                }
            }
            "receive-pack" => {
                if access != Access::ReadWrite {
                    session.channel_success(channel)?;
                    return finish_with(session, channel, 128, "", "access denied\n");
                }
                let Some(user) = user.as_ref() else {
                    session.channel_success(channel)?;
                    return finish_with(session, channel, 128, "", "access denied\n");
                };
                let owner_user = if user.username == owner {
                    user.clone()
                } else {
                    match self.store.user_by_username(&owner)? {
                        Some(owner_user) => owner_user,
                        None => {
                            session.channel_success(channel)?;
                            return finish_with(session, channel, 128, "", "unknown namespace\n");
                        }
                    }
                };
                git::ensure_bare(&repo_path)?;
                self.store.ensure_repo(owner_user.id, &name)?;
            }
            _ => unreachable!("validated git service"),
        }

        session.channel_success(channel)?;
        let mut child = match git::git_service_command(service, &repo_path).spawn() {
            Ok(child) => child,
            Err(err) => {
                return finish_with(
                    session,
                    channel,
                    128,
                    "",
                    &format!("failed to start git: {err}\n"),
                );
            }
        };

        let stdin = child
            .stdin
            .take()
            .context("git child stdin was not piped")?;
        let stdout = child
            .stdout
            .take()
            .context("git child stdout was not piped")?;
        let stderr = child
            .stderr
            .take()
            .context("git child stderr was not piped")?;
        self.git_stdin.insert(channel, stdin);

        let handle = session.handle();
        tokio::spawn(async move {
            let stdout_task = tokio::spawn(pipe_reader(stdout, handle.clone(), channel, false));
            let stderr_task = tokio::spawn(pipe_reader(stderr, handle.clone(), channel, true));
            let status = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            let code = status.ok().and_then(|status| status.code()).unwrap_or(1) as u32;
            let _ = handle.exit_status_request(channel, code).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });

        Ok(())
    }

    fn handle_issues(
        &mut self,
        channel: ChannelId,
        argv: &[String],
        session: &mut Session,
    ) -> Result<(), anyhow::Error> {
        let user = self.current_user();
        let (out, code) = tickets::run_issues(&self.store, &self.paths, user.as_ref(), argv);
        session.channel_success(channel)?;
        session.data(channel, CryptoVec::from(out))?;
        session.exit_status_request(channel, code as u32)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

#[async_trait]
impl server::Handler for Conn {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        pk: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        self.fp = Some(auth::fingerprint_of(pk));
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pty = Some((col_width as u16, row_height as u16));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        if let Some((cols, rows)) = self.pty {
            let tui = crate::tui::Tui::start(
                session.handle(),
                channel,
                self.store.clone(),
                self.paths.clone(),
                self.current_user(),
                cols,
                rows,
            )
            .await?;
            self.tui = Some(tui);
            Ok(())
        } else {
            finish_with(
                session,
                channel,
                0,
                "kohiro: interactive TUI requires a PTY (use `ssh -t`). For git use `git clone`.\n",
                "",
            )
        }
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(tui) = self.tui.as_mut() {
            if tui.channel() == channel {
                let quit = tui.on_input(data).await?;
                if quit {
                    self.tui = None;
                }
                return Ok(());
            }
        }
        let write_result = if let Some(stdin) = self.git_stdin.get_mut(&channel) {
            Some(stdin.write_all(data).await)
        } else {
            None
        };
        if let Some(Err(err)) = write_result {
            self.git_stdin.remove(&channel);
            return Err(err.into());
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.git_stdin.remove(&channel);
        if self.tui.as_ref().is_some_and(|t| t.channel() == channel) {
            self.tui = None;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.git_stdin.remove(&channel);
        if self.tui.as_ref().is_some_and(|t| t.channel() == channel) {
            self.tui = None;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pty = Some((col_width as u16, row_height as u16));
        if let Some(tui) = self.tui.as_mut() {
            if tui.channel() == channel {
                tui.on_resize(col_width as u16, row_height as u16).await?;
            }
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let line = String::from_utf8_lossy(data);
        let argv = shlex::split(&line).unwrap_or_default();
        if argv.is_empty() {
            session.channel_success(channel)?;
            return finish_with(session, channel, 1, "", "empty command\n");
        }

        if let Some(service) = git_service(&argv) {
            return self.handle_git(channel, service, &argv, session).await;
        }

        if argv[0] == "issues" {
            return self.handle_issues(channel, &argv, session);
        }

        session.channel_success(channel)?;
        finish_with(session, channel, 127, "", "unsupported command\n")
    }
}

fn git_service(argv: &[String]) -> Option<&'static str> {
    match argv {
        [command, ..] if command == "git-upload-pack" => Some("upload-pack"),
        [command, ..] if command == "git-receive-pack" => Some("receive-pack"),
        [git, service, ..] if git == "git" && service == "upload-pack" => Some("upload-pack"),
        [git, service, ..] if git == "git" && service == "receive-pack" => Some("receive-pack"),
        _ => None,
    }
}

fn finish_with(
    session: &mut Session,
    channel: ChannelId,
    code: u32,
    stdout: &str,
    stderr: &str,
) -> Result<(), anyhow::Error> {
    if !stdout.is_empty() {
        session.data(channel, CryptoVec::from(stdout))?;
    }
    if !stderr.is_empty() {
        session.extended_data(channel, 1, CryptoVec::from(stderr))?;
    }
    session.exit_status_request(channel, code)?;
    session.eof(channel)?;
    session.close(channel)?;
    Ok(())
}

async fn pipe_reader<R>(mut reader: R, handle: server::Handle, channel: ChannelId, stderr: bool)
where
    R: AsyncRead + Unpin,
{
    let mut buf = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buf).await;
        let n = match read {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let data = CryptoVec::from(&buf[..n]);
        let sent = if stderr {
            handle.extended_data(channel, 1, data).await
        } else {
            handle.data(channel, data).await
        };
        if sent.is_err() {
            break;
        }
    }
}
