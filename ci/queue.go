package ci

import (
	"context"
	"fmt"
	"log"
	"path/filepath"
	"sync"
	"time"

	kohirogit "github.com/iceice666/kohiro/git"
	"github.com/iceice666/kohiro/store"
)

// Queue is a SQLite-backed CI job queue with a single worker.
type Queue struct {
	st     *store.Store
	logDir string
	notify chan struct{}
	wg     sync.WaitGroup
}

func NewQueue(st *store.Store, logDir string) *Queue {
	return &Queue{
		st:     st,
		logDir: logDir,
		notify: make(chan struct{}, 1),
	}
}

// Enqueue inserts a new CI run and signals the worker.
func (q *Queue) Enqueue(repoID int64, sha, ref, event, image string) (store.CIRun, error) {
	run, err := q.st.EnqueueRun(repoID, sha, ref, event, image)
	if err != nil {
		return store.CIRun{}, err
	}
	select {
	case q.notify <- struct{}{}:
	default:
	}
	return run, nil
}

// Run is the blocking worker loop. Call it in a goroutine; it returns when ctx is cancelled.
func (q *Queue) Run(ctx context.Context, runner Runner) {
	n, err := q.st.RecoverStaleRuns()
	if err != nil {
		log.Printf("ci: recover stale runs: %v", err)
	} else if n > 0 {
		log.Printf("ci: recovered %d stale run(s) as 'error'", n)
	}

	for {
		// Drain all queued runs before sleeping.
		for {
			run, ok, err := q.st.ClaimNextRun()
			if err != nil {
				log.Printf("ci: claim next run: %v", err)
				break
			}
			if !ok {
				break
			}
			q.execute(ctx, runner, run)
		}

		select {
		case <-ctx.Done():
			return
		case <-q.notify:
		case <-time.After(30 * time.Second):
		}
	}
}

// Wait blocks until the in-flight run (if any) finishes.
func (q *Queue) Wait() {
	q.wg.Wait()
}

func (q *Queue) execute(ctx context.Context, runner Runner, run store.CIRun) {
	q.wg.Add(1)
	defer q.wg.Done()

	// Look up owner/name to derive repo path and read script.
	rl, err := q.st.GetRepoByID(run.RepoID)
	if err != nil {
		log.Printf("ci: run #%d: get repo: %v", run.ID, err)
		_ = q.st.MarkRunFinished(run.ID, "error", -1)
		return
	}

	repo, err := kohirogit.OpenRepo(rl.OwnerUsername, rl.Name)
	if err != nil {
		log.Printf("ci: run #%d: open repo: %v", run.ID, err)
		_ = q.st.MarkRunFinished(run.ID, "error", -1)
		return
	}

	script, ok, err := ScriptAt(repo, run.SHA, run.Event)
	if err != nil {
		log.Printf("ci: run #%d: read script: %v", run.ID, err)
		_ = q.st.MarkRunFinished(run.ID, "error", -1)
		return
	}
	if !ok {
		log.Printf("ci: run #%d: no .ci/%s script at %s, marking skipped", run.ID, run.Event, run.SHA)
		_ = q.st.MarkRunFinished(run.ID, "success", 0)
		return
	}

	logPath := fmt.Sprintf("%s/%d.log", q.logDir, run.ID)

	repoPath, err := filepath.Abs(kohirogit.RepoPath(rl.OwnerUsername, rl.Name))
	if err != nil {
		log.Printf("ci: run #%d: abs repo path: %v", run.ID, err)
		_ = q.st.MarkRunFinished(run.ID, "error", -1)
		return
	}

	exitCode, status, err := runner.Execute(ctx, RunSpec{
		ID:      run.ID,
		RepoDir: repoPath,
		SHA:     run.SHA,
		Image:   run.Image,
		Script:  script,
		LogPath: logPath,
	})
	if err != nil {
		log.Printf("ci: run #%d error: %v", run.ID, err)
	}

	if err := q.st.MarkRunFinished(run.ID, status, exitCode); err != nil {
		log.Printf("ci: mark run #%d finished: %v", run.ID, err)
	}
	log.Printf("ci: run #%d finished status=%s exit=%d", run.ID, status, exitCode)
}
