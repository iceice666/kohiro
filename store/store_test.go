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

func TestListReposForUser_OwnedPublicAndExplicit(t *testing.T) {
	s := openTemp(t)

	alice, _ := s.AddUser("alice", false)
	bob, _ := s.AddUser("bob", false)

	s.EnsureRepo(alice.ID, "myrepo")
	s.EnsureRepo(bob.ID, "public-repo")
	s.EnsureRepo(bob.ID, "private-repo")

	s.SetPublic("bob", "public-repo", true)

	bobPrivate, _ := s.GetRepo("bob", "private-repo")
	s.GrantWrite(alice.ID, bobPrivate.ID)

	listings, err := s.ListReposForUser(alice.ID)
	if err != nil {
		t.Fatalf("ListReposForUser: %v", err)
	}
	names := make(map[string]bool)
	for _, l := range listings {
		names[l.OwnerUsername+"/"+l.Name] = true
	}
	for _, want := range []string{"alice/myrepo", "bob/public-repo", "bob/private-repo"} {
		if !names[want] {
			t.Errorf("expected %q in listings, got %v", want, names)
		}
	}
}

func TestListReposForUser_Admin(t *testing.T) {
	s := openTemp(t)

	admin, _ := s.AddUser("admin", true)
	other, _ := s.AddUser("other", false)
	s.EnsureRepo(other.ID, "secret")

	listings, err := s.ListReposForUser(admin.ID)
	if err != nil {
		t.Fatalf("ListReposForUser: %v", err)
	}
	found := false
	for _, l := range listings {
		if l.OwnerUsername == "other" && l.Name == "secret" {
			found = true
		}
	}
	if !found {
		t.Fatal("admin should see all repos including secret private ones")
	}
}

func TestListPublicRepos_FiltersPrivate(t *testing.T) {
	s := openTemp(t)

	u, _ := s.AddUser("user", false)
	s.EnsureRepo(u.ID, "pub")
	s.EnsureRepo(u.ID, "priv")
	s.SetPublic("user", "pub", true)

	listings, err := s.ListPublicRepos()
	if err != nil {
		t.Fatalf("ListPublicRepos: %v", err)
	}
	for _, l := range listings {
		if l.Name == "priv" {
			t.Fatal("private repo should not appear in ListPublicRepos")
		}
	}
	found := false
	for _, l := range listings {
		if l.Name == "pub" {
			found = true
		}
	}
	if !found {
		t.Fatal("public repo should appear in ListPublicRepos")
	}
}

func TestListKeysForUser(t *testing.T) {
	s := openTemp(t)

	u, _ := s.AddUser("user", false)
	s.AddKey(u.ID, "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1=", "key1")
	s.AddKey(u.ID, "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA2=", "key2")

	keys, err := s.ListKeysForUser(u.ID)
	if err != nil {
		t.Fatalf("ListKeysForUser: %v", err)
	}
	if len(keys) != 2 {
		t.Fatalf("expected 2 keys, got %d", len(keys))
	}

	// Unknown user returns empty, not error.
	keys2, err := s.ListKeysForUser(99999)
	if err != nil {
		t.Fatalf("ListKeysForUser unknown: %v", err)
	}
	if len(keys2) != 0 {
		t.Fatalf("expected 0 keys for unknown user, got %d", len(keys2))
	}
}

func TestAddKeyStrict_SuccessNewKey(t *testing.T) {
	s := openTemp(t)
	u, _ := s.AddUser("alice", false)
	const fp = "SHA256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="

	owned, err := s.AddKeyStrict(u.ID, fp, "test")
	if err != nil {
		t.Fatalf("AddKeyStrict: %v", err)
	}
	if owned {
		t.Fatal("expected alreadyOwned=false for a new key")
	}
	// Row must exist.
	keys, _ := s.ListKeysForUser(u.ID)
	if len(keys) != 1 || keys[0].Fingerprint != fp {
		t.Fatalf("key not found after AddKeyStrict: %v", keys)
	}
}

func TestAddKeyStrict_AlreadyOwned(t *testing.T) {
	s := openTemp(t)
	u, _ := s.AddUser("alice", false)
	const fp = "SHA256:CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC="
	s.AddKey(u.ID, fp, "first")

	owned, err := s.AddKeyStrict(u.ID, fp, "second")
	if err != nil {
		t.Fatalf("AddKeyStrict (already owned): %v", err)
	}
	if !owned {
		t.Fatal("expected alreadyOwned=true")
	}
}

func TestAddKeyStrict_ClaimedByOther(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	bob, _ := s.AddUser("bob", false)
	const fp = "SHA256:DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD="
	s.AddKey(alice.ID, fp, "alice-key")

	_, err := s.AddKeyStrict(bob.ID, fp, "bob-key")
	if err != store.ErrKeyClaimedByOther {
		t.Fatalf("expected ErrKeyClaimedByOther, got %v", err)
	}
}

func TestKeyCount(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	bob, _ := s.AddUser("bob", false)

	n, err := s.KeyCount(alice.ID)
	if err != nil || n != 0 {
		t.Fatalf("expected 0, got %d %v", n, err)
	}
	s.AddKey(alice.ID, "SHA256:EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE=", "k1")
	s.AddKey(alice.ID, "SHA256:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF=", "k2")

	n, _ = s.KeyCount(alice.ID)
	if n != 2 {
		t.Fatalf("expected 2, got %d", n)
	}
	n, _ = s.KeyCount(bob.ID)
	if n != 0 {
		t.Fatalf("bob should have 0 keys, got %d", n)
	}
}

func TestRemoveKey(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	s.AddKey(alice.ID, "SHA256:GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG=", "k1")
	s.AddKey(alice.ID, "SHA256:HHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHHH=", "k2")

	keys, _ := s.ListKeysForUser(alice.ID)
	if len(keys) != 2 {
		t.Fatalf("setup: expected 2 keys, got %d", len(keys))
	}
	removeID := keys[0].ID

	if err := s.RemoveKey(alice.ID, removeID); err != nil {
		t.Fatalf("RemoveKey: %v", err)
	}
	keys, _ = s.ListKeysForUser(alice.ID)
	if len(keys) != 1 {
		t.Fatalf("expected 1 key after removal, got %d", len(keys))
	}
	if keys[0].ID == removeID {
		t.Fatal("removed key still present")
	}
}

func TestRemoveKey_OtherUser(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	bob, _ := s.AddUser("bob", false)
	s.AddKey(alice.ID, "SHA256:IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII=", "alice-key")
	s.AddKey(bob.ID, "SHA256:JJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJ=", "bob-key")

	bobKeys, _ := s.ListKeysForUser(bob.ID)
	bobKeyID := bobKeys[0].ID

	// Alice should not be able to remove Bob's key.
	err := s.RemoveKey(alice.ID, bobKeyID)
	if err != store.ErrNotFound {
		t.Fatalf("expected ErrNotFound, got %v", err)
	}
	// Bob's key still present.
	keys, _ := s.ListKeysForUser(bob.ID)
	if len(keys) != 1 {
		t.Fatal("Bob's key should still be present")
	}
}

func TestRemoveKey_NonexistentKey(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	err := s.RemoveKey(alice.ID, 99999)
	if err != store.ErrNotFound {
		t.Fatalf("expected ErrNotFound, got %v", err)
	}
}

func TestDeleteRepo_HappyPath(t *testing.T) {
	s := openTemp(t)
	alice, _ := s.AddUser("alice", false)
	bob, _ := s.AddUser("bob", false)

	r1, _ := s.EnsureRepo(alice.ID, "r1")
	_, _ = s.EnsureRepo(alice.ID, "r2")
	s.GrantWrite(bob.ID, r1.ID)

	if !s.HasWriteAccess(bob.ID, r1.ID) {
		t.Fatal("setup: bob should have write access")
	}

	if err := s.DeleteRepo("alice", "r1"); err != nil {
		t.Fatalf("DeleteRepo: %v", err)
	}

	// r1 should be gone.
	if _, err := s.GetRepo("alice", "r1"); err != store.ErrNotFound {
		t.Fatalf("r1 should be deleted, got %v", err)
	}
	// Bob's write perm on r1 should be gone.
	if s.HasWriteAccess(bob.ID, r1.ID) {
		t.Fatal("repo_perms for r1 should have been removed")
	}
	// r2 still exists.
	if _, err := s.GetRepo("alice", "r2"); err != nil {
		t.Fatalf("r2 should still exist: %v", err)
	}
}

func TestDeleteRepo_NotFound(t *testing.T) {
	s := openTemp(t)
	_, _ = s.AddUser("alice", false)
	err := s.DeleteRepo("alice", "missing")
	if err != store.ErrNotFound {
		t.Fatalf("expected ErrNotFound, got %v", err)
	}
}
