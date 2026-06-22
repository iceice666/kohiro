//! Keys pane: list / add / remove the signed-in user's SSH keys. Ported from
//! the Go `tui/keys.go`.

use std::sync::Arc;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::input::{Key, TextInput};
use super::{PaneOutcome, SUBTEXT};
use crate::auth;
use crate::store::{SshKey, Store, StoreError, User};

enum KeysMode {
    List,
    Add,
    ConfirmDelete,
}

pub(crate) struct KeysPane {
    store: Arc<Store>,
    user: Option<User>,
    items: Vec<SshKey>,
    state: ListState,
    mode: KeysMode,
    input: TextInput,
    pending_delete: i64,
    toast: Option<(String, bool)>,
}

impl KeysPane {
    pub(crate) fn new(store: Arc<Store>, user: Option<User>) -> Self {
        Self {
            store,
            user,
            items: Vec::new(),
            state: ListState::default(),
            mode: KeysMode::List,
            input: TextInput::default(),
            pending_delete: 0,
            toast: None,
        }
    }

    pub(crate) fn is_modal(&self) -> bool {
        !matches!(self.mode, KeysMode::List)
    }

    pub(crate) fn load(&mut self) {
        let Some(user) = self.user.as_ref() else {
            return;
        };
        match self.store.list_keys_for_user(user.id) {
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

    fn selected(&self) -> Option<&SshKey> {
        self.state.selected().and_then(|i| self.items.get(i))
    }

    pub(crate) fn update(&mut self, key: Key) -> PaneOutcome {
        if self.user.is_none() {
            return PaneOutcome::Ignore;
        }
        match self.mode {
            KeysMode::List => self.update_list(key),
            KeysMode::Add => self.update_add(key),
            KeysMode::ConfirmDelete => self.update_confirm_delete(key),
        }
    }

    fn update_list(&mut self, key: Key) -> PaneOutcome {
        if super::handle_nav(&mut self.state, self.items.len(), &key) {
            return PaneOutcome::Redraw;
        }
        match key {
            Key::Char('a') => {
                self.mode = KeysMode::Add;
                self.input.clear();
                self.toast = None;
                PaneOutcome::Redraw
            }
            Key::Char('d') | Key::Char('x') => match self.selected() {
                Some(item) => {
                    self.pending_delete = item.id;
                    self.mode = KeysMode::ConfirmDelete;
                    self.toast = None;
                    PaneOutcome::Redraw
                }
                None => PaneOutcome::Ignore,
            },
            _ => PaneOutcome::Ignore,
        }
    }

    fn update_add(&mut self, key: Key) -> PaneOutcome {
        match key {
            Key::Enter => {
                let raw = self.input.value.trim().to_owned();
                self.mode = KeysMode::List;
                let uid = self.user.as_ref().map(|u| u.id).unwrap_or(0);
                match russh::keys::PublicKey::from_openssh(&raw) {
                    Err(err) => self.toast = Some((format!("parse key: {err}"), true)),
                    Ok(pk) => {
                        let fp = auth::fingerprint_of(&pk);
                        match self.store.add_key_strict(uid, &fp, pk.comment()) {
                            Ok(true) => self.toast = Some(("key already added".into(), false)),
                            Ok(false) => self.toast = Some(("key added".into(), false)),
                            Err(err) => self.toast = Some((err.to_string(), true)),
                        }
                    }
                }
                self.load();
                PaneOutcome::Redraw
            }
            Key::Esc => {
                self.mode = KeysMode::List;
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
                let uid = self.user.as_ref().map(|u| u.id).unwrap_or(0);
                let id = self.pending_delete;
                self.mode = KeysMode::List;
                match self.store.key_count(uid) {
                    Err(err) => self.toast = Some((err.to_string(), true)),
                    Ok(n) if n <= 1 => self.toast = Some((StoreError::LastKey.to_string(), true)),
                    Ok(_) => match self.store.remove_key(uid, id) {
                        Ok(()) => self.toast = Some(("key removed".into(), false)),
                        Err(err) => self.toast = Some((err.to_string(), true)),
                    },
                }
                self.load();
                PaneOutcome::Redraw
            }
            Key::Char('n') | Key::Esc => {
                self.mode = KeysMode::List;
                PaneOutcome::Redraw
            }
            _ => PaneOutcome::Ignore,
        }
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect) {
        if self.user.is_none() {
            f.render_widget(
                Paragraph::new("Sign in with a registered key to view your SSH keys.")
                    .style(Style::default().fg(SUBTEXT)),
                area,
            );
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);

        let items: Vec<ListItem> = self.items.iter().map(key_item).collect();
        super::render_list(f, rows[0], items, &self.state);

        let hint = "↑↓ move · a add · d/x delete · tab switch · ctrl+c quit";
        super::render_footer(f, rows[1], self.toast.as_ref(), hint);

        match self.mode {
            KeysMode::List => {}
            KeysMode::Add => {
                let lines = vec![
                    Line::from(Span::styled(
                        "Paste an OpenSSH public key (ssh-ed25519 AAAA... comment).",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(super::input_line(&self.input.value)),
                    Line::from(Span::styled(
                        "enter: confirm   esc: cancel",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "Add SSH key", lines);
            }
            KeysMode::ConfirmDelete => {
                let fp = self
                    .selected()
                    .map(|k| short_fp(&k.fingerprint))
                    .unwrap_or_default();
                let lines = vec![
                    Line::from(format!("Remove key {fp}?")),
                    Line::from(Span::styled(
                        "If this is your last key you will lose SSH access.",
                        Style::default().fg(SUBTEXT),
                    )),
                    Line::from(Span::styled(
                        "y: yes   n/esc: no",
                        Style::default().fg(SUBTEXT),
                    )),
                ];
                super::render_modal(f, area, "Remove key", lines);
            }
        }
    }
}

fn key_item(k: &SshKey) -> ListItem<'static> {
    let title = Line::from(short_fp(&k.fingerprint));
    let desc = Line::from(Span::styled(
        k.comment.clone(),
        Style::default().fg(SUBTEXT),
    ));
    ListItem::new(vec![title, desc])
}

fn short_fp(fp: &str) -> String {
    if fp.chars().count() > 24 {
        let tail: String = fp
            .chars()
            .rev()
            .take(23)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    } else {
        fp.to_owned()
    }
}
