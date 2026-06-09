package tui

import (
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/list"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"

	"github.com/iceice666/kohiro/auth"
	"github.com/iceice666/kohiro/ci"
	"github.com/iceice666/kohiro/store"
)

// — styles —

var (
	styleCISuccess = lipgloss.NewStyle().Foreground(colorGreen).Bold(true)
	styleCIFailed  = lipgloss.NewStyle().Foreground(colorRed).Bold(true)
	styleCIQueued  = lipgloss.NewStyle().Foreground(colorYellow)
	styleCIRunning = lipgloss.NewStyle().Foreground(colorBlue)
	styleCIError   = lipgloss.NewStyle().Foreground(colorRed)
)

// — list item —

type ciRunItem struct {
	run store.CIRun
}

func (r ciRunItem) Title() string {
	return ciStatusBadge(r.run.Status) + "  " +
		styleCommitHash.Render(fmt.Sprintf("#%d", r.run.ID)) +
		"  " + shortSHA(r.run.SHA) +
		"  " + styleCommitAuthor.Render(r.run.Ref)
}

func (r ciRunItem) Description() string {
	dur := ciDuration(r.run)
	return styleCommitDate.Render(r.run.QueuedAt.Format("2006-01-02 15:04")) + "  " + dur
}

func (r ciRunItem) FilterValue() string { return r.run.SHA + " " + r.run.Ref }

func ciStatusBadge(status string) string {
	switch status {
	case "success":
		return styleCISuccess.Render("✓")
	case "failed":
		return styleCIFailed.Render("✗")
	case "running":
		return styleCIRunning.Render("►")
	case "error":
		return styleCIError.Render("!")
	default:
		return styleCIQueued.Render("·")
	}
}

func shortSHA(sha string) string {
	if len(sha) > 7 {
		return sha[:7]
	}
	return sha
}

func ciDuration(run store.CIRun) string {
	if run.StartedAt == nil {
		return ""
	}
	end := time.Now()
	if run.FinishedAt != nil {
		end = *run.FinishedAt
	}
	d := end.Sub(*run.StartedAt).Round(time.Second)
	return styleCommitDate.Render(d.String())
}

// — message types —

type ciRunsLoadedMsg struct {
	items []list.Item
	err   error
}

type ciLogChunkMsg struct {
	offset    int64
	data      []byte
	status    string
	terminal  bool
	err       error
}

type ciTickMsg struct{ runID int64; offset int64 }

// — mode —

type ciMode int

const (
	ciModeList ciMode = iota
	ciModeLog
)

// — model —

type ciModel struct {
	owner, name string
	repoID      int64
	st          *store.Store
	hooks       *auth.Hooks
	user        *store.User

	mode       ciMode
	list       list.Model
	logVP      viewport.Model
	selectedID int64
	logContent strings.Builder

	width, height int
	toast         string
	toastErr      bool
}

func newCIView(
	owner, name string,
	repoID int64,
	st *store.Store, hooks *auth.Hooks, user *store.User,
	w, h int,
) ciModel {
	contentH := h - 3
	l := list.New(nil, newStyledDelegate(), w, contentH)
	l.SetShowTitle(false)
	l.SetShowHelp(false)

	vp := viewport.New(w, contentH)

	return ciModel{
		owner:   owner,
		name:    name,
		repoID:  repoID,
		st:      st,
		hooks:   hooks,
		user:    user,
		list:    l,
		logVP:   vp,
		width:   w,
		height:  h,
	}
}

func (m ciModel) Init() tea.Cmd {
	return m.loadRunsCmd()
}

func (m ciModel) IsModal() bool { return false }

func (m *ciModel) setSize(w, h int) {
	m.width, m.height = w, h
	contentH := h - 3
	m.list.SetSize(w, contentH)
	m.logVP.Width = w
	m.logVP.Height = contentH
}

// — async commands —

func (m ciModel) loadRunsCmd() tea.Cmd {
	repoID := m.repoID
	st := m.st
	return func() tea.Msg {
		runs, err := st.ListRunsForRepo(repoID, 50)
		if err != nil {
			return ciRunsLoadedMsg{err: err}
		}
		items := make([]list.Item, len(runs))
		for i, r := range runs {
			items[i] = ciRunItem{run: r}
		}
		return ciRunsLoadedMsg{items: items}
	}
}

func (m ciModel) loadLogChunkCmd(runID, offset int64) tea.Cmd {
	st := m.st
	return func() tea.Msg {
		logPath := fmt.Sprintf("%s/%d.log", ci.LogDir, runID)
		run, err := st.GetRun(runID)
		if err != nil {
			return ciLogChunkMsg{err: err}
		}

		data, n, err := readLogFrom(logPath, offset)
		if err != nil {
			return ciLogChunkMsg{err: err}
		}

		terminal := ciIsTerminal(run.Status)
		return ciLogChunkMsg{
			offset:   offset + n,
			data:     data,
			status:   run.Status,
			terminal: terminal,
		}
	}
}

func ciIsTerminal(status string) bool {
	switch status {
	case "success", "failed", "error":
		return true
	}
	return false
}

func readLogFrom(logPath string, offset int64) ([]byte, int64, error) {
	f, err := os.Open(logPath)
	if os.IsNotExist(err) {
		return nil, 0, nil
	}
	if err != nil {
		return nil, 0, err
	}
	defer f.Close()

	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return nil, 0, err
	}
	data, err := io.ReadAll(f)
	if err != nil {
		return nil, 0, err
	}
	return data, int64(len(data)), nil
}

func (m ciModel) tickCmd(runID, offset int64) tea.Cmd {
	return tea.Tick(500*time.Millisecond, func(time.Time) tea.Msg {
		return ciTickMsg{runID: runID, offset: offset}
	})
}

// — update —

func (m ciModel) Update(msg tea.Msg) (ciModel, tea.Cmd) {
	switch msg := msg.(type) {
	case ciRunsLoadedMsg:
		if msg.err != nil {
			m.toast = msg.err.Error()
			m.toastErr = true
			return m, nil
		}
		cmd := m.list.SetItems(msg.items)
		m.toast = ""
		return m, cmd

	case ciLogChunkMsg:
		if msg.err != nil {
			m.toast = msg.err.Error()
			m.toastErr = true
			return m, nil
		}
		if len(msg.data) > 0 {
			m.logContent.Write(msg.data)
			m.logVP.SetContent(m.logContent.String())
			m.logVP.GotoBottom()
		}
		if !msg.terminal {
			return m, m.tickCmd(m.selectedID, msg.offset)
		}
		return m, nil

	case ciTickMsg:
		if m.mode == ciModeLog && msg.runID == m.selectedID {
			return m, m.loadLogChunkCmd(msg.runID, msg.offset)
		}
		return m, nil

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	// Forward to active sub-view.
	if m.mode == ciModeLog {
		var cmd tea.Cmd
		m.logVP, cmd = m.logVP.Update(msg)
		return m, cmd
	}
	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m ciModel) handleKey(msg tea.KeyMsg) (ciModel, tea.Cmd) {
	switch m.mode {
	case ciModeLog:
		switch {
		case msg.Type == tea.KeyEsc:
			m.mode = ciModeList
			return m, m.loadRunsCmd()
		default:
			var cmd tea.Cmd
			m.logVP, cmd = m.logVP.Update(msg)
			return m, cmd
		}

	default: // ciModeList
		switch {
		case msg.Type == tea.KeyEnter:
			item, ok := m.list.SelectedItem().(ciRunItem)
			if !ok {
				return m, nil
			}
			m.selectedID = item.run.ID
			m.logContent.Reset()
			m.logVP.SetContent("")
			m.logVP.GotoTop()
			m.mode = ciModeLog
			return m, m.loadLogChunkCmd(item.run.ID, 0)
		default:
			var cmd tea.Cmd
			m.list, cmd = m.list.Update(msg)
			return m, cmd
		}
	}
}

// — view —

func (m ciModel) View() string {
	var sb strings.Builder

	if m.mode == ciModeLog {
		sb.WriteString(m.logVP.View())
		sb.WriteString("\n")
		hint := styleKey.Render("esc") + styleFooter.Render(": back   ") +
			styleKey.Render("↑/↓") + styleFooter.Render(": scroll")
		sb.WriteString(hint)
		return sb.String()
	}

	// List mode.
	sb.WriteString(m.list.View())
	sb.WriteString("\n")
	hint := styleKey.Render("enter") + styleFooter.Render(": view log")
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

