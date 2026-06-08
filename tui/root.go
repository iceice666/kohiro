package tui

import (
	"strings"

	"github.com/charmbracelet/bubbles/key"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/ssh"

	"github.com/iceice666/kohiro/auth"
	"github.com/iceice666/kohiro/issues"
	"github.com/iceice666/kohiro/store"
)

type rootTab int

const (
	rootTabRepos rootTab = iota
	rootTabKeys
)

var rootTabNames = []string{"Repos", "Keys"}

type rootModel struct {
	st          *store.Store
	hooks       *auth.Hooks
	user        *store.User
	issueClient *issues.Client

	width, height int
	active        rootTab
	detail        *repoDetailModel

	repos reposModel
	keys  keysModel
}

func NewRoot(st *store.Store, hooks *auth.Hooks, user *store.User, issueClient *issues.Client, sess ssh.Session) *rootModel {
	pty, _, _ := sess.Pty()
	w, h := pty.Window.Width, pty.Window.Height
	if w == 0 {
		w = 80
	}
	if h == 0 {
		h = 24
	}
	contentH := h - 3 // header+tabs + separator + slack
	return &rootModel{
		st:          st,
		hooks:       hooks,
		user:        user,
		issueClient: issueClient,
		width:       w,
		height:      h,
		repos:       newReposModel(st, hooks, user, w, contentH),
		keys:        newKeysModel(st, hooks, user, w, contentH),
	}
}

func (m *rootModel) Init() tea.Cmd {
	return tea.Batch(m.repos.Init(), m.keys.Init())
}

func (m *rootModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		contentH := m.height - 3
		m.repos.setSize(m.width, contentH)
		m.keys.setSize(m.width, contentH)
		if m.detail != nil {
			m.detail.setSize(m.width, m.height)
		}
		return m, nil

	case reposLoadedMsg:
		var cmd tea.Cmd
		m.repos, cmd = m.repos.Update(msg)
		return m, cmd

	case keysLoadedMsg:
		var cmd tea.Cmd
		m.keys, cmd = m.keys.Update(msg)
		return m, cmd

	case createRepoResultMsg, deleteRepoResultMsg, toggleVisibilityResultMsg:
		var cmd tea.Cmd
		m.repos, cmd = m.repos.Update(msg)
		return m, cmd

	case addKeyResultMsg, removeKeyResultMsg:
		var cmd tea.Cmd
		m.keys, cmd = m.keys.Update(msg)
		return m, cmd

	case openRepoMsg:
		if m.hooks.CanRead(m.user, msg.owner, msg.name) {
			detail, cmd := newRepoDetail(msg.owner, msg.name,
				m.st, m.hooks, m.user, m.issueClient,
				m.width, m.height)
			m.detail = &detail
			return m, cmd
		}
		return m, nil

	case popDetailMsg:
		m.detail = nil
		return m, nil

	case tea.KeyMsg:
		if key.Matches(msg, defaultKeyMap.Quit) {
			return m, tea.Quit
		}
		if m.detail != nil {
			updated, cmd := m.detail.Update(msg)
			m.detail = &updated
			return m, cmd
		}
		// Don't cycle tabs while the active child has a modal overlay open
		// (textinput or confirm dialog): Tab should reach the input, not root.
		activeIsModal := false
		switch m.active {
		case rootTabRepos:
			activeIsModal = m.repos.IsModal()
		case rootTabKeys:
			activeIsModal = m.keys.IsModal()
		}
		if !activeIsModal && key.Matches(msg, defaultKeyMap.Tab) {
			m.active = (m.active + 1) % rootTab(len(rootTabNames))
			return m, nil
		}
		return m.delegateToActive(msg)
	}

	if m.detail != nil {
		updated, cmd := m.detail.Update(msg)
		m.detail = &updated
		return m, cmd
	}
	return m.delegateToActive(msg)
}

func (m *rootModel) delegateToActive(msg tea.Msg) (tea.Model, tea.Cmd) {
	var cmd tea.Cmd
	switch m.active {
	case rootTabRepos:
		m.repos, cmd = m.repos.Update(msg)
	case rootTabKeys:
		m.keys, cmd = m.keys.Update(msg)
	}
	return m, cmd
}

func (m *rootModel) View() string {
	if m.detail != nil {
		return m.detail.View()
	}

	var sb strings.Builder

	// Header: app name, tabs, username.
	appLabel := styleHeader.Render(" kohiro")
	divider := styleBreadcrumbSep.Render("  │  ")
	tabRow := ""
	for i, name := range rootTabNames {
		if rootTab(i) == m.active {
			tabRow += styleTabActive.Render(name)
		} else {
			tabRow += styleTabInactive.Render(name)
		}
	}
	userLabel := ""
	if m.user != nil {
		userLabel = styleFooter.Render("@" + m.user.Username + " ")
	}
	headerContent := appLabel + divider + tabRow
	gap := m.width - lipgloss.Width(headerContent) - lipgloss.Width(userLabel)
	if gap < 0 {
		gap = 0
	}
	sb.WriteString(headerContent + strings.Repeat(" ", gap) + userLabel)
	sb.WriteString("\n")
	sb.WriteString(styleSeparator.Render(strings.Repeat("─", m.width)))
	sb.WriteString("\n")

	// Content.
	switch m.active {
	case rootTabRepos:
		sb.WriteString(m.repos.View())
	case rootTabKeys:
		sb.WriteString(m.keys.View())
	}

	return sb.String()
}
