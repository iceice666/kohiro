package tui

import "github.com/charmbracelet/bubbles/key"

type keyMap struct {
	Up    key.Binding
	Down  key.Binding
	Enter key.Binding
	Back  key.Binding
	Tab   key.Binding
	Quit  key.Binding
	Top   key.Binding
	Bot   key.Binding
	PgUp  key.Binding
	PgDn  key.Binding
	// Mutation bindings (Keys and Repos tabs).
	Add     key.Binding
	Delete  key.Binding
	Toggle  key.Binding
	Yes     key.Binding
	No      key.Binding
	Comment key.Binding // Issues sub-tab
}

var defaultKeyMap = keyMap{
	Up:    key.NewBinding(key.WithKeys("up", "k"), key.WithHelp("↑/k", "up")),
	Down:  key.NewBinding(key.WithKeys("down", "j"), key.WithHelp("↓/j", "down")),
	Enter: key.NewBinding(key.WithKeys("enter"), key.WithHelp("enter", "open/confirm")),
	Back:  key.NewBinding(key.WithKeys("esc"), key.WithHelp("esc", "back/cancel")),
	Tab:   key.NewBinding(key.WithKeys("tab"), key.WithHelp("tab", "switch tab")),
	// q removed: typing 'q' in a textinput would otherwise quit the session.
	Quit: key.NewBinding(key.WithKeys("ctrl+c"), key.WithHelp("ctrl+c", "quit")),
	Top:  key.NewBinding(key.WithKeys("g"), key.WithHelp("g", "top")),
	Bot:  key.NewBinding(key.WithKeys("G"), key.WithHelp("G", "bottom")),
	PgUp: key.NewBinding(key.WithKeys("pgup"), key.WithHelp("pgup", "page up")),
	PgDn: key.NewBinding(key.WithKeys("pgdown"), key.WithHelp("pgdn", "page down")),
	// Mutations.
	Add:     key.NewBinding(key.WithKeys("a", "n"), key.WithHelp("a/n", "add/new")),
	Delete:  key.NewBinding(key.WithKeys("d", "x"), key.WithHelp("d/x", "delete")),
	Toggle:  key.NewBinding(key.WithKeys("p"), key.WithHelp("p", "toggle public")),
	Yes:     key.NewBinding(key.WithKeys("y"), key.WithHelp("y", "yes")),
	No:      key.NewBinding(key.WithKeys("n", "esc"), key.WithHelp("n/esc", "no")),
	Comment: key.NewBinding(key.WithKeys("c"), key.WithHelp("c", "comment")),
}
