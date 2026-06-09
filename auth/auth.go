package auth

import (
	"errors"
	"log"
	"strings"

	"github.com/charmbracelet/ssh"
	wishgit "github.com/charmbracelet/wish/git"
	gogit "github.com/go-git/go-git/v5"
	gogitobj "github.com/go-git/go-git/v5/plumbing/object"
	gossh "golang.org/x/crypto/ssh"

	kohirogit "github.com/iceice666/kohiro/git"
	"github.com/iceice666/kohiro/store"
)

// Enqueuer is the interface auth uses to trigger CI runs without importing the ci package.
type Enqueuer interface {
	Enqueue(repoID int64, sha, ref, event, image string) (store.CIRun, error)
}

// Hooks implements wishgit.Hooks using the SQLite store.
type Hooks struct {
	st *store.Store
	eq Enqueuer // may be nil (CI disabled)
}

// New creates a Hooks instance. Pass nil for eq to disable CI enqueueing.
func New(s *store.Store, eq Enqueuer) *Hooks {
	return &Hooks{st: s, eq: eq}
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

// CanWrite returns true if user may push to / create issues on the named repo:
// admin, namespace owner, or an explicit repo_perms.write grant.
func (h *Hooks) CanWrite(user *store.User, ownerUsername, name string) bool {
	if user == nil {
		return false
	}
	if user.IsAdmin || user.Username == ownerUsername {
		return true
	}
	r, err := h.st.GetRepo(ownerUsername, name)
	if err != nil {
		return false
	}
	return h.st.HasWriteAccess(user.ID, r.ID)
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

	repoRec, err := h.st.EnsureRepo(ownerUser.ID, name)
	if err != nil {
		log.Printf("push: ensureRepo %s/%s: %v", owner, name, err)
		return
	}

	if h.eq == nil {
		return
	}

	// Resolve HEAD to get the pushed SHA and ref name.
	gitRepo, err := kohirogit.OpenRepo(owner, name)
	if err != nil {
		log.Printf("push: open repo %s/%s: %v", owner, name, err)
		return
	}
	head, err := gitRepo.Head()
	if err != nil {
		log.Printf("push: read HEAD %s/%s: %v", owner, name, err)
		return
	}
	sha := head.Hash().String()
	ref := head.Name().String()

	// Only enqueue if .ci/push exists at the pushed commit.
	_, hasScript, err := ciScriptAt(gitRepo, sha, "push")
	if err != nil {
		log.Printf("push: check .ci/push %s/%s@%s: %v", owner, name, sha, err)
		return
	}
	if !hasScript {
		return
	}

	image, err := ciResolveImage(gitRepo, sha)
	if err != nil {
		log.Printf("push: resolve image %s/%s@%s: %v", owner, name, sha, err)
		return
	}

	if _, err := h.eq.Enqueue(repoRec.ID, sha, ref, "push", image); err != nil {
		log.Printf("push: enqueue CI run %s/%s@%s: %v", owner, name, sha, err)
	}
}

func (h *Hooks) Fetch(_ string, _ ssh.PublicKey) {}

// ciScriptAt returns (content, true, nil) if .ci/<event> exists at sha, ("", false, nil) if absent.
func ciScriptAt(repo *gogit.Repository, sha, event string) (string, bool, error) {
	data, _, err := kohirogit.BlobAt(repo, sha, ".ci/"+event, 256*1024)
	if err != nil {
		if err == gogitobj.ErrFileNotFound {
			return "", false, nil
		}
		return "", false, err
	}
	return string(data), true, nil
}

// ciResolveImage returns the image name from .ci/image at sha, or "alpine:latest" if absent.
func ciResolveImage(repo *gogit.Repository, sha string) (string, error) {
	data, _, err := kohirogit.BlobAt(repo, sha, ".ci/image", 256)
	if err != nil {
		if err == gogitobj.ErrFileNotFound {
			return "alpine:latest", nil
		}
		return "", err
	}
	image := strings.TrimSpace(string(data))
	if image == "" {
		return "alpine:latest", nil
	}
	return image, nil
}

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
