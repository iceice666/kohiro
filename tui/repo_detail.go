package tui

import (
	"errors"
	"fmt"
	"path"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/list"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"

	kohirogit "github.com/iceice666/kohiro/git"
)

type detailTab int

const (
	detailFiles   detailTab = iota
	detailCommits detailTab = iota
)

type fileEntry struct {
	name  string
	isDir bool
}

func (f fileEntry) Title() string {
	if f.isDir {
		return styleDir.Render(f.name + "/")
	}
	return f.name
}
func (f fileEntry) Description() string { return "" }
func (f fileEntry) FilterValue() string { return f.name }

type commitEntry struct {
	hash    string
	author  string
	subject string
	when    time.Time
}

func (c commitEntry) Title() string {
	return styleCommitHash.Render(c.hash) + "  " + c.subject
}
func (c commitEntry) Description() string {
	return styleCommitDate.Render(c.when.Format("2006-01-02")) + "  " + styleCommitAuthor.Render(c.author)
}
func (c commitEntry) FilterValue() string { return c.subject + " " + c.author }

type detailTreeLoadedMsg struct {
	items []list.Item
	err   error
}

type detailCommitsLoadedMsg struct {
	items []list.Item
	err   error
}

type detailBlobLoadedMsg struct {
	content   string
	err       error
	truncated bool
}

type repoDetailModel struct {
	owner, name string
	activeSub   detailTab
	width, height int

	currentPath string
	files       list.Model
	blobVP      viewport.Model
	blobOpen    bool
	blobErr     string

	commits list.Model

	errMsg string
}

func newRepoDetail(owner, name string, width, height int) (repoDetailModel, tea.Cmd) {
	contentH := height - 3 // room for tab bar + breadcrumb + footer

	fl := list.New(nil, newStyledDelegate(), width, contentH)
	fl.SetShowTitle(false)
	fl.SetShowHelp(false)

	cl := list.New(nil, newStyledDelegate(), width, contentH)
	cl.SetShowTitle(false)
	cl.SetShowHelp(false)

	vp := viewport.New(width, contentH)

	m := repoDetailModel{
		owner:  owner,
		name:   name,
		files:  fl,
		commits: cl,
		blobVP: vp,
		width:  width,
		height: height,
	}
	return m, tea.Batch(m.loadTreeCmd(""), m.loadCommitsCmd())
}

func (m repoDetailModel) loadTreeCmd(dirPath string) tea.Cmd {
	owner, name := m.owner, m.name
	return func() tea.Msg {
		repo, err := kohirogit.OpenRepo(owner, name)
		if err != nil {
			return detailTreeLoadedMsg{err: err}
		}
		tree, err := kohirogit.TreeAt(repo, dirPath)
		if err != nil {
			return detailTreeLoadedMsg{err: err}
		}
		if tree == nil {
			return detailTreeLoadedMsg{items: []list.Item{}}
		}
		var items []list.Item
		for _, e := range tree.Entries {
			items = append(items, fileEntry{name: e.Name, isDir: !e.Mode.IsFile()})
		}
		return detailTreeLoadedMsg{items: items}
	}
}

func (m repoDetailModel) loadCommitsCmd() tea.Cmd {
	owner, name := m.owner, m.name
	return func() tea.Msg {
		repo, err := kohirogit.OpenRepo(owner, name)
		if err != nil {
			return detailCommitsLoadedMsg{err: err}
		}
		commits, err := kohirogit.CommitLog(repo, 50)
		if err != nil {
			return detailCommitsLoadedMsg{err: err}
		}
		items := make([]list.Item, len(commits))
		for i, c := range commits {
			subject := firstLine(c.Message)
			hash := c.Hash.String()
			if len(hash) > 7 {
				hash = hash[:7]
			}
			items[i] = commitEntry{
				hash:    hash,
				author:  c.Author.Name,
				subject: subject,
				when:    c.Author.When,
			}
		}
		return detailCommitsLoadedMsg{items: items}
	}
}

func (m repoDetailModel) loadBlobCmd(filePath string) tea.Cmd {
	owner, name := m.owner, m.name
	return func() tea.Msg {
		repo, err := kohirogit.OpenRepo(owner, name)
		if err != nil {
			return detailBlobLoadedMsg{err: err}
		}
		data, truncated, err := kohirogit.Blob(repo, filePath, 256*1024)
		if errors.Is(err, kohirogit.ErrTooLarge) {
			return detailBlobLoadedMsg{content: "<file too large (> 1 MiB)>"}
		}
		if err != nil {
			return detailBlobLoadedMsg{err: err}
		}
		if kohirogit.IsBinary(data) {
			return detailBlobLoadedMsg{content: fmt.Sprintf("<binary file, %d bytes>", len(data)), truncated: truncated}
		}
		content := string(data)
		if truncated {
			content += "\n<truncated — showing first 256 KiB>"
		}
		return detailBlobLoadedMsg{content: content, truncated: truncated}
	}
}

func (m repoDetailModel) Init() tea.Cmd { return nil }

func (m repoDetailModel) Update(msg tea.Msg) (repoDetailModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width, m.height = msg.Width, msg.Height
		contentH := m.height - 3
		m.files.SetSize(m.width, contentH)
		m.commits.SetSize(m.width, contentH)
		m.blobVP.Width = m.width
		m.blobVP.Height = contentH

	case detailTreeLoadedMsg:
		if msg.err != nil {
			m.errMsg = msg.err.Error()
			return m, nil
		}
		cmd := m.files.SetItems(msg.items)
		return m, cmd

	case detailCommitsLoadedMsg:
		if msg.err != nil {
			// Non-fatal: just show empty commits list.
			return m, nil
		}
		cmd := m.commits.SetItems(msg.items)
		return m, cmd

	case detailBlobLoadedMsg:
		if msg.err != nil {
			m.blobErr = msg.err.Error()
			m.blobOpen = true
			m.blobVP.SetContent(styleError.Render("Error: " + msg.err.Error()))
		} else {
			m.blobVP.SetContent(msg.content)
			m.blobErr = ""
			m.blobOpen = true
		}
		return m, nil

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	// Forward to active sub-model.
	if m.blobOpen {
		var cmd tea.Cmd
		m.blobVP, cmd = m.blobVP.Update(msg)
		return m, cmd
	}
	if m.activeSub == detailFiles {
		var cmd tea.Cmd
		m.files, cmd = m.files.Update(msg)
		return m, cmd
	}
	var cmd tea.Cmd
	m.commits, cmd = m.commits.Update(msg)
	return m, cmd
}

func (m repoDetailModel) handleKey(msg tea.KeyMsg) (repoDetailModel, tea.Cmd) {
	switch {
	case key.Matches(msg, defaultKeyMap.Tab):
		if !m.blobOpen {
			if m.activeSub == detailFiles {
				m.activeSub = detailCommits
			} else {
				m.activeSub = detailFiles
			}
		}
		return m, nil

	case key.Matches(msg, defaultKeyMap.Back):
		if m.blobOpen {
			m.blobOpen = false
			return m, nil
		}
		if m.currentPath != "" {
			m.currentPath = path.Dir(m.currentPath)
			if m.currentPath == "." {
				m.currentPath = ""
			}
			return m, m.loadTreeCmd(m.currentPath)
		}
		// Signal root to pop detail — return special msg.
		return m, func() tea.Msg { return popDetailMsg{} }

	case key.Matches(msg, defaultKeyMap.Enter):
		if m.activeSub == detailFiles && !m.blobOpen {
			item, ok := m.files.SelectedItem().(fileEntry)
			if !ok {
				return m, nil
			}
			if item.isDir {
				m.currentPath = joinPath(m.currentPath, item.name)
				return m, m.loadTreeCmd(m.currentPath)
			}
			filePath := joinPath(m.currentPath, item.name)
			return m, m.loadBlobCmd(filePath)
		}
		return m, nil
	}

	// Forward to active sub-view.
	if m.blobOpen {
		var cmd tea.Cmd
		m.blobVP, cmd = m.blobVP.Update(msg)
		return m, cmd
	}
	if m.activeSub == detailFiles {
		var cmd tea.Cmd
		m.files, cmd = m.files.Update(msg)
		return m, cmd
	}
	var cmd tea.Cmd
	m.commits, cmd = m.commits.Update(msg)
	return m, cmd
}

func (m repoDetailModel) View() string {
	var sb strings.Builder

	// Tab bar.
	filesTab := styleTabInactive.Render("Files")
	commitsTab := styleTabInactive.Render("Commits")
	if m.activeSub == detailFiles {
		filesTab = styleTabActive.Render("Files")
	} else {
		commitsTab = styleTabActive.Render("Commits")
	}
	breadcrumb := styleBreadcrumb.Render(m.owner + "/" + m.name)
	if m.currentPath != "" {
		breadcrumb += styleBreadcrumbSep.Render(" › ") + styleBreadcrumb.Render(m.currentPath)
	}
	sb.WriteString(breadcrumb + "  " + filesTab + "  " + commitsTab + "\n")
	sb.WriteString(styleSeparator.Render(strings.Repeat("─", m.width)) + "\n")

	if m.errMsg != "" {
		sb.WriteString(styleError.Render("Error: " + m.errMsg))
		return sb.String()
	}

	if m.blobOpen {
		sb.WriteString(m.blobVP.View())
	} else if m.activeSub == detailFiles {
		sb.WriteString(m.files.View())
	} else {
		sb.WriteString(m.commits.View())
	}

	sb.WriteString("\n")
	hint := styleKey.Render("tab") + styleFooter.Render(": switch   ") +
		styleKey.Render("esc") + styleFooter.Render(": back   ") +
		styleKey.Render("ctrl+c") + styleFooter.Render(": quit")
	sb.WriteString(hint)
	return sb.String()
}

func (m *repoDetailModel) setSize(w, h int) {
	m.width, m.height = w, h
	contentH := h - 3
	m.files.SetSize(w, contentH)
	m.commits.SetSize(w, contentH)
	m.blobVP.Width = w
	m.blobVP.Height = contentH
}

type popDetailMsg struct{}

func joinPath(base, name string) string {
	if base == "" {
		return name
	}
	return base + "/" + name
}

func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}
