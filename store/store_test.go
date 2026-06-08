package store_test

import (
	"path/filepath"
	"testing"

	"github.com/iceice666/kohiro/store"
)

func openTemp(t *testing.T) *store.Store {
	t.Helper()
	dir := t.TempDir()
	s, err := store.Open(filepath.Join(dir, "test.db"))
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func TestBootstrap(t *testing.T) {
	s := openTemp(t)
	const fp = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

	if err := s.Bootstrap("admin", fp, "test"); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}
	// Idempotent second call.
	if err := s.Bootstrap("admin", fp, "test"); err != nil {
		t.Fatalf("Bootstrap (2nd): %v", err)
	}

	u, err := s.UserByFingerprint(fp)
	if err != nil {
		t.Fatalf("UserByFingerprint: %v", err)
	}
	if u.Username != "admin" || !u.IsAdmin {
		t.Fatalf("got user %+v, want admin/isAdmin=true", u)
	}
}

func TestUserByFingerprint_NotFound(t *testing.T) {
	s := openTemp(t)
	_, err := s.UserByFingerprint("SHA256:notexist")
	if err != store.ErrNotFound {
		t.Fatalf("expected ErrNotFound, got %v", err)
	}
}

func TestEnsureRepo(t *testing.T) {
	s := openTemp(t)

	u, err := s.AddUser("alice", false)
	if err != nil {
		t.Fatalf("AddUser: %v", err)
	}

	r, err := s.EnsureRepo(u.ID, "myrepo")
	if err != nil {
		t.Fatalf("EnsureRepo: %v", err)
	}
	if r.Name != "myrepo" || r.OwnerID != u.ID {
		t.Fatalf("unexpected repo: %+v", r)
	}

	// Second call must be idempotent.
	r2, err := s.EnsureRepo(u.ID, "myrepo")
	if err != nil {
		t.Fatalf("EnsureRepo (2nd): %v", err)
	}
	if r2.ID != r.ID {
		t.Fatal("EnsureRepo returned different ID on second call")
	}
}

func TestGetRepo(t *testing.T) {
	s := openTemp(t)

	u, _ := s.AddUser("bob", false)
	s.EnsureRepo(u.ID, "proj")

	r, err := s.GetRepo("bob", "proj")
	if err != nil {
		t.Fatalf("GetRepo: %v", err)
	}
	if r.Name != "proj" {
		t.Fatalf("wrong repo name: %q", r.Name)
	}

	_, err = s.GetRepo("bob", "missing")
	if err != store.ErrNotFound {
		t.Fatalf("expected ErrNotFound, got %v", err)
	}
}

func TestHasWriteAccess(t *testing.T) {
	s := openTemp(t)

	owner, _ := s.AddUser("owner", false)
	collab, _ := s.AddUser("collab", false)
	s.EnsureRepo(owner.ID, "repo")
	r, _ := s.GetRepo("owner", "repo")

	if s.HasWriteAccess(collab.ID, r.ID) {
		t.Fatal("collab should not have write access before being granted it")
	}
}

