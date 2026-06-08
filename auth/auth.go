package auth

import (
	"errors"
	"log"
	"strings"

	"github.com/charmbracelet/ssh"
	wishgit "github.com/charmbracelet/wish/git"
	gossh "golang.org/x/crypto/ssh"

	"github.com/iceice666/kohiro/store"
)

// Hooks implements wishgit.Hooks using the SQLite store.
type Hooks struct {
	st *store.Store
}

func New(s *store.Store) *Hooks {
	return &Hooks{st: s}
}

func (h *Hooks) AuthRepo(repo string, pk ssh.PublicKey) wishgit.AccessLevel {
	owner, name, ok := parseRepo(repo)
	if !ok {
		return wishgit.NoAccess
	}

	user := h.userFromKey(pk)

	if user != nil {
		if user.IsAdmin || user.Username == owner {
			return wishgit.ReadWriteAccess
		}
		r, err := h.st.GetRepo(owner, name)
		if err != nil {
			return wishgit.NoAccess
		}
		if h.st.HasWriteAccess(user.ID, r.ID) {
			return wishgit.ReadWriteAccess
		}
		if r.Public {
			return wishgit.ReadOnlyAccess
		}
		return wishgit.NoAccess
	}

	if h.CanRead(nil, owner, name) {
		return wishgit.ReadOnlyAccess
	}
	return wishgit.NoAccess
}

// UserFromSession resolves the kohiro user authenticated on this SSH session,
// or nil if the presented key isn't registered.
func (h *Hooks) UserFromSession(sess ssh.Session) *store.User {
	return h.userFromKey(sess.PublicKey())
}

// CanRead returns true if user (may be nil for anonymous) may read the repo
// owned by ownerUsername.
func (h *Hooks) CanRead(user *store.User, ownerUsername, name string) bool {
	if user != nil {
		if user.IsAdmin || user.Username == ownerUsername {
			return true
		}
		r, err := h.st.GetRepo(ownerUsername, name)
		if err != nil {
			return false
		}
		if h.st.HasWriteAccess(user.ID, r.ID) || r.Public {
			return true
		}
		return false
	}
	r, err := h.st.GetRepo(ownerUsername, name)
	return err == nil && r.Public
}

// CanWriteInNamespace returns true if user may create, delete, or modify repos
// in ownerUsername's namespace. Admins pass for every namespace.
// WARNING: intentionally permissive so M7 admin panes can reuse it.
// Every TUI mutation site must ALSO check item.owner == m.user.Username to
// enforce the M4b restriction that TUI users act only in their own namespace.
func (h *Hooks) CanWriteInNamespace(user *store.User, ownerUsername string) bool {
	return user != nil && (user.IsAdmin || user.Username == ownerUsername)
}

func (h *Hooks) Push(repo string, pk ssh.PublicKey) {
	owner, name, ok := parseRepo(repo)
	if !ok {
		return
	}

	user := h.userFromKey(pk)
	if user == nil {
		return
	}

	// Resolve the namespace owner: admins can push into any existing user's namespace.
	ownerUser := user
	if user.Username != owner {
		u, err := h.st.UserByUsername(owner)
		if errors.Is(err, store.ErrNotFound) {
			log.Printf("push: namespace %q has no registered user", owner)
			return
		}
		if err != nil {
			log.Printf("push: lookup namespace %q: %v", owner, err)
			return
		}
		ownerUser = u
	}

	if _, err := h.st.EnsureRepo(ownerUser.ID, name); err != nil {
		log.Printf("push: ensureRepo %s/%s: %v", owner, name, err)
		return
	}
	log.Printf("post-receive: %s", repo)
}

func (h *Hooks) Fetch(_ string, _ ssh.PublicKey) {}

func (h *Hooks) userFromKey(pk ssh.PublicKey) *store.User {
	if pk == nil {
		return nil
	}
	fp := gossh.FingerprintSHA256(pk)
	u, err := h.st.UserByFingerprint(fp)
	if err != nil {
		return nil
	}
	return u
}

// parseRepo splits "owner/name.git" into (owner, name).
// Returns ok=false for single-component paths (no owner namespace).
func parseRepo(path string) (owner, name string, ok bool) {
	path = strings.TrimSuffix(path, ".git")
	idx := strings.IndexByte(path, '/')
	if idx < 0 {
		return "", "", false
	}
	return path[:idx], path[idx+1:], true
}
