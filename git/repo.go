package git

import (
	wishgit "github.com/charmbracelet/wish/git"
)

const RepoDir = "./data/repos"

// Init creates a bare git repository at data/repos/<owner>/<name>.git.
func Init(owner, name string) error {
	return wishgit.EnsureRepo(RepoDir, owner+"/"+name+".git")
}
