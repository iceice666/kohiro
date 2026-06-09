package main

import (
	"fmt"
	"io"
	"log"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"

	"github.com/iceice666/kohiro/auth"
	"github.com/iceice666/kohiro/ci"
	"github.com/iceice666/kohiro/store"
)

// logsMiddleware handles: ssh host logs <owner>/<repo> [run-id]
// It tails the log file while the run is active, then exits.
func logsMiddleware(st *store.Store, hooks *auth.Hooks) wish.Middleware {
	return func(next ssh.Handler) ssh.Handler {
		return func(sess ssh.Session) {
			cmd := sess.Command()
			if len(cmd) < 2 || cmd[0] != "logs" {
				next(sess)
				return
			}

			owner, name, ok := splitOwnerName(cmd[1])
			if !ok {
				fmt.Fprintf(sess.Stderr(), "usage: logs <owner>/<repo> [run-id]\n")
				_ = sess.Exit(1)
				return
			}

			user := hooks.UserFromSession(sess)
			if !hooks.CanRead(user, owner, name) {
				fmt.Fprintf(sess.Stderr(), "access denied\n")
				_ = sess.Exit(1)
				return
			}

			repo, err := st.GetRepo(owner, name)
			if err != nil {
				fmt.Fprintf(sess.Stderr(), "repo not found: %s/%s\n", owner, name)
				_ = sess.Exit(1)
				return
			}

			var runID int64
			if len(cmd) >= 3 {
				id, err := strconv.ParseInt(cmd[2], 10, 64)
				if err != nil {
					fmt.Fprintf(sess.Stderr(), "invalid run-id: %q\n", cmd[2])
					_ = sess.Exit(1)
					return
				}
				runID = id
			} else {
				runs, err := st.ListRunsForRepo(repo.ID, 1)
				if err != nil || len(runs) == 0 {
					fmt.Fprintf(sess.Stderr(), "no CI runs found for %s/%s\n", owner, name)
					_ = sess.Exit(1)
					return
				}
				runID = runs[0].ID
			}

			// Verify the run belongs to this repo.
			run, err := st.GetRun(runID)
			if err != nil || run.RepoID != repo.ID {
				fmt.Fprintf(sess.Stderr(), "run #%d not found for %s/%s\n", runID, owner, name)
				_ = sess.Exit(1)
				return
			}

			lp := fmt.Sprintf("%s/%d.log", ci.LogDir, runID)
			if err := tailLog(sess, st, lp, runID); err != nil {
				log.Printf("logs: tail run #%d: %v", runID, err)
				_ = sess.Exit(1)
			}
		}
	}
}

func isTerminalStatus(status string) bool {
	switch status {
	case "success", "failed", "error":
		return true
	}
	return false
}

// tailLog streams logPath to sess, polling every 300 ms while the run is active.
func tailLog(sess ssh.Session, st *store.Store, logPath string, runID int64) error {
	// If already terminal, dump and exit.
	run, err := st.GetRun(runID)
	if err != nil {
		return err
	}
	if isTerminalStatus(run.Status) {
		_, err := appendLogFrom(sess, logPath, 0)
		return err
	}

	var offset int64
	ticker := time.NewTicker(300 * time.Millisecond)
	defer ticker.Stop()

	for {
		select {
		case <-sess.Context().Done():
			return nil
		case <-ticker.C:
			n, err := appendLogFrom(sess, logPath, offset)
			if err != nil {
				return err
			}
			offset += n

			run, err := st.GetRun(runID)
			if err != nil {
				return err
			}
			if isTerminalStatus(run.Status) {
				// Flush remaining bytes.
				if _, err := appendLogFrom(sess, logPath, offset); err != nil {
					return err
				}
				return nil
			}
		}
	}
}

// appendLogFrom reads logPath from offset and writes to w. Returns bytes read.
func appendLogFrom(w io.Writer, logPath string, offset int64) (int64, error) {
	f, err := os.Open(logPath)
	if os.IsNotExist(err) {
		return 0, nil
	}
	if err != nil {
		return 0, err
	}
	defer f.Close()

	if _, err := f.Seek(offset, io.SeekStart); err != nil {
		return 0, err
	}
	n, err := io.Copy(w, f)
	if err != nil && !strings.Contains(err.Error(), "broken pipe") && !strings.Contains(err.Error(), "use of closed") {
		return n, err
	}
	return n, nil
}
