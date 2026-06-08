package git

import (
	"bytes"
	"errors"
	"io"

	gogit "github.com/go-git/go-git/v5"
	"github.com/go-git/go-git/v5/plumbing"
	"github.com/go-git/go-git/v5/plumbing/object"
)

var ErrTooLarge = errors.New("blob too large")

const blobHardCap = 1 << 20 // 1 MiB

func OpenRepo(owner, name string) (*gogit.Repository, error) {
	return gogit.PlainOpen(RepoPath(owner, name))
}

// CommitLog returns up to n commits starting from HEAD, newest first.
// Returns an empty slice (not an error) for empty/unborn repos.
func CommitLog(repo *gogit.Repository, n int) ([]*object.Commit, error) {
	ref, err := repo.Head()
	if errors.Is(err, plumbing.ErrReferenceNotFound) {
		return []*object.Commit{}, nil
	}
	if err != nil {
		return nil, err
	}
	iter, err := repo.Log(&gogit.LogOptions{From: ref.Hash()})
	if err != nil {
		return nil, err
	}
	var commits []*object.Commit
	err = iter.ForEach(func(c *object.Commit) error {
		commits = append(commits, c)
		if len(commits) >= n {
			return io.EOF
		}
		return nil
	})
	if err != nil && !errors.Is(err, io.EOF) {
		return nil, err
	}
	return commits, nil
}

// HeadTree returns the tree at HEAD. Returns (nil, nil) for empty/unborn repos.
func HeadTree(repo *gogit.Repository) (*object.Tree, error) {
	ref, err := repo.Head()
	if errors.Is(err, plumbing.ErrReferenceNotFound) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	commit, err := repo.CommitObject(ref.Hash())
	if err != nil {
		return nil, err
	}
	return commit.Tree()
}

// TreeAt returns the subtree at path within HEAD. Empty path returns the root tree.
func TreeAt(repo *gogit.Repository, path string) (*object.Tree, error) {
	root, err := HeadTree(repo)
	if err != nil || root == nil || path == "" {
		return root, err
	}
	return root.Tree(path)
}

// Blob returns file contents at path in HEAD, capped at maxBytes.
// truncated is true when the file is larger than maxBytes but within the hard cap.
// Returns ErrTooLarge when the file exceeds the 1 MiB hard cap.
func Blob(repo *gogit.Repository, path string, maxBytes int64) (data []byte, truncated bool, err error) {
	root, err := HeadTree(repo)
	if err != nil || root == nil {
		return nil, false, err
	}
	f, err := root.File(path)
	if err != nil {
		return nil, false, err
	}
	if f.Size > blobHardCap {
		return nil, false, ErrTooLarge
	}
	r, err := f.Reader()
	if err != nil {
		return nil, false, err
	}
	defer r.Close()
	if f.Size > maxBytes {
		buf := make([]byte, maxBytes)
		_, err = io.ReadFull(r, buf)
		return buf, true, err
	}
	buf, err := io.ReadAll(r)
	return buf, false, err
}

// IsBinary reports whether b looks like binary data (NUL byte in first 8 KiB).
func IsBinary(b []byte) bool {
	check := b
	if len(check) > 8192 {
		check = check[:8192]
	}
	return bytes.IndexByte(check, 0) >= 0
}
