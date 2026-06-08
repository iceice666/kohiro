package tui

import (
	"fmt"
	"log"
	"regexp"
	"strings"

	"github.com/charmbracelet/bubbles/list"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/iceice666/kohiro/auth"
	kohirogit "github.com/iceice666/kohiro/git"
	"github.com/iceice666/kohiro/store"
)

type reposMode int

const (
	reposModeList reposMode = iota
	reposModeCreate
	reposModeConfirmDelete
	reposModeConfirmToggle
)

var repoNameRE = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{0,63}$`)

type repoItem struct {
	owner, name string
	public      bool
}

func (r repoItem) Title() string       { return r.owner + "/" + r.name }
func (r repoItem) Description() string {
	if r.public {
		return styleTagPublic.Render("● public")
	}
	return styleTagPrivate.Render("● private")
}
func (r repoItem) FilterValue() string { return r.owner + "/" + r.name }

func visibilityStr(public bool) string {
	if public {
		return "public"
	}
	return "private"
}

type reposLoadedMsg struct {
	items []list.Item
	err   error
}

type openRepoMsg struct{ owner, name string }

type createRepoResultMsg struct {
	err    error
	name   string
	existed bool
}

type deleteRepoResultMsg struct {
	err          error
	diskOrphaned bool
}

type toggleVisibilityResultMsg struct {
	err    error
	newPub bool
}

type reposModel struct {
	list  list.Model
	st    *store.Store
	hooks *auth.Hooks
	user  *store.User

	width, height int

	mode                 reposMode
	prompt               inputModel
	confirm              confirmModel
	pendingOwner         string
	pendingName          string
	pendingNewVisibility bool
	toast                string
	toastErr             bool
}

func newReposModel(st *store.Store, hooks *auth.Hooks, user *store.User, width, height int) reposModel {
	l := list.New(nil, newStyledDelegate(), width, height)
	l.Title = "Repositories"
	l.SetShowHelp(false)
	return reposModel{
		list: l, st: st, hooks: hooks, user: user,
		width: width, height: height,
	}
}

func (m reposModel) Init() tea.Cmd {
	return m.loadCmd()
}

func (m reposModel) loadCmd() tea.Cmd {
	return func() tea.Msg {
		var rows []store.RepoListing
		var err error
		if m.user == nil {
			rows, err = m.st.ListPublicRepos()
		} else {
			rows, err = m.st.ListReposForUser(m.user.ID)
		}
		if err != nil {
			return reposLoadedMsg{err: err}
		}
		items := make([]list.Item, len(rows))
		for i, r := range rows {
			items[i] = repoItem{owner: r.OwnerUsername, name: r.Name, public: r.Public}
		}
		return reposLoadedMsg{items: items}
	}
}

// IsModal reports whether the model is showing an input or confirmation overlay.
func (m reposModel) IsModal() bool { return m.mode != reposModeList }

func (m reposModel) Update(msg tea.Msg) (reposModel, tea.Cmd) {
	switch msg := msg.(type) {
	case reposLoadedMsg:
		if msg.err != nil {
			m.list.Title = fmt.Sprintf("Repositories (error: %v)", msg.err)
			return m, nil
		}
		cmd := m.list.SetItems(msg.items)
		return m, cmd

	case createRepoResultMsg:
		if msg.err != nil {
			m.toast, m.toastErr = msg.err.Error(), true
		} else if msg.existed {
			m.toast, m.toastErr = "repo already exists", false
		} else {
			m.toast, m.toastErr = "created "+msg.name, false
		}
		m.mode = reposModeList
		return m, m.loadCmd()

	case deleteRepoResultMsg:
		if msg.err != nil {
			m.toast, m.toastErr = msg.err.Error(), true
		} else if msg.diskOrphaned {
			m.toast, m.toastErr = "deleted (warning: disk dir remained)", true
		} else {
			m.toast, m.toastErr = "deleted", false
		}
		m.mode = reposModeList
		return m, m.loadCmd()

	case toggleVisibilityResultMsg:
		if msg.err != nil {
			m.toast, m.toastErr = msg.err.Error(), true
		} else {
			m.toast, m.toastErr = "now "+visibilityStr(msg.newPub), false
		}
		m.mode = reposModeList
		return m, m.loadCmd()

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	// Route non-key messages (e.g. cursor blink ticks) to the active sub-model.
	if m.mode == reposModeCreate {
		var cmd tea.Cmd
		m.prompt, cmd = m.prompt.Update(msg)
		return m, cmd
	}
	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m reposModel) handleKey(msg tea.KeyMsg) (reposModel, tea.Cmd) {
	switch m.mode {
	case reposModeList:
		// While the list filter is active, don't intercept mutation keys.
		if m.list.FilterState() == list.Filtering {
			var cmd tea.Cmd
			m.list, cmd = m.list.Update(msg)
			return m, cmd
		}
		switch msg.String() {
		case "enter":
			item, ok := m.list.SelectedItem().(repoItem)
			if ok {
				return m, func() tea.Msg { return openRepoMsg{owner: item.owner, name: item.name} }
			}
		case "n":
			if m.user == nil {
				m.toast, m.toastErr = "sign in to create repos", true
				return m, nil
			}
			m.prompt = newInput(
				"Create repo in "+m.user.Username+"/",
				"Allowed: lowercase letters, digits, . _ -. Max 64 chars.",
				"myrepo",
			)
			m.mode = reposModeCreate
			m.toast = ""
			return m, m.prompt.Focus()
		case "d", "x":
			item, ok := m.list.SelectedItem().(repoItem)
			if !ok {
				return m, nil
			}
			// Strict self-scope: TUI users act only in their own namespace.
			if m.user == nil || item.owner != m.user.Username || !m.hooks.CanWriteInNamespace(m.user, item.owner) {
				m.toast, m.toastErr = "not your repo", true
				return m, nil
			}
			m.pendingOwner, m.pendingName = item.owner, item.name
			m.confirm = newConfirm(
				"Delete "+item.owner+"/"+item.name+"?",
				"This removes the bare repo on disk. Cannot be undone.",
			)
			m.mode = reposModeConfirmDelete
			m.toast = ""
			return m, nil
		case "p":
			item, ok := m.list.SelectedItem().(repoItem)
			if !ok {
				return m, nil
			}
			if m.user == nil || item.owner != m.user.Username || !m.hooks.CanWriteInNamespace(m.user, item.owner) {
				m.toast, m.toastErr = "not your repo", true
				return m, nil
			}
			m.pendingOwner, m.pendingName = item.owner, item.name
			m.pendingNewVisibility = !item.public
			target := visibilityStr(!item.public)
			m.confirm = newConfirm(
				fmt.Sprintf("Make %s/%s %s?", item.owner, item.name, target),
				"",
			)
			m.mode = reposModeConfirmToggle
			m.toast = ""
			return m, nil
		}

	case reposModeCreate:
		switch msg.String() {
		case "enter":
			name := strings.TrimSpace(m.prompt.Value())
			if !repoNameRE.MatchString(name) {
				m.toast, m.toastErr = "invalid name: use lowercase, digits, . _ -, start with alnum, max 64 chars", true
				m.mode = reposModeList
				return m, nil
			}
			m.mode = reposModeList
			return m, createRepoCmd(m.st, m.user, name)
		case "esc":
			m.mode = reposModeList
			return m, nil
		}
		var cmd tea.Cmd
		m.prompt, cmd = m.prompt.Update(msg)
		return m, cmd

	case reposModeConfirmDelete:
		switch msg.String() {
		case "y":
			return m, deleteRepoCmd(m.st, m.pendingOwner, m.pendingName)
		case "n", "esc":
			m.mode = reposModeList
			return m, nil
		}

	case reposModeConfirmToggle:
		switch msg.String() {
		case "y":
			return m, toggleVisibilityCmd(m.st, m.pendingOwner, m.pendingName, m.pendingNewVisibility)
		case "n", "esc":
			m.mode = reposModeList
			return m, nil
		}
	}

	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m reposModel) View() string {
	switch m.mode {
	case reposModeCreate:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.prompt.View())
	case reposModeConfirmDelete, reposModeConfirmToggle:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.confirm.View())
	}
	var sb strings.Builder
	sb.WriteString(m.list.View())
	if m.toast != "" {
		style := styleToastOK
		if m.toastErr {
			style = styleToastError
		}
		sb.WriteString("\n")
		sb.WriteString(style.Render(m.toast))
	}
	return sb.String()
}

func (m *reposModel) setSize(w, h int) {
	m.width, m.height = w, h
	m.list.SetSize(w, h)
}

func createRepoCmd(st *store.Store, user *store.User, name string) tea.Cmd {
	return func() tea.Msg {
		_, err := st.GetRepo(user.Username, name)
		existed := err == nil

		if _, err := st.EnsureRepo(user.ID, name); err != nil {
			return createRepoResultMsg{err: err, name: name}
		}
		if err := kohirogit.Init(user.Username, name); err != nil {
			return createRepoResultMsg{err: err, name: name}
		}
		return createRepoResultMsg{name: name, existed: existed}
	}
}

func deleteRepoCmd(st *store.Store, owner, name string) tea.Cmd {
	return func() tea.Msg {
		if err := st.DeleteRepo(owner, name); err != nil {
			return deleteRepoResultMsg{err: err}
		}
		if err := kohirogit.Delete(owner, name); err != nil {
			log.Printf("delete repo %s/%s: disk cleanup: %v", owner, name, err)
			return deleteRepoResultMsg{diskOrphaned: true}
		}
		return deleteRepoResultMsg{}
	}
}

func toggleVisibilityCmd(st *store.Store, owner, name string, newPub bool) tea.Cmd {
	return func() tea.Msg {
		err := st.SetPublic(owner, name, newPub)
		return toggleVisibilityResultMsg{err: err, newPub: newPub}
	}
}
