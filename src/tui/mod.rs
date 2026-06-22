//! Interactive SSH TUI, rendered with ratatui over the russh PTY channel.
//!
//! Rendering is inline in the russh handler: [`Tui`] owns a ratatui
//! [`Terminal`] backed by a [`TerminalHandle`] that ships output over the
//! channel, plus a [`RootModel`]. Each input/resize event mutates the model and
//! redraws. No separate event loop.

pub mod input;

mod detail;
mod keys;
mod repos;

use std::io::Write;
use std::sync::Arc;

use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use russh::ChannelId;
use russh::server::Handle;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::auth;
use crate::paths::Paths;
use crate::store::{Store, User};

use detail::RepoDetail;
use input::Key;
use keys::KeysPane;
use repos::ReposPane;

// --- Catppuccin palette (ported from the Go tui/styles.go) ---------------

pub(crate) const PURPLE: Color = Color::Rgb(0xCB, 0xA6, 0xF7);
pub(crate) const BLUE: Color = Color::Rgb(0x89, 0xB4, 0xFA);
pub(crate) const GREEN: Color = Color::Rgb(0xA6, 0xE3, 0xA1);
pub(crate) const RED: Color = Color::Rgb(0xF3, 0x8B, 0xA8);
pub(crate) const YELLOW: Color = Color::Rgb(0xF9, 0xE2, 0xAF);
pub(crate) const TEAL: Color = Color::Rgb(0x94, 0xE2, 0xD5);
pub(crate) const PEACH: Color = Color::Rgb(0xFA, 0xB3, 0x87);
pub(crate) const SUBTEXT: Color = Color::Rgb(0xA6, 0xAD, 0xC8);
pub(crate) const OVERLAY: Color = Color::Rgb(0x6C, 0x70, 0x86);

// --- Over-SSH terminal handle (verbatim from russh ratatui example) -------

struct TerminalHandle {
    sender: UnboundedSender<Vec<u8>>,
    // The sink collects the data which is finally sent to sender.
    sink: Vec<u8>,
}

impl TerminalHandle {
    async fn start(handle: Handle, channel_id: ChannelId) -> Self {
        let (sender, mut receiver) = unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                let result = handle.data(channel_id, data.into()).await;
                if result.is_err() {
                    log::debug!("tui: failed to send data to channel");
                    break;
                }
            }
        });
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

impl std::io::Write for TerminalHandle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sink.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Err(err) = self.sender.send(self.sink.clone()) {
            return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, err));
        }
        self.sink.clear();
        Ok(())
    }
}

type SshTerminal = Terminal<CrosstermBackend<TerminalHandle>>;

// --- Public TUI handle owned by the russh connection ----------------------

pub struct Tui {
    handle: Handle,
    channel: ChannelId,
    terminal: SshTerminal,
    model: RootModel,
}

impl Tui {
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        handle: Handle,
        channel: ChannelId,
        store: Arc<Store>,
        paths: Arc<Paths>,
        user: Option<User>,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<Tui> {
        let term_handle = TerminalHandle::start(handle.clone(), channel).await;
        let backend = CrosstermBackend::new(term_handle);
        let viewport = Viewport::Fixed(Rect {
            x: 0,
            y: 0,
            width: cols.max(1),
            height: rows.max(1),
        });
        let mut terminal = Terminal::with_options(backend, TerminalOptions { viewport })?;
        // Enter alternate screen + hide cursor. Buffered into the same sink and
        // shipped on the first draw's flush, so ordering with draws is kept.
        write!(terminal.backend_mut(), "\x1b[?1049h\x1b[?25l")?;

        let mut model = RootModel::new(store, paths, user);
        model.load_initial();

        let mut tui = Tui {
            handle,
            channel,
            terminal,
            model,
        };
        tui.draw()?;
        Ok(tui)
    }

    pub fn channel(&self) -> ChannelId {
        self.channel
    }

    /// Feed a burst of channel bytes. Returns `true` when the session should
    /// quit (caller drops the `Tui`).
    pub async fn on_input(&mut self, bytes: &[u8]) -> anyhow::Result<bool> {
        for key in input::decode(bytes) {
            match self.model.update(key).await {
                Outcome::Quit => {
                    self.teardown().await;
                    return Ok(true);
                }
                Outcome::Redraw => self.draw()?,
                Outcome::Ignore => {}
            }
        }
        Ok(false)
    }

    pub async fn on_resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.terminal.resize(Rect {
            x: 0,
            y: 0,
            width: cols.max(1),
            height: rows.max(1),
        })?;
        self.draw()?;
        Ok(())
    }

    async fn teardown(&mut self) {
        // Restore cursor + leave alternate screen.
        let _ = write!(self.terminal.backend_mut(), "\x1b[?25h\x1b[?1049l");
        let _ = self.terminal.backend_mut().flush();
        let _ = self.handle.exit_status_request(self.channel, 0).await;
        let _ = self.handle.eof(self.channel).await;
        let _ = self.handle.close(self.channel).await;
    }

    fn draw(&mut self) -> anyhow::Result<()> {
        let model = &self.model;
        self.terminal.draw(|f| draw_root(f, model))?;
        Ok(())
    }
}

// --- Outcomes -------------------------------------------------------------

pub(crate) enum Outcome {
    Redraw,
    Quit,
    Ignore,
}

pub(crate) enum PaneOutcome {
    Redraw,
    Ignore,
    OpenRepo { owner: String, name: String },
}

pub(crate) enum DetailOutcome {
    Redraw,
    Ignore,
    Pop,
}

// --- Root model -----------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootTab {
    Repos,
    Keys,
}

pub(crate) struct RootModel {
    store: Arc<Store>,
    paths: Arc<Paths>,
    user: Option<User>,
    active: RootTab,
    repos: ReposPane,
    keys: KeysPane,
    detail: Option<RepoDetail>,
}

impl RootModel {
    fn new(store: Arc<Store>, paths: Arc<Paths>, user: Option<User>) -> Self {
        let repos = ReposPane::new(store.clone(), paths.clone(), user.clone());
        let keys = KeysPane::new(store.clone(), user.clone());
        Self {
            store,
            paths,
            user,
            active: RootTab::Repos,
            repos,
            keys,
            detail: None,
        }
    }

    fn load_initial(&mut self) {
        self.repos.load();
        self.keys.load();
    }

    async fn update(&mut self, key: Key) -> Outcome {
        if key == Key::CtrlC {
            return Outcome::Quit;
        }

        if let Some(detail) = self.detail.as_mut() {
            return match detail.update(key).await {
                DetailOutcome::Pop => {
                    self.detail = None;
                    Outcome::Redraw
                }
                DetailOutcome::Redraw => Outcome::Redraw,
                DetailOutcome::Ignore => Outcome::Ignore,
            };
        }

        let active_modal = match self.active {
            RootTab::Repos => self.repos.is_modal(),
            RootTab::Keys => self.keys.is_modal(),
        };
        if key == Key::Tab && !active_modal {
            self.active = match self.active {
                RootTab::Repos => RootTab::Keys,
                RootTab::Keys => RootTab::Repos,
            };
            return Outcome::Redraw;
        }

        let outcome = match self.active {
            RootTab::Repos => self.repos.update(key),
            RootTab::Keys => self.keys.update(key),
        };
        match outcome {
            PaneOutcome::OpenRepo { owner, name } => {
                if auth::can_read(&self.store, self.user.as_ref(), &owner, &name) {
                    let detail = RepoDetail::new(
                        owner,
                        name,
                        self.store.clone(),
                        self.paths.clone(),
                        self.user.clone(),
                    )
                    .await;
                    self.detail = Some(detail);
                    Outcome::Redraw
                } else {
                    Outcome::Ignore
                }
            }
            PaneOutcome::Redraw => Outcome::Redraw,
            PaneOutcome::Ignore => Outcome::Ignore,
        }
    }
}

// --- Rendering ------------------------------------------------------------

pub(crate) fn draw_root(f: &mut Frame, m: &RootModel) {
    let area = f.area();
    if let Some(detail) = m.detail.as_ref() {
        detail.render(f, area);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(area);

    render_header(f, rows[0], m);
    render_separator(f, rows[1]);

    match m.active {
        RootTab::Repos => m.repos.render(f, rows[2]),
        RootTab::Keys => m.keys.render(f, rows[2]),
    }
}

fn render_header(f: &mut Frame, area: Rect, m: &RootModel) {
    let user_label = m.user.as_ref().map(|u| format!("@{} ", u.username));
    let user_width = user_label
        .as_deref()
        .map_or(0, |s| s.chars().count() as u16);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(user_width)])
        .split(area);

    let mut spans = vec![
        Span::styled(
            " kohiro",
            Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(OVERLAY)),
    ];
    for (label, tab) in [("Repos", RootTab::Repos), ("Keys", RootTab::Keys)] {
        let style = if m.active == tab {
            Style::default()
                .fg(PURPLE)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(SUBTEXT)
        };
        spans.push(Span::styled(format!(" {label} "), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), cols[0]);

    if let Some(label) = user_label {
        f.render_widget(
            Paragraph::new(Span::styled(label, Style::default().fg(SUBTEXT)))
                .alignment(Alignment::Right),
            cols[1],
        );
    }
}

fn render_separator(f: &mut Frame, area: Rect) {
    let line = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(line).style(Style::default().fg(OVERLAY)),
        area,
    );
}

// --- Shared render/input helpers used by panes ----------------------------

pub(crate) fn move_selection(state: &mut ListState, len: usize, key: &Key) {
    if len == 0 {
        state.select(None);
        return;
    }
    let cur = state.selected().unwrap_or(0).min(len - 1);
    let next = match key {
        Key::Up => cur.saturating_sub(1),
        Key::Down => (cur + 1).min(len - 1),
        Key::PageUp => cur.saturating_sub(10),
        Key::PageDown => (cur + 10).min(len - 1),
        _ => cur,
    };
    state.select(Some(next));
}

/// Apply list navigation for the common key set. Returns `true` if `key` was a
/// navigation key (so the caller redraws).
pub(crate) fn handle_nav(state: &mut ListState, len: usize, key: &Key) -> bool {
    match key {
        Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
            move_selection(state, len, key);
            true
        }
        Key::Char('k') => {
            move_selection(state, len, &Key::Up);
            true
        }
        Key::Char('j') => {
            move_selection(state, len, &Key::Down);
            true
        }
        Key::Char('g') => {
            if len > 0 {
                state.select(Some(0));
            }
            true
        }
        Key::Char('G') => {
            if len > 0 {
                state.select(Some(len - 1));
            }
            true
        }
        _ => false,
    }
}

pub(crate) fn render_list(
    f: &mut Frame,
    area: Rect,
    title: &str,
    empty_message: &str,
    items: Vec<ListItem>,
    state: &ListState,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(OVERLAY))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ));

    if items.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(empty_message.to_owned(), Style::default().fg(SUBTEXT)),
            ]))
            .block(block)
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD))
        .highlight_symbol("› ");
    let mut st = state.clone();
    f.render_stateful_widget(list, area, &mut st);
}

pub(crate) fn render_footer(f: &mut Frame, area: Rect, toast: Option<&(String, bool)>, hint: &str) {
    let line = match toast {
        Some((msg, true)) => Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        )),
        Some((msg, false)) => Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        )),
        None => Line::from(Span::styled(hint.to_owned(), Style::default().fg(SUBTEXT))),
    };
    f.render_widget(Paragraph::new(line), area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// Render a centered, bordered modal box with a title and body lines.
pub(crate) fn render_modal(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let inner_w = lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(title.chars().count());
    let width = (inner_w as u16 + 4).clamp(24, area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = centered_rect(width, height, area);
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(PURPLE))
        .title(Span::styled(
            title.to_owned(),
            Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rect,
    );
}

/// Cursor glyph appended to a focused text input's value.
pub(crate) fn input_line(value: &str) -> String {
    format!("{value}▏")
}
