package tui

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/list"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/iceice666/kohiro/auth"
	kohirogit "github.com/iceice666/kohiro/git"
	"github.com/iceice666/kohiro/issues"
	"github.com/iceice666/kohiro/store"
)

// — List item —

type issueListItem struct {
	id       string
	humanID  string
	title    string
	status   string
	author   string
	created  time.Time
	comments int
}

func (i issueListItem) Title() string {
	badge := styleCommitHash.Render("[" + i.status + "]")
	return badge + "  " + i.title
}

func (i issueListItem) Description() string {
	return styleCommitAuthor.Render(i.humanID) + "  " +
		styleCommitDate.Render(i.created.Format("2006-01-02")) +
		"  " + styleCommitAuthor.Render(i.author) +
		fmt.Sprintf("  %d comment(s)", i.comments)
}

func (i issueListItem) FilterValue() string { return i.title + " " + i.author }

// — Message types —

type issuesLoadedMsg struct {
	items []list.Item
	err   error
}

type issueDetailLoadedMsg struct {
	detail issues.BugDetail
	err    error
}

type issueCreatedMsg struct {
	err error
}

type issueCommentedMsg struct {
	err error
}

type issueClosedMsg struct {
	err error
}

// — Mode —

type issuesMode int

const (
	issuesModeList    issuesMode = iota
	issuesModeDetail             // viewport showing issue + comments
	issuesModeNew                // textareaModel (title+body)
	issuesModeComment            // textareaModel (body only)
	issuesModeClose              // confirmModel
)

// — Model —

type issuesModel struct {
	owner, name string
	st          *store.Store
	hooks       *auth.Hooks
	user        *store.User
	client      *issues.Client

	mode           issuesMode
	list           list.Model
	detailVP       viewport.Model
	selectedID     string
	selectedHuman  string
	selectedTitle  string
	selectedStatus string

	newForm      textareaModel
	commentForm  textareaModel
	closeConfirm confirmModel

	width, height int
	toast         string
	toastErr      bool
}

func newIssuesView(
	owner, name string,
	st *store.Store, hooks *auth.Hooks, user *store.User,
	client *issues.Client,
	w, h int,
) issuesModel {
	contentH := h - 3
	l := list.New(nil, newStyledDelegate(), w, contentH)
	l.SetShowTitle(false)
	l.SetShowHelp(false)

	vp := viewport.New(w, contentH)

	return issuesModel{
		owner:       owner,
		name:        name,
		st:          st,
		hooks:       hooks,
		user:        user,
		client:      client,
		list:        l,
		detailVP:    vp,
		newForm:     newTextarea("New Issue", "", "Title", "Describe the issue…"),
		commentForm: newTextarea("New Comment", "", "", "Leave a comment…"),
		width:       w,
		height:      h,
	}
}

func (m issuesModel) Init() tea.Cmd {
	return m.loadIssuesCmd()
}

func (m issuesModel) IsModal() bool {
	return m.mode == issuesModeNew || m.mode == issuesModeComment || m.mode == issuesModeClose
}

func (m *issuesModel) setSize(w, h int) {
	m.width, m.height = w, h
	contentH := h - 3
	m.list.SetSize(w, contentH)
	m.detailVP.Width = w
	m.detailVP.Height = contentH
}

// — Async commands —

func (m issuesModel) loadIssuesCmd() tea.Cmd {
	owner, name := m.owner, m.name
	client := m.client
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		bugs, err := client.List(ctx, kohirogit.RepoPath(owner, name))
		if err != nil {
			return issuesLoadedMsg{err: err}
		}
		items := make([]list.Item, len(bugs))
		for i, b := range bugs {
			items[i] = issueListItem{
				id:       b.ID,
				humanID:  b.HumanID,
				title:    b.Title,
				status:   b.Status,
				author:   b.Author,
				created:  b.Created,
				comments: b.Comments,
			}
		}
		return issuesLoadedMsg{items: items}
	}
}

func (m issuesModel) loadDetailCmd(id string) tea.Cmd {
	owner, name := m.owner, m.name
	client := m.client
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		detail, err := client.Show(ctx, kohirogit.RepoPath(owner, name), id)
		return issueDetailLoadedMsg{detail: detail, err: err}
	}
}

func (m issuesModel) ensureIdentity(ctx context.Context) (string, error) {
	repo, err := m.st.GetRepo(m.owner, m.name)
	if err != nil {
		return "", fmt.Errorf("get repo: %w", err)
	}
	if id, err := m.st.GetGitBugIdentity(m.user.ID, repo.ID); err == nil {
		return id, nil
	}
	email := fmt.Sprintf("%s@kohiro", m.user.Username)
	id, err := m.client.EnsureIdentity(ctx, kohirogit.RepoPath(m.owner, m.name), m.user.Username, email)
	if err != nil {
		return "", err
	}
	_ = m.st.PutGitBugIdentity(m.user.ID, repo.ID, id)
	return id, nil
}

func (m issuesModel) createIssueCmd(title, body string) tea.Cmd {
	owner, name := m.owner, m.name
	client := m.client
	model := m
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer cancel()
		gitBugID, err := model.ensureIdentity(ctx)
		if err != nil {
			return issueCreatedMsg{err: err}
		}
		_, err = client.New(ctx, kohirogit.RepoPath(owner, name), gitBugID, title, body)
		return issueCreatedMsg{err: err}
	}
}

func (m issuesModel) commentIssueCmd(id, body string) tea.Cmd {
	owner, name := m.owner, m.name
	client := m.client
	model := m
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer cancel()
		gitBugID, err := model.ensureIdentity(ctx)
		if err != nil {
			return issueCommentedMsg{err: err}
		}
		err = client.Comment(ctx, kohirogit.RepoPath(owner, name), gitBugID, id, body)
		return issueCommentedMsg{err: err}
	}
}

func (m issuesModel) closeIssueCmd(id string) tea.Cmd {
	owner, name := m.owner, m.name
	client := m.client
	model := m
	return func() tea.Msg {
		ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer cancel()
		gitBugID, err := model.ensureIdentity(ctx)
		if err != nil {
			return issueClosedMsg{err: err}
		}
		err = client.Close(ctx, kohirogit.RepoPath(owner, name), gitBugID, id)
		return issueClosedMsg{err: err}
	}
}

// — Update —

func (m issuesModel) Update(msg tea.Msg) (issuesModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.setSize(msg.Width, msg.Height)
		return m, nil

	case issuesLoadedMsg:
		if msg.err != nil {
			m.toast = msg.err.Error()
			m.toastErr = true
			return m, nil
		}
		cmd := m.list.SetItems(msg.items)
		m.toast = ""
		return m, cmd

	case issueDetailLoadedMsg:
		if msg.err != nil {
			m.toast = msg.err.Error()
			m.toastErr = true
			return m, nil
		}
		m.detailVP.SetContent(renderDetail(msg.detail))
		m.detailVP.GotoTop()
		m.selectedStatus = msg.detail.Status
		m.mode = issuesModeDetail
		return m, nil

	case issueCreatedMsg:
		if msg.err != nil {
			m.toast = "error: " + msg.err.Error()
			m.toastErr = true
			return m, nil
		}
		m.toast = "issue created"
		m.toastErr = false
		m.mode = issuesModeList
		return m, m.loadIssuesCmd()

	case issueCommentedMsg:
		if msg.err != nil {
			m.toast = "error: " + msg.err.Error()
			m.toastErr = true
			m.mode = issuesModeDetail
			return m, nil
		}
		m.toast = "comment added"
		m.toastErr = false
		return m, m.loadDetailCmd(m.selectedID)

	case issueClosedMsg:
		if msg.err != nil {
			m.toast = "error: " + msg.err.Error()
			m.toastErr = true
			m.mode = issuesModeDetail
			return m, nil
		}
		m.toast = "issue closed"
		m.toastErr = false
		return m, m.loadDetailCmd(m.selectedID)

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	// Forward to active sub-model.
	switch m.mode {
	case issuesModeDetail:
		var cmd tea.Cmd
		m.detailVP, cmd = m.detailVP.Update(msg)
		return m, cmd
	case issuesModeNew:
		var cmd tea.Cmd
		m.newForm, cmd = m.newForm.Update(msg)
		return m, cmd
	case issuesModeComment:
		var cmd tea.Cmd
		m.commentForm, cmd = m.commentForm.Update(msg)
		return m, cmd
	default:
		var cmd tea.Cmd
		m.list, cmd = m.list.Update(msg)
		return m, cmd
	}
}

func (m issuesModel) handleKey(msg tea.KeyMsg) (issuesModel, tea.Cmd) {
	switch m.mode {
	case issuesModeNew:
		// Ctrl+Enter submits.
		if msg.Type == tea.KeyCtrlJ || (msg.Type == tea.KeyEnter && msg.Alt) {
			title := strings.TrimSpace(m.newForm.TitleValue())
			body := strings.TrimSpace(m.newForm.BodyValue())
			if title == "" {
				m.toast = "title cannot be empty"
				m.toastErr = true
				return m, nil
			}
			m.newForm.Reset()
			m.mode = issuesModeList
			return m, m.createIssueCmd(title, body)
		}
		if key.Matches(msg, defaultKeyMap.Back) {
			m.newForm.Reset()
			m.mode = issuesModeList
			return m, nil
		}
		var cmd tea.Cmd
		m.newForm, cmd = m.newForm.Update(msg)
		return m, cmd

	case issuesModeComment:
		if msg.Type == tea.KeyCtrlJ || (msg.Type == tea.KeyEnter && msg.Alt) {
			body := strings.TrimSpace(m.commentForm.BodyValue())
			if body == "" {
				m.toast = "comment cannot be empty"
				m.toastErr = true
				return m, nil
			}
			id := m.selectedID
			m.commentForm.Reset()
			m.mode = issuesModeDetail
			return m, m.commentIssueCmd(id, body)
		}
		if key.Matches(msg, defaultKeyMap.Back) {
			m.commentForm.Reset()
			m.mode = issuesModeDetail
			return m, nil
		}
		var cmd tea.Cmd
		m.commentForm, cmd = m.commentForm.Update(msg)
		return m, cmd

	case issuesModeClose:
		if key.Matches(msg, defaultKeyMap.Yes) {
			id := m.selectedID
			m.mode = issuesModeDetail
			return m, m.closeIssueCmd(id)
		}
		if key.Matches(msg, defaultKeyMap.No) || key.Matches(msg, defaultKeyMap.Back) {
			m.mode = issuesModeDetail
			return m, nil
		}
		return m, nil

	case issuesModeDetail:
		if key.Matches(msg, defaultKeyMap.Back) {
			m.mode = issuesModeList
			m.selectedID = ""
			return m, nil
		}
		if key.Matches(msg, defaultKeyMap.Comment) {
			if !m.canWrite() {
				m.toast = "not enough permission"
				m.toastErr = true
				return m, nil
			}
			m.commentForm.Reset()
			m.mode = issuesModeComment
			return m, m.commentForm.Focus()
		}
		if key.Matches(msg, defaultKeyMap.Delete) {
			if !m.canWrite() {
				m.toast = "not enough permission"
				m.toastErr = true
				return m, nil
			}
			if m.selectedStatus == "closed" {
				m.toast = "already closed"
				m.toastErr = false
				return m, nil
			}
			m.closeConfirm = newConfirm(
				"Close issue "+m.selectedHuman+"?",
				m.selectedTitle,
			)
			m.mode = issuesModeClose
			return m, nil
		}
		var cmd tea.Cmd
		m.detailVP, cmd = m.detailVP.Update(msg)
		return m, cmd

	default: // issuesModeList
		if key.Matches(msg, defaultKeyMap.Enter) {
			item, ok := m.list.SelectedItem().(issueListItem)
			if !ok {
				return m, nil
			}
			m.selectedID = item.id
			m.selectedHuman = item.humanID
			m.selectedTitle = item.title
			m.selectedStatus = item.status
			return m, m.loadDetailCmd(item.id)
		}
		if key.Matches(msg, defaultKeyMap.Add) {
			if !m.canWrite() {
				m.toast = "not enough permission"
				m.toastErr = true
				return m, nil
			}
			m.newForm.Reset()
			m.mode = issuesModeNew
			return m, m.newForm.Focus()
		}
		var cmd tea.Cmd
		m.list, cmd = m.list.Update(msg)
		return m, cmd
	}
}

func (m issuesModel) canWrite() bool {
	return m.hooks.CanWrite(m.user, m.owner, m.name)
}

// — View —

func (m issuesModel) View() string {
	var sb strings.Builder

	switch m.mode {
	case issuesModeNew:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.newForm.View())

	case issuesModeComment:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.commentForm.View())

	case issuesModeClose:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.closeConfirm.View())

	case issuesModeDetail:
		sb.WriteString(m.detailVP.View())
		sb.WriteString("\n")
		hint := styleKey.Render("c") + styleFooter.Render(": comment   ") +
			styleKey.Render("d/x") + styleFooter.Render(": close   ") +
			styleKey.Render("esc") + styleFooter.Render(": back")
		if m.toast != "" {
			if m.toastErr {
				hint = styleToastError.Render(m.toast) + "   " + hint
			} else {
				hint = styleToastOK.Render(m.toast) + "   " + hint
			}
		}
		sb.WriteString(hint)
		return sb.String()
	}

	// List mode.
	sb.WriteString(m.list.View())
	sb.WriteString("\n")
	hint := styleKey.Render("n") + styleFooter.Render(": new   ") +
		styleKey.Render("enter") + styleFooter.Render(": open")
	if m.toast != "" {
		if m.toastErr {
			hint = styleToastError.Render(m.toast) + "   " + hint
		} else {
			hint = styleToastOK.Render(m.toast) + "   " + hint
		}
	}
	sb.WriteString(hint)
	return sb.String()
}

// renderDetail formats a BugDetail as a readable string for the viewport.
func renderDetail(d issues.BugDetail) string {
	var sb strings.Builder
	sb.WriteString(styleHeader.Render(d.Title))
	sb.WriteString("\n")
	sb.WriteString(styleCommitHash.Render(d.HumanID) + "  " +
		styleCommitAuthor.Render(d.Author) + "  " +
		styleCommitDate.Render(d.Created.Format("2006-01-02")) + "  " +
		"[" + d.Status + "]")
	sb.WriteString("\n\n")
	for i, c := range d.Comments {
		if i == 0 {
			sb.WriteString(c.Body)
		} else {
			sb.WriteString(styleSeparator.Render(strings.Repeat("─", 40)))
			sb.WriteString("\n")
			sb.WriteString(styleCommitAuthor.Render(c.Author) + "\n")
			sb.WriteString(c.Body)
		}
		sb.WriteString("\n\n")
	}
	return sb.String()
}
