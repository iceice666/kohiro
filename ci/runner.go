package ci

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"time"
)

var ErrNoRuntime = errors.New("no container runtime found (tried podman, docker, nerdctl)")

// LogDir is the default directory for CI run log files.
const LogDir = "./data/logs"

// Runner executes a single CI job.
type Runner interface {
	Execute(ctx context.Context, spec RunSpec) (exitCode int, status string, err error)
}

// RunSpec describes a single CI job execution.
type RunSpec struct {
	ID      int64
	RepoDir string // absolute path to the bare repo
	SHA     string
	Image   string
	Script  string
	LogPath string // e.g. data/logs/<id>.log
}

// ShellRunner runs CI jobs by shelling out to a container runtime.
type ShellRunner struct {
	Binary string // "podman", "docker", or "nerdctl"
}

// DetectRuntime probes PATH for a supported container runtime in priority order
// and returns the name of the first one found.
func DetectRuntime() (string, error) {
	for _, candidate := range []string{"podman", "docker", "nerdctl"} {
		if _, err := exec.LookPath(candidate); err == nil {
			return candidate, nil
		}
	}
	return "", ErrNoRuntime
}

func NewShellRunner(binary string) *ShellRunner {
	return &ShellRunner{Binary: binary}
}

// Execute extracts a working tree from the bare repo at spec.SHA into a temp
// dir, then runs the CI script inside a container mounted at /work.
// Returns (exitCode, status, nil) on a clean run; (0, "error", err) on
// infrastructure failure (working tree extraction, image pull, etc.).
func (r *ShellRunner) Execute(ctx context.Context, spec RunSpec) (int, string, error) {
	f, err := os.OpenFile(spec.LogPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return 0, "error", fmt.Errorf("open log file: %w", err)
	}
	defer f.Close()

	fmt.Fprintf(f, "=== kohiro CI run #%d sha=%s image=%s ===\n", spec.ID, spec.SHA, spec.Image)

	runCtx, cancel := context.WithTimeout(ctx, 30*time.Minute)
	defer cancel()

	// Extract working tree from the bare repo into a temp dir.
	workDir, err := extractWorkTree(runCtx, spec.RepoDir, spec.SHA, f)
	if err != nil {
		fmt.Fprintf(f, "\n=== working-tree extraction failed: %v ===\n", err)
		return 0, "error", err
	}
	defer os.RemoveAll(workDir)

	cmd := exec.CommandContext(runCtx, r.Binary,
		"run", "--rm",
		"-v", workDir+":/work:ro",
		"-w", "/work",
		spec.Image,
		"sh", "-c", spec.Script,
	)
	cmd.Stdout = f
	cmd.Stderr = f

	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			code := exitErr.ExitCode()
			return code, "failed", nil
		}
		fmt.Fprintf(f, "\n=== runner error: %v ===\n", err)
		return 0, "error", err
	}
	return 0, "success", nil
}

// extractWorkTree uses `git archive <sha> | tar -x -C <tmpDir>` to produce a
// read-only working tree snapshot. Returns the path to the temp dir.
func extractWorkTree(ctx context.Context, bareRepoDir, sha string, logW io.Writer) (string, error) {
	tmpDir, err := os.MkdirTemp("", "kohiro-ci-*")
	if err != nil {
		return "", fmt.Errorf("mkdirtemp: %w", err)
	}

	pr, pw := io.Pipe()

	archive := exec.CommandContext(ctx, "git", "-C", bareRepoDir, "archive", sha)
	archive.Stdout = pw
	archive.Stderr = logW

	untar := exec.CommandContext(ctx, "tar", "-x", "-C", tmpDir)
	untar.Stdin = pr
	untar.Stderr = logW

	if err := archive.Start(); err != nil {
		_ = os.RemoveAll(tmpDir)
		return "", fmt.Errorf("git archive start: %w", err)
	}
	if err := untar.Start(); err != nil {
		_ = archive.Process.Kill()
		_ = os.RemoveAll(tmpDir)
		return "", fmt.Errorf("tar start: %w", err)
	}

	archiveErr := archive.Wait()
	pw.CloseWithError(archiveErr)
	untarErr := untar.Wait()

	if archiveErr != nil {
		_ = os.RemoveAll(tmpDir)
		return "", fmt.Errorf("git archive: %w", archiveErr)
	}
	if untarErr != nil {
		_ = os.RemoveAll(tmpDir)
		return "", fmt.Errorf("tar: %w", untarErr)
	}
	return tmpDir, nil
}
