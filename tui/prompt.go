package tui

import (
	"github.com/charmbracelet/bubbles/textarea"
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

// textareaModel is a multi-field modal for issue create (title + body) and
// comment (body only, when titlePlaceholder is "").
// Ctrl+Enter submits; Esc cancels; Tab cycles title↔body.
type textareaFocus int

const (
	taFocusTitle textareaFocus = iota
	taFocusBody
)

type textareaModel struct {
	ti    textinput.Model
	ta    textarea.Model
	label string
	hint  string
	// When titlePlaceholder is empty the title input is hidden (comment mode).
	hasTitleField bool
	focus         textareaFocus
}

func newTextarea(label, hint, titlePlaceholder, bodyPlaceholder string) textareaModel {
	ti := textinput.New()
	ti.Placeholder = titlePlaceholder
	ti.CharLimit = 256
	ti.Width = 60

	ta := textarea.New()
	ta.Placeholder = bodyPlaceholder
	ta.SetWidth(60)
	ta.SetHeight(8)
	ta.CharLimit = 8192

	return textareaModel{
		ti:            ti,
		ta:            ta,
		label:         label,
		hint:          hint,
		hasTitleField: titlePlaceholder != "",
	}
}

func (m *textareaModel) Focus() tea.Cmd {
	if m.hasTitleField {
		m.focus = taFocusTitle
		m.ta.Blur()
		return m.ti.Focus()
	}
	m.focus = taFocusBody
	return m.ta.Focus()
}

func (m *textareaModel) Reset() {
	m.ti.Reset()
	m.ta.Reset()
}

func (m textareaModel) TitleValue() string { return m.ti.Value() }
func (m textareaModel) BodyValue() string  { return m.ta.Value() }

func (m textareaModel) Update(msg tea.Msg) (textareaModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.Type {
		case tea.KeyTab:
			if m.hasTitleField {
				if m.focus == taFocusTitle {
					m.focus = taFocusBody
					m.ti.Blur()
					return m, m.ta.Focus()
				}
				m.focus = taFocusTitle
				m.ta.Blur()
				return m, m.ti.Focus()
			}
		}
	}

	var cmds []tea.Cmd
	if m.focus == taFocusTitle {
		var cmd tea.Cmd
		m.ti, cmd = m.ti.Update(msg)
		cmds = append(cmds, cmd)
	} else {
		var cmd tea.Cmd
		m.ta, cmd = m.ta.Update(msg)
		cmds = append(cmds, cmd)
	}
	return m, tea.Batch(cmds...)
}

func (m textareaModel) View() string {
	hintLine := styleKey.Render("ctrl+enter") + styleFooter.Render(": submit   ") +
		styleKey.Render("esc") + styleFooter.Render(": cancel")
	if m.hasTitleField {
		hintLine = styleKey.Render("tab") + styleFooter.Render(": switch field   ") + hintLine
	}

	var parts []string
	parts = append(parts, styleHeader.Render(m.label))
	if m.hasTitleField {
		parts = append(parts, styleFooter.Render("Title"), m.ti.View())
	}
	parts = append(parts, styleFooter.Render("Body"), m.ta.View())
	if m.hint != "" {
		parts = append(parts, styleFooter.Render(m.hint))
	}
	parts = append(parts, hintLine)
	return styleModal.Render(lipgloss.JoinVertical(lipgloss.Left, parts...))
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
