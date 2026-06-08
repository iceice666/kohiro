package tui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/bubbletea"
	"github.com/muesli/termenv"

	"github.com/iceice666/kohiro/auth"
	"github.com/iceice666/kohiro/issues"
	"github.com/iceice666/kohiro/store"
)

func Middleware(st *store.Store, hooks *auth.Hooks) wish.Middleware {
	client := &issues.Client{Binary: "git-bug"}
	handler := func(sess ssh.Session) (tea.Model, []tea.ProgramOption) {
		if _, _, isPty := sess.Pty(); !isPty {
			if len(sess.Command()) == 0 {
				wish.WriteString(sess, "kohiro: interactive TUI requires a PTY (use `ssh -t`). For git use `git clone`.\n")
			}
			return nil, nil
		}
		user := hooks.UserFromSession(sess)
		return NewRoot(st, hooks, user, client, sess), bubbletea.MakeOptions(sess)
	}
	return bubbletea.MiddlewareWithColorProfile(handler, termenv.ANSI256)
}
