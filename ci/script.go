package ci

import (
	"strings"

	gogit "github.com/go-git/go-git/v5"
	gogitobj "github.com/go-git/go-git/v5/plumbing/object"

	kohirogit "github.com/iceice666/kohiro/git"
)

const (
	ImagePath    = ".ci/image"
	DefaultImage = "alpine:latest"
)

// ScriptAt returns (script, true, nil) if .ci/<event> exists at sha in repo,
// or ("", false, nil) when the file is absent.
func ScriptAt(repo *gogit.Repository, sha, event string) (string, bool, error) {
	data, _, err := kohirogit.BlobAt(repo, sha, ".ci/"+event, 256*1024)
	if err != nil {
		if err == gogitobj.ErrFileNotFound {
			return "", false, nil
		}
		return "", false, err
	}
	return string(data), true, nil
}

// ResolveImage returns the trimmed contents of .ci/image at sha, or DefaultImage if absent.
func ResolveImage(repo *gogit.Repository, sha string) (string, error) {
	data, _, err := kohirogit.BlobAt(repo, sha, ImagePath, 256)
	if err != nil {
		if err == gogitobj.ErrFileNotFound {
			return DefaultImage, nil
		}
		return "", err
	}
	image := strings.TrimSpace(string(data))
	if image == "" {
		return DefaultImage, nil
	}
	return image, nil
}
