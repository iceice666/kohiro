package issues

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"sync"
	"time"
)

// Client shells out to the git-bug binary.
// All methods that write (New, Comment, Close) hold a per-repo mutex for the
// duration of the adopt + action pair to serialize concurrent SSH sessions.
type Client struct {
	Binary string // defaults to "git-bug"
	locks  sync.Map
}

func (c *Client) binary() string {
	if c.Binary != "" {
		return c.Binary
	}
	return "git-bug"
}

func (c *Client) lockFor(repoPath string) *sync.Mutex {
	v, _ := c.locks.LoadOrStore(repoPath, &sync.Mutex{})
	return v.(*sync.Mutex)
}

// run executes git-bug with the given args, Dir set to repoPath.
func (c *Client) run(ctx context.Context, repoPath string, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, c.binary(), args...)
	cmd.Dir = repoPath
	out, err := cmd.Output()
	if err != nil {
		var ee *exec.ExitError
		if ok := isExitError(err, &ee); ok {
			return nil, fmt.Errorf("git-bug %s: %w (stderr: %s)", args[0], err, strings.TrimSpace(string(ee.Stderr)))
		}
		return nil, fmt.Errorf("git-bug %s: %w", args[0], err)
	}
	return out, nil
}

func isExitError(err error, out **exec.ExitError) bool {
	ee, ok := err.(*exec.ExitError)
	if ok {
		*out = ee
	}
	return ok
}

// — JSON types —

type bugTime struct {
	Time string `json:"time"` // RFC3339
}

func (t bugTime) parse() time.Time {
	v, _ := time.Parse(time.RFC3339, t.Time)
	return v
}

type identity struct {
	Name string `json:"name"`
}

// listBug is the JSON shape returned by `git-bug bug --format=json` list items.
type listBug struct {
	ID         string   `json:"id"`
	HumanID    string   `json:"human_id"`
	Status     string   `json:"status"`
	Title      string   `json:"title"`
	Author     identity `json:"author"`
	CreateTime bugTime  `json:"create_time"`
	Comments   int      `json:"comments"`
}

// showBug is the JSON shape returned by `git-bug bug show --format=json`.
type showBug struct {
	ID         string        `json:"id"`
	HumanID    string        `json:"human_id"`
	Status     string        `json:"status"`
	Title      string        `json:"title"`
	Author     identity      `json:"author"`
	CreateTime bugTime       `json:"create_time"`
	Comments   []showComment `json:"comments"`
}

type showComment struct {
	Author  identity `json:"author"`
	Message string   `json:"message"`
}

// — Public types —

type Bug struct {
	ID       string
	HumanID  string
	Title    string
	Status   string
	Author   string
	Created  time.Time
	Comments int
}

type Comment struct {
	Author string
	Body   string
}

type BugDetail struct {
	Bug
	Comments []Comment
}

// List returns all bugs in the repo.
func (c *Client) List(ctx context.Context, repoPath string) ([]Bug, error) {
	out, err := c.run(ctx, repoPath, "bug", "--format=json")
	if err != nil {
		return nil, err
	}
	if strings.TrimSpace(string(out)) == "null" || strings.TrimSpace(string(out)) == "" {
		return []Bug{}, nil
	}
	var raw []listBug
	if err := json.Unmarshal(out, &raw); err != nil {
		return nil, fmt.Errorf("git-bug list parse: %w", err)
	}
	bugs := make([]Bug, len(raw))
	for i, b := range raw {
		bugs[i] = Bug{
			ID:       b.ID,
			HumanID:  b.HumanID,
			Title:    b.Title,
			Status:   b.Status,
			Author:   b.Author.Name,
			Created:  b.CreateTime.parse(),
			Comments: b.Comments,
		}
	}
	return bugs, nil
}

// Show returns the full detail of a bug including all comments.
func (c *Client) Show(ctx context.Context, repoPath, id string) (BugDetail, error) {
	out, err := c.run(ctx, repoPath, "bug", "show", "--format=json", id)
	if err != nil {
		return BugDetail{}, err
	}
	var raw showBug
	if err := json.Unmarshal(out, &raw); err != nil {
		return BugDetail{}, fmt.Errorf("git-bug show parse: %w", err)
	}
	comments := make([]Comment, len(raw.Comments))
	for i, rc := range raw.Comments {
		comments[i] = Comment{Author: rc.Author.Name, Body: rc.Message}
	}
	return BugDetail{
		Bug: Bug{
			ID:       raw.ID,
			HumanID:  raw.HumanID,
			Title:    raw.Title,
			Status:   raw.Status,
			Author:   raw.Author.Name,
			Created:  raw.CreateTime.parse(),
			Comments: len(raw.Comments),
		},
		Comments: comments,
	}, nil
}

// EnsureIdentity creates a new git-bug identity with the given name/email and
// returns its 64-char ID. The identity is implicitly adopted for this repo
// session. Callers should cache the returned ID and use Adopt on subsequent
// writes.
func (c *Client) EnsureIdentity(ctx context.Context, repoPath, name, email string) (string, error) {
	mu := c.lockFor(repoPath)
	mu.Lock()
	defer mu.Unlock()

	out, err := c.run(ctx, repoPath, "user", "new",
		"--name="+name, "--email="+email, "--non-interactive")
	if err != nil {
		return "", fmt.Errorf("git-bug user new: %w", err)
	}
	id := strings.TrimSpace(string(out))
	if len(id) == 0 {
		return "", fmt.Errorf("git-bug user new: empty output")
	}
	return id, nil
}

// adopt switches the repo's active git-bug identity. Must be called while the
// per-repo lock is held.
func (c *Client) adopt(ctx context.Context, repoPath, gitBugUserID string) error {
	_, err := c.run(ctx, repoPath, "user", "adopt", gitBugUserID)
	return err
}

// New creates a new issue and returns its full 64-char ID.
func (c *Client) New(ctx context.Context, repoPath, gitBugUserID, title, body string) (string, error) {
	mu := c.lockFor(repoPath)
	mu.Lock()
	defer mu.Unlock()

	if err := c.adopt(ctx, repoPath, gitBugUserID); err != nil {
		return "", err
	}
	out, err := c.run(ctx, repoPath, "bug", "new",
		"--title="+title, "--message="+body, "--non-interactive")
	if err != nil {
		return "", err
	}
	// stdout is "<human_id> created\n"
	line := strings.TrimSpace(string(out))
	return line, nil
}

// Comment adds a comment to an existing bug.
func (c *Client) Comment(ctx context.Context, repoPath, gitBugUserID, id, body string) error {
	mu := c.lockFor(repoPath)
	mu.Lock()
	defer mu.Unlock()

	if err := c.adopt(ctx, repoPath, gitBugUserID); err != nil {
		return err
	}
	_, err := c.run(ctx, repoPath, "bug", "comment", "new", id,
		"--message="+body, "--non-interactive")
	return err
}

// Close marks a bug as closed.
func (c *Client) Close(ctx context.Context, repoPath, gitBugUserID, id string) error {
	mu := c.lockFor(repoPath)
	mu.Lock()
	defer mu.Unlock()

	if err := c.adopt(ctx, repoPath, gitBugUserID); err != nil {
		return err
	}
	_, err := c.run(ctx, repoPath, "bug", "status", "close", id)
	return err
}
