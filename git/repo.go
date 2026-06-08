package git

import (
	"os"
	"path/filepath"

	wishgit "github.com/charmbracelet/wish/git"
)

const RepoDir = "./data/repos"

// Init creates a bare git repository at data/repos/<owner>/<name>.git.
func Init(owner, name string) error {
	return wishgit.EnsureRepo(RepoDir, owner+"/"+name+".git")
}

// Delete removes the bare repository directory. Best-effort: a concurrent
// clone or push may observe vanished files, which is acceptable for M4b.
func Delete(owner, name string) error {
	return os.RemoveAll(filepath.Join(RepoDir, owner, name+".git"))
}
