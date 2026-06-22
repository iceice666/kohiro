//! Repo detail view: Files (browser + blob viewer), Commits, Issues
//! (myque-backed), and CI job sub-tabs. Ported from the Go `tui/repo_detail.go`
//! and `tui/issues_view.go`, minus git-bug comments.

use std::sync::Arc;

use myque::{Status, StoredTask};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph, Wrap};

use super::input::{Key, MultilineInput, TextInput};
use super::{BLUE, DetailOutcome, OVERLAY, PEACH, PURPLE, SUBTEXT, TEAL};
use crate::auth;
use crate::git::{self, BlobView, CommitEntry, TreeEntry};
use crate::paths::Paths;
use crate::store::{Store, User};
use crate::{ci, tickets};

const ALL_STATUSES: [Status; 8] = [
    Status::Backlog,
    Status::Ready,
    Status::Blocked,
    Status::Running,
    Status::Review,
    Status::Done,
    Status::Failed,
    Status::Cancelled,
];

fn status_index(status: &Status) -> usize {
    ALL_STATUSES.iter().position(|s| s == status).unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailSub {
    Files,
    Commits,
    Issues,
    Ci,
}

fn next_sub(sub: DetailSub) -> DetailSub {
    match sub {
        DetailSub::Files => DetailSub::Commits,
        DetailSub::Commits => DetailSub::Issues,
        DetailSub::Issues => DetailSub::Ci,
        DetailSub::Ci => DetailSub::Files,
    }
}

pub(crate) struct RepoDetail {
    paths: Arc<Paths>,
    owner: String,
    name: String,
    active_sub: DetailSub,
    current_path: String,
    files: Vec<TreeEntry>,
    files_state: ListState,
    blob: Option<BlobView>,
    blob_scroll: u16,
    commits: Vec<CommitEntry>,
    commits_state: ListState,
    issues: IssuesSub,
    ci_jobs: Vec<chilin::Job>,
    ci_state: ListState,
    selected_ci: Option<chilin::Job>,
    ci_log: String,
    ci_scroll: u16,
}

impl RepoDetail {
    pub(crate) async fn new(
        owner: String,
        name: String,
        store: Arc<Store>,
        paths: Arc<Paths>,
        user: Option<User>,
    ) -> Self {
        let mut issues = IssuesSub::new(store, paths.clone(), user, owner.clone(), name.clone());
        issues.load();

        let mut detail = RepoDetail {
            paths,
            owner,
            name,
            active_sub: DetailSub::Files,
            current_path: String::new(),
            files: Vec::new(),
            files_state: ListState::default(),
            blob: None,
            blob_scroll: 0,
            commits: Vec::new(),
            commits_state: ListState::default(),
            ci_jobs: Vec::new(),
            ci_state: ListState::default(),
            selected_ci: None,
            ci_log: String::new(),
            ci_scroll: 0,
            issues,
        };
        detail.load_tree().await;
        detail.load_commits().await;
        detail.load_ci_jobs();
        detail
    }

    fn repo_dir(&self) -> std::path::PathBuf {
        self.paths.repo_path(&self.owner, &self.name)
    }

    async fn load_tree(&mut self) {
        let dir = self.repo_dir();
        match git::list_tree(&dir, &self.current_path).await {
            Ok(entries) => {
                self.files = entries;
                select_first(&mut self.files_state, self.files.len());
            }
            Err(_) => {
                self.files.clear();
                self.files_state.select(None);
            }
        }
    }

    async fn load_commits(&mut self) {
        let dir = self.repo_dir();
        self.commits = git::commit_log(&dir, 50).await.unwrap_or_default();
        select_first(&mut self.commits_state, self.commits.len());
    }

    fn load_ci_jobs(&mut self) {
        match ci::list_jobs(&self.paths, &self.owner, &self.name, 50) {
            Ok(jobs) => {
                self.ci_jobs = jobs;
                select_first(&mut self.ci_state, self.ci_jobs.len());
            }
            Err(_) => {
                self.ci_jobs.clear();
                self.ci_state.select(None);
            }
        }
    }

    fn open_ci_job(&mut self) -> DetailOutcome {
        let Some(id) = self
            .ci_state
            .selected()
            .and_then(|i| self.ci_jobs.get(i))
            .map(|job| job.id)
        else {
            return DetailOutcome::Ignore;
        };
        self.load_selected_ci(id);
        self.ci_scroll = 0;
        DetailOutcome::Redraw
    }

    fn load_selected_ci(&mut self, id: i64) {
        match ci::get_job(&self.paths, &self.owner, &self.name, id) {
            Ok(Some(job)) => {
                self.ci_log = ci::read_job_log(&job);
                self.selected_ci = Some(job);
            }
            _ => {
                self.ci_log = "no such CI job\n".to_owned();
                self.selected_ci = None;
            }
        }
    }

    fn refresh_selected_ci(&mut self) -> DetailOutcome {
        if let Some(id) = self.selected_ci.as_ref().map(|job| job.id) {
            self.load_selected_ci(id);
            DetailOutcome::Redraw
        } else {
            self.load_ci_jobs();
            DetailOutcome::Redraw
        }
    }

    fn scroll_ci_down(&mut self, by: u16) {
        let max = self.ci_log.lines().count().saturating_sub(1) as u16;
        self.ci_scroll = self.ci_scroll.saturating_add(by).min(max);
    }

    pub(crate) async fn update(&mut self, key: Key) -> DetailOutcome {
        match key {
            Key::Tab => {
                let issues_modal =
                    matches!(self.active_sub, DetailSub::Issues) && self.issues.is_modal();
                if self.blob.is_none() && !issues_modal {
                    self.active_sub = next_sub(self.active_sub);
                    if matches!(self.active_sub, DetailSub::Ci) {
                        self.load_ci_jobs();
                    }
                }
                DetailOutcome::Redraw
            }
            Key::Esc => self.handle_esc().await,
            _ => match self.active_sub {
                DetailSub::Files => self.update_files(key).await,
                DetailSub::Commits => {
                    if super::handle_nav(&mut self.commits_state, self.commits.len(), &key) {
                        DetailOutcome::Redraw
                    } else {
                        DetailOutcome::Ignore
                    }
                }
                DetailSub::Issues => self.issues.update(key),
                DetailSub::Ci => {
                    if self.selected_ci.is_some() {
                        match key {
                            Key::Char('r') => self.refresh_selected_ci(),
                            Key::Up | Key::Char('k') => {
                                self.ci_scroll = self.ci_scroll.saturating_sub(1);
                                DetailOutcome::Redraw
                            }
                            Key::Down | Key::Char('j') => {
                                self.scroll_ci_down(1);
                                DetailOutcome::Redraw
                            }
                            Key::PageUp => {
                                self.ci_scroll = self.ci_scroll.saturating_sub(10);
                                DetailOutcome::Redraw
                            }
                            Key::PageDown => {
                                self.scroll_ci_down(10);
                                DetailOutcome::Redraw
                            }
                            _ => DetailOutcome::Ignore,
                        }
                    } else {
                        match key {
                            Key::Enter => self.open_ci_job(),
                            Key::Char('r') => self.refresh_selected_ci(),
                            _ if super::handle_nav(
                                &mut self.ci_state,
                                self.ci_jobs.len(),
                                &key,
                            ) =>
                            {
                                DetailOutcome::Redraw
                            }
                            _ => DetailOutcome::Ignore,
                        }
                    }
                }
            },
        }
    }

    async fn handle_esc(&mut self) -> DetailOutcome {
        if self.blob.is_some() {
            self.blob = None;
            return DetailOutcome::Redraw;
        }
        if matches!(self.active_sub, DetailSub::Issues) && !self.issues.is_list() {
            return self.issues.update(Key::Esc);
        }
        if matches!(self.active_sub, DetailSub::Ci) && self.selected_ci.is_some() {
            self.selected_ci = None;
            self.ci_log.clear();
            self.ci_scroll = 0;
            self.load_ci_jobs();
            return DetailOutcome::Redraw;
        }
        if matches!(self.active_sub, DetailSub::Files) && !self.current_path.is_empty() {
            self.current_path = parent_path(&self.current_path);
            self.load_tree().await;
            return DetailOutcome::Redraw;
        }
        DetailOutcome::Pop
    }

    async fn update_files(&mut self, key: Key) -> DetailOutcome {
        if self.blob.is_some() {
            return match key {
                Key::Up | Key::Char('k') => {
                    self.blob_scroll = self.blob_scroll.saturating_sub(1);
                    DetailOutcome::Redraw
                }
                Key::Down | Key::Char('j') => {
                    self.scroll_blob_down(1);
                    DetailOutcome::Redraw
                }
                Key::PageUp => {
                    self.blob_scroll = self.blob_scroll.saturating_sub(10);
                    DetailOutcome::Redraw
                }
                Key::PageDown => {
                    self.scroll_blob_down(10);
                    DetailOutcome::Redraw
                }
                _ => DetailOutcome::Ignore,
            };
        }
        match key {
            Key::Enter => {
                let Some(entry) = self
                    .files_state
                    .selected()
                    .and_then(|i| self.files.get(i))
                    .cloned()
                else {
                    return DetailOutcome::Ignore;
                };
                if entry.is_dir {
                    self.current_path = join_path(&self.current_path, &entry.name);
                    self.load_tree().await;
                } else {
                    let path = join_path(&self.current_path, &entry.name);
                    let dir = self.repo_dir();
                    self.blob_scroll = 0;
                    self.blob = Some(match git::read_blob(&dir, &path).await {
                        Ok(view) => view,
                        Err(err) => BlobView {
                            text: format!("Error: {err}"),
                            note: None,
                        },
                    });
                }
                DetailOutcome::Redraw
            }
            _ => {
                if super::handle_nav(&mut self.files_state, self.files.len(), &key) {
                    DetailOutcome::Redraw
                } else {
                    DetailOutcome::Ignore
                }
            }
        }
    }

    fn scroll_blob_down(&mut self, by: u16) {
        let max = self.blob_line_count().saturating_sub(1);
        self.blob_scroll = self.blob_scroll.saturating_add(by).min(max);
    }

    fn blob_line_count(&self) -> u16 {
        self.blob
            .as_ref()
            .map(|b| {
                let mut n = b.text.lines().count();
                if b.note.is_some() {
                    n += 1;
                }
                n as u16
            })
            .unwrap_or(0)
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect) {
        let rows = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        self.render_breadcrumb(f, rows[0]);
        f.render_widget(
            Paragraph::new("─".repeat(rows[1].width as usize)).style(Style::default().fg(OVERLAY)),
            rows[1],
        );

        let content = rows[2];
        match self.active_sub {
            DetailSub::Files => {
                if let Some(blob) = self.blob.as_ref() {
                    render_blob(f, content, blob, self.blob_scroll);
                } else {
                    let items: Vec<ListItem> = self.files.iter().map(file_item).collect();
                    let title = if self.current_path.is_empty() {
                        "Files".to_owned()
                    } else {
                        format!("Files /{}", self.current_path)
                    };
                    super::render_list(
                        f,
                        content,
                        &title,
                        "No files on the default branch yet.",
                        items,
                        &self.files_state,
                    );
                }
            }
            DetailSub::Commits => {
                let items: Vec<ListItem> = self.commits.iter().map(commit_item).collect();
                super::render_list(
                    f,
                    content,
                    "Commits",
                    "No commits yet. Push to this repository to populate history.",
                    items,
                    &self.commits_state,
                );
            }
            DetailSub::Issues => self.issues.render(f, content),
            DetailSub::Ci => {
                if let Some(job) = self.selected_ci.as_ref() {
                    render_ci_detail(f, content, job, &self.ci_log, self.ci_scroll);
                } else {
                    let items: Vec<ListItem> = self.ci_jobs.iter().map(ci_job_item).collect();
                    super::render_list(
                        f,
                        content,
                        "CI jobs",
                        "No CI jobs yet. Push a commit with .ci/push to enqueue one.",
                        items,
                        &self.ci_state,
                    );
                }
            }
        }

        let (toast, hint) = match self.active_sub {
            DetailSub::Files if self.blob.is_some() => {
                (None, "↑↓ scroll · esc close · ctrl+c quit")
            }
            DetailSub::Files => (
                None,
                "↑↓ move · enter open · esc back/up · tab switch · ctrl+c quit",
            ),
            DetailSub::Commits => (None, "↑↓ move · esc back · tab switch · ctrl+c quit"),
            DetailSub::Issues => (self.issues.toast.as_ref(), self.issues.footer_hint()),
            DetailSub::Ci if self.selected_ci.is_some() => {
                (None, "↑↓ scroll · r refresh · esc jobs · ctrl+c quit")
            }
            DetailSub::Ci => (
                None,
                "↑↓ move · enter logs · r refresh · esc back · tab switch · ctrl+c quit",
            ),
        };
        super::render_footer(f, rows[3], toast, hint);

        if matches!(self.active_sub, DetailSub::Issues) {
            self.issues.render_modal_overlay(f, area);
        }
    }

    fn render_breadcrumb(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled(
            format!("{}/{}", self.owner, self.name),
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        )];
        if matches!(self.active_sub, DetailSub::Files) && !self.current_path.is_empty() {
            spans.push(Span::styled(" › ", Style::default().fg(OVERLAY)));
            spans.push(Span::styled(
                self.current_path.clone(),
                Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::raw("   "));
        for (label, sub) in [
            ("Files", DetailSub::Files),
            ("Commits", DetailSub::Commits),
            ("Issues", DetailSub::Issues),
            ("CI", DetailSub::Ci),
        ] {
            let style = if self.active_sub == sub {
                Style::default()
                    .fg(PURPLE)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(SUBTEXT)
            };
            spans.push(Span::styled(format!(" {label} "), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

// --- Issues sub-tab (myque-backed) ---------------------------------------

enum IssuesMode {
    List,
    Detail,
    New,
    EditBody,
    StatusPick,
}

struct IssuesSub {
    store: Arc<Store>,
    paths: Arc<Paths>,
    user: Option<User>,
    owner: String,
    name: String,
    mode: IssuesMode,
    items: Vec<StoredTask>,
    state: ListState,
    selected: Option<StoredTask>,
    detail_scroll: u16,
    input: TextInput,
    body_input: MultilineInput,
    status_state: ListState,
    toast: Option<(String, bool)>,
}

impl IssuesSub {
    fn new(
        store: Arc<Store>,
        paths: Arc<Paths>,
        user: Option<User>,
        owner: String,
        name: String,
    ) -> Self {
        Self {
            store,
            paths,
            user,
            owner,
            name,
            mode: IssuesMode::List,
            items: Vec::new(),
            state: ListState::default(),
            selected: None,
            input: TextInput::default(),
            body_input: MultilineInput::default(),
            status_state: ListState::default(),
            detail_scroll: 0,
            toast: None,
        }
    }

    fn load(&mut self) {
        match tickets::list_tasks(&self.paths, &self.owner, &self.name) {
            Ok(items) => {
                self.items = items;
                select_first(&mut self.state, self.items.len());
            }
            Err(err) => {
                self.items.clear();
                self.state.select(None);
                self.toast = Some((err.to_string(), true));
            }
        }
    }

    fn is_modal(&self) -> bool {
        matches!(self.mode, IssuesMode::New | IssuesMode::StatusPick)
    }

    fn is_list(&self) -> bool {
        matches!(self.mode, IssuesMode::List)
    }

    fn can_write(&self) -> bool {
        auth::can_write(&self.store, self.user.as_ref(), &self.owner, &self.name)
    }

    fn update(&mut self, key: Key) -> DetailOutcome {
        match self.mode {
            IssuesMode::List => self.update_list(key),
            IssuesMode::Detail => self.update_detail(key),
            IssuesMode::New => self.update_new(key),
            IssuesMode::EditBody => self.update_edit_body(key),
            IssuesMode::StatusPick => self.update_status_pick(key),
        }
    }

    fn update_list(&mut self, key: Key) -> DetailOutcome {
        if super::handle_nav(&mut self.state, self.items.len(), &key) {
            return DetailOutcome::Redraw;
        }
        match key {
            Key::Enter => {
                let Some(id) = self
                    .state
                    .selected()
                    .and_then(|i| self.items.get(i))
                    .map(|t| t.task.id.clone())
                else {
                    return DetailOutcome::Ignore;
                };
                match tickets::get_task(&self.paths, &self.owner, &self.name, &id) {
                    Ok(task) => {
                        self.status_state
                            .select(Some(status_index(&task.task.status)));
                        self.selected = Some(task);
                        self.detail_scroll = 0;
                        self.mode = IssuesMode::Detail;
                        self.toast = None;
                    }
                    Err(err) => self.toast = Some((err.to_string(), true)),
                }
                DetailOutcome::Redraw
            }
            Key::Char('n') => {
                if !self.can_write() {
                    self.toast = Some(("not enough permission".into(), true));
                    return DetailOutcome::Redraw;
                }
                self.mode = IssuesMode::New;
                self.input.clear();
                self.toast = None;
                DetailOutcome::Redraw
            }
            _ => DetailOutcome::Ignore,
        }
    }

    fn update_detail(&mut self, key: Key) -> DetailOutcome {
        match key {
            Key::Esc => {
                self.mode = IssuesMode::List;
                self.selected = None;
                DetailOutcome::Redraw
            }
            Key::Up | Key::Char('k') => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
                DetailOutcome::Redraw
            }
            Key::Down | Key::Char('j') => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
                DetailOutcome::Redraw
            }
            Key::PageUp => {
                self.detail_scroll = self.detail_scroll.saturating_sub(10);
                DetailOutcome::Redraw
            }
            Key::PageDown => {
                self.detail_scroll = self.detail_scroll.saturating_add(10);
                DetailOutcome::Redraw
            }
            Key::Char('e') => {
                if !self.can_write() {
                    self.toast = Some(("not enough permission".into(), true));
                    return DetailOutcome::Redraw;
                }
                if let Some(sel) = self.selected.as_ref() {
                    self.body_input.set(sel.body.clone());
                    self.detail_scroll = 0;
                    self.mode = IssuesMode::EditBody;
                    self.toast = None;
                }
                DetailOutcome::Redraw
            }
            Key::Char('m') => {
                if !self.can_write() {
                    self.toast = Some(("not enough permission".into(), true));
                    return DetailOutcome::Redraw;
                }
                if let Some(sel) = self.selected.as_ref() {
                    self.status_state
                        .select(Some(status_index(&sel.task.status)));
                }
                self.mode = IssuesMode::StatusPick;
                DetailOutcome::Redraw
            }
            _ => DetailOutcome::Ignore,
        }
    }

    fn update_new(&mut self, key: Key) -> DetailOutcome {
        match key {
            Key::Enter => {
                let title = self.input.value.trim().to_owned();
                if title.is_empty() {
                    self.toast = Some(("title cannot be empty".into(), true));
                    return DetailOutcome::Redraw;
                }
                self.mode = IssuesMode::List;
                match tickets::create_titled(
                    &self.paths,
                    &self.owner,
                    &self.name,
                    title,
                    Status::Backlog,
                ) {
                    Ok(_) => self.toast = Some(("issue created".into(), false)),
                    Err(err) => self.toast = Some((err.to_string(), true)),
                }
                self.load();
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = IssuesMode::List;
                DetailOutcome::Redraw
            }
            other => {
                self.input.handle(&other);
                DetailOutcome::Redraw
            }
        }
    }

    fn update_edit_body(&mut self, key: Key) -> DetailOutcome {
        match key {
            Key::CtrlS => {
                let Some(id) = self.selected.as_ref().map(|t| t.task.id.clone()) else {
                    self.mode = IssuesMode::Detail;
                    return DetailOutcome::Redraw;
                };
                match tickets::set_body(
                    &self.paths,
                    &self.owner,
                    &self.name,
                    &id,
                    self.body_input.value.clone(),
                ) {
                    Ok(updated) => {
                        self.selected = Some(updated);
                        self.toast = Some(("body saved".into(), false));
                        self.load();
                    }
                    Err(err) => self.toast = Some((err.to_string(), true)),
                }
                self.mode = IssuesMode::Detail;
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = IssuesMode::Detail;
                self.body_input.clear();
                DetailOutcome::Redraw
            }
            other => {
                self.body_input.handle(&other);
                DetailOutcome::Redraw
            }
        }
    }

    fn update_status_pick(&mut self, key: Key) -> DetailOutcome {
        if super::handle_nav(&mut self.status_state, ALL_STATUSES.len(), &key) {
            return DetailOutcome::Redraw;
        }
        match key {
            Key::Enter => {
                let idx = self.status_state.selected().unwrap_or(0);
                let status = ALL_STATUSES[idx].clone();
                let Some(id) = self.selected.as_ref().map(|t| t.task.id.clone()) else {
                    self.mode = IssuesMode::Detail;
                    return DetailOutcome::Redraw;
                };
                match tickets::set_status(&self.paths, &self.owner, &self.name, &id, status) {
                    Ok(updated) => {
                        self.selected = Some(updated);
                        self.toast = Some(("status updated".into(), false));
                        self.load();
                    }
                    Err(err) => self.toast = Some((err.to_string(), true)),
                }
                self.mode = IssuesMode::Detail;
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = IssuesMode::Detail;
                DetailOutcome::Redraw
            }
            _ => DetailOutcome::Ignore,
        }
    }

    fn footer_hint(&self) -> &'static str {
        match self.mode {
            IssuesMode::List => {
                "↑↓ move · enter open · n new · esc back · tab switch · ctrl+c quit"
            }
            IssuesMode::Detail => "↑↓ scroll · e edit body · m set status · esc back · ctrl+c quit",
            IssuesMode::New => "enter: create · esc: cancel",
            IssuesMode::EditBody => {
                "type body · arrows move · enter newline · ctrl+s save · esc cancel"
            }
            IssuesMode::StatusPick => "↑↓ move · enter set · esc cancel",
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let show_detail = matches!(
            self.mode,
            IssuesMode::Detail | IssuesMode::EditBody | IssuesMode::StatusPick
        );
        if matches!(self.mode, IssuesMode::EditBody) {
            if let Some(sel) = self.selected.as_ref() {
                let para = Paragraph::new(issue_edit_lines(sel, &self.body_input))
                    .wrap(Wrap { trim: false })
                    .scroll((self.detail_scroll, 0));
                f.render_widget(para, area);
            }
        } else if show_detail {
            if let Some(sel) = self.selected.as_ref() {
                let para = Paragraph::new(issue_detail_lines(sel))
                    .wrap(Wrap { trim: false })
                    .scroll((self.detail_scroll, 0));
                f.render_widget(para, area);
            }
        } else {
            let items: Vec<ListItem> = self.items.iter().map(issue_item).collect();
            super::render_list(
                f,
                area,
                "Issues",
                "No issues yet. Press n to create one.",
                items,
                &self.state,
            );
        }
    }

    fn render_modal_overlay(&self, f: &mut Frame, area: Rect) {
        match self.mode {
            IssuesMode::New => {
                let lines = vec![
                    Line::from(Span::styled(
                        "New issue title:",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(super::input_line(&self.input.value)),
                    Line::from(Span::styled(
                        "enter: create   esc: cancel",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "New Issue", lines);
            }
            IssuesMode::EditBody => {}
            IssuesMode::StatusPick => {
                let sel = self.status_state.selected().unwrap_or(0);
                let mut lines: Vec<Line> = ALL_STATUSES
                    .iter()
                    .enumerate()
                    .map(|(i, status)| {
                        let (marker, style) = if i == sel {
                            (
                                "▌ ",
                                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
                            )
                        } else {
                            ("  ", Style::default().fg(SUBTEXT))
                        };
                        Line::from(Span::styled(format!("{marker}{}", status.as_str()), style))
                    })
                    .collect();
                lines.push(Line::from(Span::styled(
                    "↑↓ move · enter set · esc cancel",
                    Style::default().fg(SUBTEXT),
                )));
                super::render_modal(f, area, "Set status", lines);
            }
            _ => {}
        }
    }
}

// --- Item / content builders ---------------------------------------------

fn file_item(entry: &TreeEntry) -> ListItem<'static> {
    if entry.is_dir {
        ListItem::new(Line::from(Span::styled(
            format!("{}/", entry.name),
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        )))
    } else {
        ListItem::new(Line::from(entry.name.clone()))
    }
}

fn commit_item(commit: &CommitEntry) -> ListItem<'static> {
    let title = Line::from(vec![
        Span::styled(
            commit.short_hash.clone(),
            Style::default().fg(PEACH).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(commit.subject.clone()),
    ]);
    let desc = Line::from(vec![
        Span::styled(commit.date.clone(), Style::default().fg(SUBTEXT)),
        Span::raw("  "),
        Span::styled(commit.author.clone(), Style::default().fg(BLUE)),
    ]);
    ListItem::new(vec![title, desc])
}

fn ci_job_item(job: &chilin::Job) -> ListItem<'static> {
    let title = Line::from(vec![
        Span::styled(
            format!("#{:<5}", job.id),
            Style::default().fg(PEACH).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{}]", job.status), Style::default().fg(BLUE)),
        Span::raw("  "),
        Span::raw(job.label.clone()),
    ]);
    let desc = Line::from(Span::styled(
        format!("{}  {}", job.enqueued_at, ci::format_command(&job.command)),
        Style::default().fg(SUBTEXT),
    ));
    ListItem::new(vec![title, desc])
}

fn render_ci_detail(f: &mut Frame, area: Rect, job: &chilin::Job, log: &str, scroll: u16) {
    let rows = ratatui::layout::Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(1)])
        .split(area);
    let meta = vec![
        Line::from(vec![
            Span::styled(
                format!("CI job #{}", job.id),
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format!("[{}]", job.status), Style::default().fg(BLUE)),
        ]),
        Line::from(Span::styled(
            format!("command: {}", ci::format_command(&job.command)),
            Style::default().fg(SUBTEXT),
        )),
        Line::from(Span::styled(
            format!(
                "queued: {}  started: {}  ended: {}",
                job.enqueued_at,
                job.started_at.as_deref().unwrap_or("-"),
                job.ended_at.as_deref().unwrap_or("-")
            ),
            Style::default().fg(SUBTEXT),
        )),
    ];
    f.render_widget(Paragraph::new(meta).wrap(Wrap { trim: false }), rows[0]);
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(OVERLAY))
        .title(Span::styled(
            " CI log ",
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(log.to_owned())
            .block(block)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
}

fn issue_item(task: &StoredTask) -> ListItem<'static> {
    let title = Line::from(vec![
        Span::styled(
            format!("[{}]", task.task.status),
            Style::default().fg(PEACH).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::raw(task.task.title.clone()),
    ]);
    let desc = Line::from(Span::styled(
        format!("{}  {}", task.task.id, task.task.labels.join(", ")),
        Style::default().fg(SUBTEXT),
    ));
    ListItem::new(vec![title, desc])
}

fn issue_detail_lines(task: &StoredTask) -> Vec<Line<'static>> {
    let t = &task.task;
    let mut lines = vec![
        Line::from(Span::styled(
            t.title.clone(),
            Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(t.id.clone(), Style::default().fg(PEACH)),
            Span::raw("  "),
            Span::styled(format!("[{}]", t.status), Style::default().fg(BLUE)),
            Span::raw("  "),
            Span::styled(
                format!("labels: {}", t.labels.join(", ")),
                Style::default().fg(SUBTEXT),
            ),
        ]),
        Line::from(Span::styled(
            format!("created: {}   updated: {}", t.created_at, t.updated_at),
            Style::default().fg(SUBTEXT),
        )),
        Line::from(""),
    ];
    for line in task.body.lines() {
        lines.push(Line::from(line.to_owned()));
    }
    lines
}

fn issue_edit_lines(task: &StoredTask, input: &MultilineInput) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            format!("# {}", task.task.title),
            Style::default().fg(PEACH).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("{} · {}", task.task.id, task.task.status.as_str()),
            Style::default().fg(SUBTEXT),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Editing issue body. Ctrl+S saves, Esc cancels. Arrow keys move the cursor.",
            Style::default().fg(SUBTEXT),
        )),
        Line::from(""),
    ];
    lines.extend(input.display_lines().into_iter().map(Line::from));
    lines
}

fn render_blob(f: &mut Frame, area: Rect, blob: &BlobView, scroll: u16) {
    let mut text = blob.text.clone();
    if let Some(note) = blob.note.as_ref() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(note);
    }
    f.render_widget(Paragraph::new(text).scroll((scroll, 0)), area);
}

// --- Path helpers ---------------------------------------------------------

fn select_first(state: &mut ListState, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        let sel = state.selected().unwrap_or(0).min(len - 1);
        state.select(Some(sel));
    }
}

fn join_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}/{name}")
    }
}

fn parent_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent.to_owned(),
        None => String::new(),
    }
}
