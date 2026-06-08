package tui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/list"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	gossh "golang.org/x/crypto/ssh"

	"github.com/iceice666/kohiro/auth"
	"github.com/iceice666/kohiro/store"
)

type keysMode int

const (
	keysModeList keysMode = iota
	keysModeAdd
	keysModeConfirmDelete
)

type keyItem struct {
	id          int64
	fingerprint string
	comment     string
}

func (k keyItem) Title() string {
	return shortFP(k.fingerprint)
}
func (k keyItem) Description() string { return k.comment }
func (k keyItem) FilterValue() string { return k.fingerprint + " " + k.comment }

type keysLoadedMsg struct {
	items []list.Item
	err   error
}

type addKeyResultMsg struct {
	err          error
	alreadyOwned bool
}

type removeKeyResultMsg struct {
	err error
}

type keysModel struct {
	list   list.Model
	st     *store.Store
	hooks  *auth.Hooks
	user   *store.User
	noUser bool

	width, height int

	mode             keysMode
	prompt           inputModel
	confirm          confirmModel
	pendingDeleteID  int64
	pendingDeleteFP  string
	toast            string
	toastErr         bool
}

func newKeysModel(st *store.Store, hooks *auth.Hooks, user *store.User, width, height int) keysModel {
	l := list.New(nil, newStyledDelegate(), width, height)
	l.Title = "SSH Keys"
	l.SetShowHelp(false)
	return keysModel{
		list: l, st: st, hooks: hooks, user: user,
		noUser: user == nil, width: width, height: height,
	}
}

func (m keysModel) Init() tea.Cmd {
	if m.noUser {
		return nil
	}
	return m.loadCmd()
}

func (m keysModel) loadCmd() tea.Cmd {
	return func() tea.Msg {
		keys, err := m.st.ListKeysForUser(m.user.ID)
		if err != nil {
			return keysLoadedMsg{err: err}
		}
		items := make([]list.Item, len(keys))
		for i, k := range keys {
			items[i] = keyItem{id: k.ID, fingerprint: k.Fingerprint, comment: k.Comment}
		}
		return keysLoadedMsg{items: items}
	}
}

// IsModal reports whether the model is showing an input or confirmation overlay.
func (m keysModel) IsModal() bool { return m.mode != keysModeList }

func (m keysModel) Update(msg tea.Msg) (keysModel, tea.Cmd) {
	switch msg := msg.(type) {
	case keysLoadedMsg:
		if msg.err != nil {
			m.list.Title = fmt.Sprintf("SSH Keys (error: %v)", msg.err)
			return m, nil
		}
		cmd := m.list.SetItems(msg.items)
		return m, cmd

	case addKeyResultMsg:
		if msg.err != nil {
			m.toast, m.toastErr = msg.err.Error(), true
		} else if msg.alreadyOwned {
			m.toast, m.toastErr = "key already added", false
		} else {
			m.toast, m.toastErr = "key added", false
		}
		m.mode = keysModeList
		return m, m.loadCmd()

	case removeKeyResultMsg:
		if msg.err != nil {
			m.toast, m.toastErr = msg.err.Error(), true
		} else {
			m.toast, m.toastErr = "key removed", false
		}
		m.mode = keysModeList
		return m, m.loadCmd()

	case tea.KeyMsg:
		return m.handleKey(msg)
	}

	// Route non-key messages (e.g. cursor blink ticks) to the active sub-model.
	if m.mode == keysModeAdd {
		var cmd tea.Cmd
		m.prompt, cmd = m.prompt.Update(msg)
		return m, cmd
	}
	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m keysModel) handleKey(msg tea.KeyMsg) (keysModel, tea.Cmd) {
	switch m.mode {
	case keysModeList:
		// While the list filter is active, don't intercept mutation keys.
		if m.list.FilterState() == list.Filtering {
			var cmd tea.Cmd
			m.list, cmd = m.list.Update(msg)
			return m, cmd
		}
		switch msg.String() {
		case "a":
			m.prompt = newInput(
				"Add SSH key",
				"Paste an OpenSSH public key (ssh-ed25519 AAA... comment).",
				"ssh-ed25519 AAAA...",
			)
			m.mode = keysModeAdd
			m.toast = ""
			return m, m.prompt.Focus()
		case "d", "x":
			if m.noUser {
				return m, nil
			}
			item, ok := m.list.SelectedItem().(keyItem)
			if !ok {
				return m, nil
			}
			m.pendingDeleteID = item.id
			m.pendingDeleteFP = item.fingerprint
			m.confirm = newConfirm(
				"Remove key "+shortFP(item.fingerprint)+"?",
				"If this is your last key you will lose SSH access.",
			)
			m.mode = keysModeConfirmDelete
			m.toast = ""
			return m, nil
		}

	case keysModeAdd:
		switch msg.String() {
		case "enter":
			raw := m.prompt.Value()
			m.mode = keysModeList
			return m, addKeyCmd(m.st, m.user.ID, raw)
		case "esc":
			m.mode = keysModeList
			return m, nil
		}
		var cmd tea.Cmd
		m.prompt, cmd = m.prompt.Update(msg)
		return m, cmd

	case keysModeConfirmDelete:
		switch msg.String() {
		case "y":
			return m, removeKeyCmd(m.st, m.user.ID, m.pendingDeleteID)
		case "n", "esc":
			m.mode = keysModeList
			return m, nil
		}
	}

	var cmd tea.Cmd
	m.list, cmd = m.list.Update(msg)
	return m, cmd
}

func (m keysModel) View() string {
	if m.noUser {
		return "Sign in with a registered key to view your SSH keys."
	}
	switch m.mode {
	case keysModeAdd:
		return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, m.prompt.View())
	case keysModeConfirmDelete:
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

func (m *keysModel) setSize(w, h int) {
	m.width, m.height = w, h
	m.list.SetSize(w, h)
}

func addKeyCmd(st *store.Store, userID int64, raw string) tea.Cmd {
	return func() tea.Msg {
		pk, comment, _, _, err := gossh.ParseAuthorizedKey([]byte(raw))
		if err != nil {
			return addKeyResultMsg{err: fmt.Errorf("parse key: %w", err)}
		}
		fp := gossh.FingerprintSHA256(pk)
		already, err := st.AddKeyStrict(userID, fp, comment)
		return addKeyResultMsg{err: err, alreadyOwned: already}
	}
}

func removeKeyCmd(st *store.Store, userID, keyID int64) tea.Cmd {
	return func() tea.Msg {
		n, err := st.KeyCount(userID)
		if err != nil {
			return removeKeyResultMsg{err: err}
		}
		if n <= 1 {
			return removeKeyResultMsg{err: store.ErrLastKey}
		}
		return removeKeyResultMsg{err: st.RemoveKey(userID, keyID)}
	}
}

func shortFP(fp string) string {
	if len(fp) > 24 {
		return "…" + fp[len(fp)-23:]
	}
	return fp
}
