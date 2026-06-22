//! Repo detail view: Files (browser + blob viewer), Commits, and Kanban
//! (myque-backed task board). Ported from the Go `tui/repo_detail.go`
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

/// Kanban tab/filter applied to the task list. CI tickets are regular tasks
/// carrying the `ci` agent + label, so "CI" is one filter tab here rather
/// than a separate repo-detail tab.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KanbanTab {
    All,
    Status(Status),
    Ci,
}

impl KanbanTab {
    fn label(&self) -> &'static str {
        match self {
            KanbanTab::All => "all",
            KanbanTab::Status(status) => status.as_str(),
            KanbanTab::Ci => "ci",
        }
    }

    fn matches(&self, task: &StoredTask) -> bool {
        match self {
            KanbanTab::All => true,
            KanbanTab::Status(status) => task.task.status == *status,
            KanbanTab::Ci => ci::is_ci_task(task),
        }
    }
}

fn kanban_tabs() -> Vec<KanbanTab> {
    let mut tabs = Vec::with_capacity(ALL_STATUSES.len() + 2);
    tabs.push(KanbanTab::All);
    tabs.extend(ALL_STATUSES.iter().cloned().map(KanbanTab::Status));
    tabs.push(KanbanTab::Ci);
    tabs
}

fn next_kanban_tab(tab: &KanbanTab) -> KanbanTab {
    let tabs = kanban_tabs();
    let idx = tabs
        .iter()
        .position(|candidate| candidate == tab)
        .unwrap_or(0);
    tabs[(idx + 1) % tabs.len()].clone()
}

fn prev_kanban_tab(tab: &KanbanTab) -> KanbanTab {
    let tabs = kanban_tabs();
    let idx = tabs
        .iter()
        .position(|candidate| candidate == tab)
        .unwrap_or(0);
    tabs[(idx + tabs.len() - 1) % tabs.len()].clone()
}

fn visible_task_indices(items: &[StoredTask], tab: &KanbanTab) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, task)| tab.matches(task))
        .map(|(idx, _)| idx)
        .collect()
}

fn kanban_title(active: &KanbanTab) -> String {
    let labels = kanban_tabs()
        .into_iter()
        .map(|tab| {
            if &tab == active {
                format!("[{}]", tab.label())
            } else {
                tab.label().to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("Kanban · {labels}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailSub {
    Files,
    Commits,
    Kanban,
}

fn next_sub(sub: DetailSub) -> DetailSub {
    match sub {
        DetailSub::Files => DetailSub::Commits,
        DetailSub::Commits => DetailSub::Kanban,
        DetailSub::Kanban => DetailSub::Files,
    }
}

fn prev_sub(sub: DetailSub) -> DetailSub {
    match sub {
        DetailSub::Files => DetailSub::Kanban,
        DetailSub::Commits => DetailSub::Files,
        DetailSub::Kanban => DetailSub::Commits,
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
    kanban: KanbanSub,
}

impl RepoDetail {
    pub(crate) async fn new(
        owner: String,
        name: String,
        store: Arc<Store>,
        paths: Arc<Paths>,
        user: Option<User>,
    ) -> Self {
        let mut kanban = KanbanSub::new(store, paths.clone(), user, owner.clone(), name.clone());
        kanban.load();

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
            kanban,
        };
        detail.load_tree().await;
        detail.load_commits().await;
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

    pub(crate) async fn update(&mut self, key: Key) -> DetailOutcome {
        match key {
            Key::Tab | Key::ShiftTab => {
                let kanban_modal =
                    matches!(self.active_sub, DetailSub::Kanban) && self.kanban.is_modal();
                if self.blob.is_none() && !kanban_modal {
                    self.active_sub = if key == Key::Tab {
                        next_sub(self.active_sub)
                    } else {
                        prev_sub(self.active_sub)
                    };
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
                DetailSub::Kanban => self.kanban.update(key),
            },
        }
    }

    async fn handle_esc(&mut self) -> DetailOutcome {
        if self.blob.is_some() {
            self.blob = None;
            return DetailOutcome::Redraw;
        }
        if matches!(self.active_sub, DetailSub::Kanban) && !self.kanban.is_list() {
            return self.kanban.update(Key::Esc);
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
            DetailSub::Kanban => self.kanban.render(f, content),
        }

        let (toast, hint) = match self.active_sub {
            DetailSub::Files if self.blob.is_some() => {
                (None, "↑↓ scroll · esc close · ctrl+c quit")
            }
            DetailSub::Files => (
                None,
                "↑↓ move · enter open · esc back/up · tab/shift+tab switch · ctrl+c quit",
            ),
            DetailSub::Commits => (
                None,
                "↑↓ move · esc back · tab/shift+tab switch · ctrl+c quit",
            ),
            DetailSub::Kanban => (self.kanban.toast.as_ref(), self.kanban.footer_hint()),
        };
        super::render_footer(f, rows[3], toast, hint);

        if matches!(self.active_sub, DetailSub::Kanban) {
            self.kanban.render_modal_overlay(f, area);
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
            ("Kanban", DetailSub::Kanban),
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

// --- Kanban sub-tab (myque-backed) ---------------------------------------

enum KanbanMode {
    List,
    Detail,
    New,
    EditBody,
    StatusPick,
}

struct KanbanSub {
    store: Arc<Store>,
    paths: Arc<Paths>,
    user: Option<User>,
    owner: String,
    name: String,
    mode: KanbanMode,
    items: Vec<StoredTask>,
    tab: KanbanTab,
    state: ListState,
    selected: Option<StoredTask>,
    detail_scroll: u16,
    input: TextInput,
    body_input: MultilineInput,
    status_state: ListState,
    toast: Option<(String, bool)>,
}

impl KanbanSub {
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
            mode: KanbanMode::List,
            items: Vec::new(),
            tab: KanbanTab::All,
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
                let count = self.visible().len();
                select_first(&mut self.state, count);
            }
            Err(err) => {
                self.items.clear();
                self.state.select(None);
                self.toast = Some((err.to_string(), true));
            }
        }
    }

    /// Item indices of the tasks visible under the active Kanban tab.
    fn visible(&self) -> Vec<usize> {
        visible_task_indices(&self.items, &self.tab)
    }

    fn set_tab(&mut self, tab: KanbanTab) {
        self.tab = tab;
        let count = self.visible().len();
        select_first(&mut self.state, count);
    }

    fn is_modal(&self) -> bool {
        matches!(self.mode, KanbanMode::New | KanbanMode::StatusPick)
    }

    fn is_list(&self) -> bool {
        matches!(self.mode, KanbanMode::List)
    }

    fn can_write(&self) -> bool {
        auth::can_write(&self.store, self.user.as_ref(), &self.owner, &self.name)
    }

    fn update(&mut self, key: Key) -> DetailOutcome {
        match self.mode {
            KanbanMode::List => self.update_list(key),
            KanbanMode::Detail => self.update_detail(key),
            KanbanMode::New => self.update_new(key),
            KanbanMode::EditBody => self.update_edit_body(key),
            KanbanMode::StatusPick => self.update_status_pick(key),
        }
    }

    fn update_list(&mut self, key: Key) -> DetailOutcome {
        let visible = self.visible();
        if super::handle_nav(&mut self.state, visible.len(), &key) {
            return DetailOutcome::Redraw;
        }
        match key {
            Key::Left => {
                self.set_tab(prev_kanban_tab(&self.tab));
                DetailOutcome::Redraw
            }
            Key::Right => {
                self.set_tab(next_kanban_tab(&self.tab));
                DetailOutcome::Redraw
            }
            Key::Enter => {
                let Some(id) = self
                    .state
                    .selected()
                    .and_then(|sel| visible.get(sel))
                    .and_then(|&i| self.items.get(i))
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
                        self.mode = KanbanMode::Detail;
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
                self.mode = KanbanMode::New;
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
                self.mode = KanbanMode::List;
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
                    self.mode = KanbanMode::EditBody;
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
                self.mode = KanbanMode::StatusPick;
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
                self.mode = KanbanMode::List;
                match tickets::create_titled(
                    &self.paths,
                    &self.owner,
                    &self.name,
                    title,
                    Status::Backlog,
                ) {
                    Ok(_) => self.toast = Some(("task created".into(), false)),
                    Err(err) => self.toast = Some((err.to_string(), true)),
                }
                self.load();
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = KanbanMode::List;
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
                    self.mode = KanbanMode::Detail;
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
                self.mode = KanbanMode::Detail;
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = KanbanMode::Detail;
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
                    self.mode = KanbanMode::Detail;
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
                self.mode = KanbanMode::Detail;
                DetailOutcome::Redraw
            }
            Key::Esc => {
                self.mode = KanbanMode::Detail;
                DetailOutcome::Redraw
            }
            _ => DetailOutcome::Ignore,
        }
    }

    fn footer_hint(&self) -> &'static str {
        match self.mode {
            KanbanMode::List => {
                "↑↓ move · ←→ status tab · enter open · n new · esc back · tab/shift+tab switch · ctrl+c quit"
            }
            KanbanMode::Detail => "↑↓ scroll · e edit body · m set status · esc back · ctrl+c quit",
            KanbanMode::New => "enter: create · esc: cancel",
            KanbanMode::EditBody => {
                "type body · arrows move · enter newline · ctrl+s save · esc cancel"
            }
            KanbanMode::StatusPick => "↑↓ move · enter set · esc cancel",
        }
    }

    fn render(&self, f: &mut Frame, area: Rect) {
        let show_detail = matches!(
            self.mode,
            KanbanMode::Detail | KanbanMode::EditBody | KanbanMode::StatusPick
        );
        if matches!(self.mode, KanbanMode::EditBody) {
            if let Some(sel) = self.selected.as_ref() {
                let para = Paragraph::new(kanban_edit_lines(sel, &self.body_input))
                    .wrap(Wrap { trim: false })
                    .scroll((self.detail_scroll, 0));
                f.render_widget(para, area);
            }
        } else if show_detail {
            if let Some(sel) = self.selected.as_ref() {
                let para = Paragraph::new(kanban_detail_lines(sel))
                    .wrap(Wrap { trim: false })
                    .scroll((self.detail_scroll, 0));
                f.render_widget(para, area);
            }
        } else {
            let visible = self.visible();
            let mut items: Vec<ListItem> = visible
                .iter()
                .map(|&i| kanban_item(&self.items[i]))
                .collect();
            if items.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    "No tasks in this Kanban tab.",
                    Style::default().fg(SUBTEXT),
                ))));
            }
            let title = kanban_title(&self.tab);
            super::render_list(
                f,
                area,
                &title,
                "No tasks yet. Press n to create one.",
                items,
                &self.state,
            );
        }
    }

    fn render_modal_overlay(&self, f: &mut Frame, area: Rect) {
        match self.mode {
            KanbanMode::New => {
                let lines = vec![
                    Line::from(Span::styled(
                        "New task title:",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(super::input_line(&self.input.value)),
                    Line::from(Span::styled(
                        "enter: create   esc: cancel",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "New Kanban task", lines);
            }
            KanbanMode::EditBody => {}
            KanbanMode::StatusPick => {
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

fn kanban_item(task: &StoredTask) -> ListItem<'static> {
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

fn kanban_detail_lines(task: &StoredTask) -> Vec<Line<'static>> {
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

fn kanban_edit_lines(task: &StoredTask, input: &MultilineInput) -> Vec<Line<'static>> {
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
            "Editing task body. Ctrl+S saves, Esc cancels. Arrow keys move the cursor.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use myque::{CreateTaskInput, TaskStore};
    use tempfile::tempdir;

    fn plain(title: &str, status: Status) -> CreateTaskInput {
        let mut input = CreateTaskInput::new(title);
        input.status = status;
        input
    }

    fn ci(title: &str, status: Status) -> CreateTaskInput {
        let mut input = plain(title, status);
        input.agent = "ci".to_owned();
        input.labels = vec!["ci".to_owned()];
        input
    }

    fn store_with(inputs: Vec<CreateTaskInput>) -> Vec<StoredTask> {
        let dir = tempdir().unwrap();
        let store = TaskStore::new(dir.path());
        store.init(false).unwrap();
        for input in inputs {
            store.create_task(input).unwrap();
        }
        store.load_tasks().unwrap()
    }

    fn statuses_of(items: &[StoredTask], tab: &KanbanTab) -> Vec<Status> {
        visible_task_indices(items, tab)
            .iter()
            .map(|&i| items[i].task.status.clone())
            .collect()
    }

    #[test]
    fn kanban_tabs_include_all_statuses_and_ci() {
        let tabs = kanban_tabs();
        assert_eq!(tabs.len(), ALL_STATUSES.len() + 2);
        assert_eq!(tabs.first(), Some(&KanbanTab::All));
        assert_eq!(tabs.last(), Some(&KanbanTab::Ci));
        for (tab, status) in tabs[1..=ALL_STATUSES.len()].iter().zip(ALL_STATUSES.iter()) {
            assert_eq!(tab, &KanbanTab::Status(status.clone()));
        }
    }

    #[test]
    fn status_tabs_flatten_tasks_by_status() {
        let items = store_with(vec![
            plain("a", Status::Backlog),
            plain("b", Status::Ready),
            ci("c", Status::Ready),
            ci("d", Status::Running),
        ]);

        let all = visible_task_indices(&items, &KanbanTab::All);
        assert_eq!(all.len(), 4);

        let ready = visible_task_indices(&items, &KanbanTab::Status(Status::Ready));
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().all(|&i| items[i].task.status == Status::Ready));

        let running = statuses_of(&items, &KanbanTab::Status(Status::Running));
        assert_eq!(running, vec![Status::Running]);

        let done = visible_task_indices(&items, &KanbanTab::Status(Status::Done));
        assert!(done.is_empty());
    }

    #[test]
    fn ci_is_a_special_kanban_tab() {
        let items = store_with(vec![
            plain("a", Status::Backlog),
            plain("b", Status::Ready),
            ci("c", Status::Ready),
            ci("d", Status::Running),
        ]);

        let mut ci_statuses = statuses_of(&items, &KanbanTab::Ci);
        ci_statuses.sort_by_key(status_index);
        assert_eq!(ci_statuses, vec![Status::Ready, Status::Running]);
    }

    #[test]
    fn kanban_tabs_cycle_forward_and_backward() {
        assert_eq!(
            next_kanban_tab(&KanbanTab::All),
            KanbanTab::Status(Status::Backlog)
        );
        assert_eq!(prev_kanban_tab(&KanbanTab::All), KanbanTab::Ci);
        assert_eq!(
            next_kanban_tab(&KanbanTab::Status(Status::Cancelled)),
            KanbanTab::Ci
        );
        assert_eq!(
            prev_kanban_tab(&KanbanTab::Ci),
            KanbanTab::Status(Status::Cancelled)
        );
    }
}
