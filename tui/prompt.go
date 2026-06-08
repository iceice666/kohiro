package tui

import (
	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// inputModel wraps a single-line textinput in a styled modal box.
// Dimension-agnostic: the embedder centers via lipgloss.Place.
type inputModel struct {
	ti    textinput.Model
	label string
	hint  string
}

func newInput(label, hint, placeholder string) inputModel {
	ti := textinput.New()
	ti.Placeholder = placeholder
	ti.CharLimit = 4096 // large enough for an OpenSSH public key line
	ti.Width = 60
	return inputModel{ti: ti, label: label, hint: hint}
}

// Focus must use a pointer receiver so m.ti.Focus() (itself a pointer receiver)
// persists the focus=true state on the stored textinput.Model.
func (m *inputModel) Focus() tea.Cmd { return m.ti.Focus() }
func (m inputModel) Value() string   { return m.ti.Value() }

func (m inputModel) Update(msg tea.Msg) (inputModel, tea.Cmd) {
	var cmd tea.Cmd
	m.ti, cmd = m.ti.Update(msg)
	return m, cmd
}

func (m inputModel) View() string {
	hintLine := styleKey.Render("enter") + styleFooter.Render(": confirm   ") +
		styleKey.Render("esc") + styleFooter.Render(": cancel")
	body := lipgloss.JoinVertical(
		lipgloss.Left,
		styleHeader.Render(m.label),
		m.ti.View(),
		styleFooter.Render(m.hint),
		hintLine,
	)
	return styleModal.Render(body)
}

// confirmModel is a stateless y/N prompt. Key handling lives in the embedder.
// Dimension-agnostic: the embedder centers via lipgloss.Place.
type confirmModel struct {
	prompt string
	detail string
}

func newConfirm(prompt, detail string) confirmModel {
	return confirmModel{prompt: prompt, detail: detail}
}

func (m confirmModel) View() string {
	hintLine := styleKey.Render("y") + styleFooter.Render(": yes   ") +
		styleKey.Render("n") + styleFooter.Render("/") +
		styleKey.Render("esc") + styleFooter.Render(": no")
	lines := []string{styleHeader.Render(m.prompt)}
	if m.detail != "" {
		lines = append(lines, styleFooter.Render(m.detail))
	}
	lines = append(lines, hintLine)
	return styleModal.Render(lipgloss.JoinVertical(lipgloss.Left, lines...))
}
