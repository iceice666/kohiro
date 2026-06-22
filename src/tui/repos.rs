//! Repos pane: list of accessible repositories with create / delete / toggle
//! actions. Ported from the Go `tui/repos.go`.

use std::sync::Arc;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState};
use ratatui::Frame;

use super::input::{Key, TextInput};
use super::{PaneOutcome, GREEN, SUBTEXT, YELLOW};
use crate::auth;
use crate::git;
use crate::paths::Paths;
use crate::store::{RepoListing, Store, User};

enum ReposMode {
    List,
    Create,
    ConfirmDelete,
    ConfirmToggle,
}

pub(crate) struct ReposPane {
    store: Arc<Store>,
    paths: Arc<Paths>,
    user: Option<User>,
    items: Vec<RepoListing>,
    state: ListState,
    mode: ReposMode,
    input: TextInput,
    pending_owner: String,
    pending_name: String,
    pending_visibility: bool,
    toast: Option<(String, bool)>,
}

impl ReposPane {
    pub(crate) fn new(store: Arc<Store>, paths: Arc<Paths>, user: Option<User>) -> Self {
        Self {
            store,
            paths,
            user,
            items: Vec::new(),
            state: ListState::default(),
            mode: ReposMode::List,
            input: TextInput::default(),
            pending_owner: String::new(),
            pending_name: String::new(),
            pending_visibility: false,
            toast: None,
        }
    }

    pub(crate) fn is_modal(&self) -> bool {
        !matches!(self.mode, ReposMode::List)
    }

    pub(crate) fn load(&mut self) {
        let result = match self.user.as_ref() {
            Some(user) => self.store.list_repos_for_user(user.id),
            None => self.store.list_public_repos(),
        };
        match result {
            Ok(items) => {
                self.items = items;
                self.fix_selection();
            }
            Err(err) => {
                self.items.clear();
                self.state.select(None);
                self.toast = Some((err.to_string(), true));
            }
        }
    }

    fn fix_selection(&mut self) {
        if self.items.is_empty() {
            self.state.select(None);
        } else {
            let sel = self.state.selected().unwrap_or(0).min(self.items.len() - 1);
            self.state.select(Some(sel));
        }
    }

    fn selected(&self) -> Option<&RepoListing> {
        self.state.selected().and_then(|i| self.items.get(i))
    }

    fn owns(&self, owner: &str) -> bool {
        self.user.as_ref().is_some_and(|u| u.username == owner)
            && auth::can_write_in_namespace(self.user.as_ref(), owner)
    }

    pub(crate) fn update(&mut self, key: Key) -> PaneOutcome {
        match self.mode {
            ReposMode::List => self.update_list(key),
            ReposMode::Create => self.update_create(key),
            ReposMode::ConfirmDelete => self.update_confirm_delete(key),
            ReposMode::ConfirmToggle => self.update_confirm_toggle(key),
        }
    }

    fn update_list(&mut self, key: Key) -> PaneOutcome {
        if super::handle_nav(&mut self.state, self.items.len(), &key) {
            return PaneOutcome::Redraw;
        }
        match key {
            Key::Enter => match self.selected().cloned() {
                Some(item) => PaneOutcome::OpenRepo {
                    owner: item.owner_username,
                    name: item.name,
                },
                None => PaneOutcome::Ignore,
            },
            Key::Char('n') => {
                if self.user.is_none() {
                    self.toast = Some(("sign in to create repos".into(), true));
                    return PaneOutcome::Redraw;
                }
                self.mode = ReposMode::Create;
                self.input.clear();
                self.toast = None;
                PaneOutcome::Redraw
            }
            Key::Char('d') | Key::Char('x') => {
                let Some(item) = self.selected().cloned() else {
                    return PaneOutcome::Ignore;
                };
                if !self.owns(&item.owner_username) {
                    self.toast = Some(("not your repo".into(), true));
                    return PaneOutcome::Redraw;
                }
                self.pending_owner = item.owner_username;
                self.pending_name = item.name;
                self.mode = ReposMode::ConfirmDelete;
                self.toast = None;
                PaneOutcome::Redraw
            }
            Key::Char('p') => {
                let Some(item) = self.selected().cloned() else {
                    return PaneOutcome::Ignore;
                };
                if !self.owns(&item.owner_username) {
                    self.toast = Some(("not your repo".into(), true));
                    return PaneOutcome::Redraw;
                }
                self.pending_owner = item.owner_username;
                self.pending_name = item.name;
                self.pending_visibility = !item.public;
                self.mode = ReposMode::ConfirmToggle;
                self.toast = None;
                PaneOutcome::Redraw
            }
            _ => PaneOutcome::Ignore,
        }
    }

    fn update_create(&mut self, key: Key) -> PaneOutcome {
        match key {
            Key::Enter => {
                let name = self.input.value.trim().to_owned();
                self.mode = ReposMode::List;
                if !valid_repo_name(&name) {
                    self.toast = Some((
                        "invalid name: lowercase, digits, . _ -, start alnum, max 64".into(),
                        true,
                    ));
                    return PaneOutcome::Redraw;
                }
                let (username, uid) = match self.user.as_ref() {
                    Some(user) => (user.username.clone(), user.id),
                    None => return PaneOutcome::Redraw,
                };
                let existed = matches!(self.store.get_repo(&username, &name), Ok(Some(_)));
                if let Err(err) = self.store.ensure_repo(uid, &name) {
                    self.toast = Some((err.to_string(), true));
                    self.load();
                    return PaneOutcome::Redraw;
                }
                if let Err(err) = git::ensure_bare(&self.paths.repo_path(&username, &name)) {
                    self.toast = Some((err.to_string(), true));
                    self.load();
                    return PaneOutcome::Redraw;
                }
                self.toast = Some((
                    if existed {
                        "repo already exists".into()
                    } else {
                        format!("created {name}")
                    },
                    false,
                ));
                self.load();
                PaneOutcome::Redraw
            }
            Key::Esc => {
                self.mode = ReposMode::List;
                PaneOutcome::Redraw
            }
            other => {
                self.input.handle(&other);
                PaneOutcome::Redraw
            }
        }
    }

    fn update_confirm_delete(&mut self, key: Key) -> PaneOutcome {
        match key {
            Key::Char('y') => {
                let owner = self.pending_owner.clone();
                let name = self.pending_name.clone();
                self.mode = ReposMode::List;
                match self.store.delete_repo(&owner, &name) {
                    Err(err) => self.toast = Some((err.to_string(), true)),
                    Ok(()) => match git::delete(&self.paths.repo_path(&owner, &name)) {
                        Ok(()) => self.toast = Some(("deleted".into(), false)),
                        Err(_) => {
                            self.toast = Some(("deleted (warning: disk dir remained)".into(), true))
                        }
                    },
                }
                self.load();
                PaneOutcome::Redraw
            }
            Key::Char('n') | Key::Esc => {
                self.mode = ReposMode::List;
                PaneOutcome::Redraw
            }
            _ => PaneOutcome::Ignore,
        }
    }

    fn update_confirm_toggle(&mut self, key: Key) -> PaneOutcome {
        match key {
            Key::Char('y') => {
                let owner = self.pending_owner.clone();
                let name = self.pending_name.clone();
                let target = self.pending_visibility;
                self.mode = ReposMode::List;
                match self.store.set_public(&owner, &name, target) {
                    Err(err) => self.toast = Some((err.to_string(), true)),
                    Ok(()) => self.toast = Some((format!("now {}", visibility_str(target)), false)),
                }
                self.load();
                PaneOutcome::Redraw
            }
            Key::Char('n') | Key::Esc => {
                self.mode = ReposMode::List;
                PaneOutcome::Redraw
            }
            _ => PaneOutcome::Ignore,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        let items: Vec<ListItem> = self.items.iter().map(repo_item).collect();
        super::render_list(f, rows[0], items, &self.state);

        let hint =
            "↑↓ move · enter open · n new · d/x delete · p toggle · tab switch · ctrl+c quit";
        super::render_footer(f, rows[1], self.toast.as_ref(), hint);

        match self.mode {
            ReposMode::List => {}
            ReposMode::Create => {
                let username = self
                    .user
                    .as_ref()
                    .map(|u| u.username.as_str())
                    .unwrap_or("");
                let title = format!("Create repo in {username}/");
                let lines = vec![
                    Line::from(Span::styled(
                        "Allowed: lowercase, digits, . _ - (max 64).",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(super::input_line(&self.input.value)),
                    Line::from(Span::styled(
                        "enter: confirm   esc: cancel",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, &title, lines);
            }
            ReposMode::ConfirmDelete => {
                let lines = vec![
                    Line::from(format!(
                        "Delete {}/{}?",
                        self.pending_owner, self.pending_name
                    )),
                    Line::from(Span::styled(
                        "Removes the bare repo on disk. Cannot be undone.",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(Span::styled(
                        "y: yes   n/esc: no",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "Confirm delete", lines);
            }
            ReposMode::ConfirmToggle => {
                let lines = vec![
                    Line::from(format!(
                        "Make {}/{} {}?",
                        self.pending_owner,
                        self.pending_name,
                        visibility_str(self.pending_visibility)
                    )),
                    Line::from(Span::styled(
                        "y: yes   n/esc: no",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "Confirm visibility", lines);
            }
        }
    }
}

fn repo_item(r: &RepoListing) -> ListItem<'static> {
    let title = Line::from(format!("{}/{}", r.owner_username, r.name));
    let (label, color) = if r.public {
        ("● public", GREEN)
    } else {
        ("● private", YELLOW)
    };
    let desc = Line::from(Span::styled(label, Style::default().fg(color)));
    ListItem::new(vec![title, desc])
}

fn visibility_str(public: bool) -> &'static str {
    if public {
        "public"
    } else {
        "private"
    }
}

/// `^[a-z0-9][a-z0-9._-]{0,63}$` without pulling in a regex dependency.
fn valid_repo_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let is_lower_alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !is_lower_alnum(bytes[0]) {
        return false;
    }
    bytes
        .iter()
        .all(|&c| is_lower_alnum(c) || c == b'.' || c == b'_' || c == b'-')
}
