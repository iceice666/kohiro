package store

import (
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"
)

var (
	ErrNotFound          = errors.New("not found")
	ErrKeyClaimedByOther = errors.New("key already registered to another user")
	ErrLastKey           = errors.New("cannot remove the last key")
)

type Store struct {
	db *sql.DB
}

type User struct {
	ID       int64
	Username string
	IsAdmin  bool
}

type Repo struct {
	ID      int64
	OwnerID int64
	Name    string
	Public  bool
}

func Open(path string) (*Store, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", path+"?_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)")
	if err != nil {
		return nil, err
	}
	s := &Store{db: db}
	if err := s.migrate(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return s, nil
}

func (s *Store) Close() error {
	return s.db.Close()
}

func (s *Store) migrate() error {
	_, err := s.db.Exec(`
		CREATE TABLE IF NOT EXISTS users (
			id       INTEGER PRIMARY KEY AUTOINCREMENT,
			username TEXT    NOT NULL UNIQUE,
			is_admin INTEGER NOT NULL DEFAULT 0
		);
		CREATE TABLE IF NOT EXISTS ssh_keys (
			id          INTEGER PRIMARY KEY AUTOINCREMENT,
			user_id     INTEGER NOT NULL REFERENCES users(id),
			fingerprint TEXT    NOT NULL UNIQUE,
			comment     TEXT    NOT NULL DEFAULT ''
		);
		CREATE TABLE IF NOT EXISTS repos (
			id       INTEGER PRIMARY KEY AUTOINCREMENT,
			owner_id INTEGER NOT NULL REFERENCES users(id),
			name     TEXT    NOT NULL,
			public   INTEGER NOT NULL DEFAULT 0,
			UNIQUE(owner_id, name)
		);
		CREATE TABLE IF NOT EXISTS repo_perms (
			repo_id INTEGER NOT NULL REFERENCES repos(id),
			user_id INTEGER NOT NULL REFERENCES users(id),
			write   INTEGER NOT NULL DEFAULT 0,
			PRIMARY KEY(repo_id, user_id)
		);
		CREATE TABLE IF NOT EXISTS git_bug_identities (
			user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
			repo_id    INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
			git_bug_id TEXT    NOT NULL,
			PRIMARY KEY (user_id, repo_id)
		);
		CREATE TABLE IF NOT EXISTS ci_runs (
			id          INTEGER PRIMARY KEY AUTOINCREMENT,
			repo_id     INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
			sha         TEXT    NOT NULL,
			ref         TEXT    NOT NULL,
			event       TEXT    NOT NULL,
			status      TEXT    NOT NULL,
			exit_code   INTEGER,
			image       TEXT    NOT NULL DEFAULT '',
			queued_at   TEXT    NOT NULL,
			started_at  TEXT,
			finished_at TEXT
		);
		CREATE INDEX IF NOT EXISTS idx_ci_runs_repo_id ON ci_runs(repo_id, id DESC);
		CREATE INDEX IF NOT EXISTS idx_ci_runs_status  ON ci_runs(status);
	`)
	return err
}

func (s *Store) AddUser(username string, isAdmin bool) (*User, error) {
	admin := 0
	if isAdmin {
		admin = 1
	}
	res, err := s.db.Exec(`INSERT INTO users(username, is_admin) VALUES (?, ?)`, username, admin)
	if err != nil {
		return nil, err
	}
	id, _ := res.LastInsertId()
	return &User{ID: id, Username: username, IsAdmin: isAdmin}, nil
}

// AddKey associates a fingerprint with a user. Idempotent: no-ops if already present.
func (s *Store) AddKey(userID int64, fingerprint, comment string) error {
	_, err := s.db.Exec(
		`INSERT OR IGNORE INTO ssh_keys(user_id, fingerprint, comment) VALUES (?, ?, ?)`,
		userID, fingerprint, comment,
	)
	return err
}

func (s *Store) UserByFingerprint(fp string) (*User, error) {
	var u User
	var isAdmin int
	err := s.db.QueryRow(`
		SELECT u.id, u.username, u.is_admin
		FROM users u
		JOIN ssh_keys k ON k.user_id = u.id
		WHERE k.fingerprint = ?
	`, fp).Scan(&u.ID, &u.Username, &isAdmin)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	u.IsAdmin = isAdmin != 0
	return &u, nil
}

func (s *Store) UserByUsername(username string) (*User, error) {
	var u User
	var isAdmin int
	err := s.db.QueryRow(
		`SELECT id, username, is_admin FROM users WHERE username = ?`, username,
	).Scan(&u.ID, &u.Username, &isAdmin)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	u.IsAdmin = isAdmin != 0
	return &u, nil
}

// EnsureRepo creates a DB entry for the repo if it doesn't exist, then returns it.
func (s *Store) EnsureRepo(ownerID int64, name string) (*Repo, error) {
	_, err := s.db.Exec(
		`INSERT OR IGNORE INTO repos(owner_id, name, public) VALUES (?, ?, 0)`,
		ownerID, name,
	)
	if err != nil {
		return nil, err
	}
	return s.repoByOwnerAndName(ownerID, name)
}

func (s *Store) GetRepo(ownerUsername, name string) (*Repo, error) {
	var r Repo
	var isPub int
	err := s.db.QueryRow(`
		SELECT r.id, r.owner_id, r.name, r.public
		FROM repos r
		JOIN users u ON u.id = r.owner_id
		WHERE u.username = ? AND r.name = ?
	`, ownerUsername, name).Scan(&r.ID, &r.OwnerID, &r.Name, &isPub)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	r.Public = isPub != 0
	return &r, nil
}

// GetRepoByID returns the repo and its owner's username by repo ID.
func (s *Store) GetRepoByID(id int64) (RepoListing, error) {
	var l RepoListing
	var isPub int
	err := s.db.QueryRow(`
		SELECT r.id, r.owner_id, r.name, r.public, u.username
		FROM repos r JOIN users u ON u.id = r.owner_id
		WHERE r.id = ?
	`, id).Scan(&l.ID, &l.OwnerID, &l.Name, &isPub, &l.OwnerUsername)
	if errors.Is(err, sql.ErrNoRows) {
		return RepoListing{}, ErrNotFound
	}
	if err != nil {
		return RepoListing{}, err
	}
	l.Public = isPub != 0
	return l, nil
}

func (s *Store) repoByOwnerAndName(ownerID int64, name string) (*Repo, error) {
	var r Repo
	var isPub int
	err := s.db.QueryRow(
		`SELECT id, owner_id, name, public FROM repos WHERE owner_id = ? AND name = ?`,
		ownerID, name,
	).Scan(&r.ID, &r.OwnerID, &r.Name, &isPub)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	r.Public = isPub != 0
	return &r, nil
}

func (s *Store) SetPublic(ownerUsername, name string, public bool) error {
	pub := 0
	if public {
		pub = 1
	}
	res, err := s.db.Exec(`
		UPDATE repos SET public = ?
		WHERE name = ? AND owner_id = (SELECT id FROM users WHERE username = ?)
	`, pub, name, ownerUsername)
	if err != nil {
		return err
	}
	n, _ := res.RowsAffected()
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

func (s *Store) GrantWrite(userID, repoID int64) error {
	_, err := s.db.Exec(
		`INSERT OR REPLACE INTO repo_perms(repo_id, user_id, write) VALUES (?, ?, 1)`,
		repoID, userID,
	)
	return err
}

func (s *Store) HasWriteAccess(userID, repoID int64) bool {
	var w int
	_ = s.db.QueryRow(
		`SELECT write FROM repo_perms WHERE repo_id = ? AND user_id = ?`, repoID, userID,
	).Scan(&w)
	return w != 0
}

type RepoListing struct {
	Repo
	OwnerUsername string
}

func (s *Store) ListReposForUser(userID int64) ([]RepoListing, error) {
	rows, err := s.db.Query(`
		SELECT r.id, r.owner_id, r.name, r.public, u.username
		FROM repos r
		JOIN users u ON u.id = r.owner_id
		WHERE r.public = 1
		   OR r.owner_id = ?
		   OR EXISTS (SELECT 1 FROM repo_perms p WHERE p.repo_id = r.id AND p.user_id = ?)
		   OR (SELECT is_admin FROM users WHERE id = ?) = 1
		ORDER BY u.username, r.name
	`, userID, userID, userID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanRepoListings(rows)
}

func (s *Store) ListPublicRepos() ([]RepoListing, error) {
	rows, err := s.db.Query(`
		SELECT r.id, r.owner_id, r.name, r.public, u.username
		FROM repos r
		JOIN users u ON u.id = r.owner_id
		WHERE r.public = 1
		ORDER BY u.username, r.name
	`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanRepoListings(rows)
}

func scanRepoListings(rows *sql.Rows) ([]RepoListing, error) {
	var listings []RepoListing
	for rows.Next() {
		var l RepoListing
		var isPub int
		if err := rows.Scan(&l.ID, &l.OwnerID, &l.Name, &isPub, &l.OwnerUsername); err != nil {
			return nil, err
		}
		l.Public = isPub != 0
		listings = append(listings, l)
	}
	if listings == nil {
		listings = []RepoListing{}
	}
	return listings, rows.Err()
}

type SSHKey struct {
	ID          int64
	Fingerprint string
	Comment     string
}

func (s *Store) ListKeysForUser(userID int64) ([]SSHKey, error) {
	rows, err := s.db.Query(
		`SELECT id, fingerprint, comment FROM ssh_keys WHERE user_id = ? ORDER BY id`,
		userID,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var keys []SSHKey
	for rows.Next() {
		var k SSHKey
		if err := rows.Scan(&k.ID, &k.Fingerprint, &k.Comment); err != nil {
			return nil, err
		}
		keys = append(keys, k)
	}
	if keys == nil {
		keys = []SSHKey{}
	}
	return keys, rows.Err()
}

// AddKeyStrict is the TUI add-path. Pre-checks the fingerprint uniqueness:
//   - same user already has it → (true, nil), no insert
//   - another user has it      → (false, ErrKeyClaimedByOther)
//   - free                     → INSERT, (false, nil or db err)
//
// The existing AddKey (INSERT OR IGNORE) is kept for Bootstrap which intentionally no-ops.
func (s *Store) AddKeyStrict(userID int64, fingerprint, comment string) (bool, error) {
	var existingUser int64
	err := s.db.QueryRow(
		`SELECT user_id FROM ssh_keys WHERE fingerprint = ?`, fingerprint,
	).Scan(&existingUser)
	if err == nil {
		if existingUser == userID {
			return true, nil
		}
		return false, ErrKeyClaimedByOther
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return false, err
	}
	_, err = s.db.Exec(
		`INSERT INTO ssh_keys(user_id, fingerprint, comment) VALUES (?, ?, ?)`,
		userID, fingerprint, comment,
	)
	return false, err
}

// KeyCount returns the number of SSH keys registered to userID.
func (s *Store) KeyCount(userID int64) (int, error) {
	var n int
	err := s.db.QueryRow(
		`SELECT COUNT(*) FROM ssh_keys WHERE user_id = ?`, userID,
	).Scan(&n)
	return n, err
}

// RemoveKey deletes the key if and only if it belongs to userID. Returns ErrNotFound
// if no row matched (wrong id or wrong owner).
func (s *Store) RemoveKey(userID, keyID int64) error {
	res, err := s.db.Exec(
		`DELETE FROM ssh_keys WHERE id = ? AND user_id = ?`, keyID, userID,
	)
	if err != nil {
		return err
	}
	n, _ := res.RowsAffected()
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// DeleteRepo removes the repo and its permission rows in a transaction.
// Returns ErrNotFound when no repo matches ownerUsername/name.
func (s *Store) DeleteRepo(ownerUsername, name string) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()

	var repoID int64
	err = tx.QueryRow(`
		SELECT r.id FROM repos r
		JOIN users u ON u.id = r.owner_id
		WHERE u.username = ? AND r.name = ?`, ownerUsername, name,
	).Scan(&repoID)
	if errors.Is(err, sql.ErrNoRows) {
		return ErrNotFound
	}
	if err != nil {
		return err
	}
	if _, err := tx.Exec(`DELETE FROM repo_perms WHERE repo_id = ?`, repoID); err != nil {
		return err
	}
	if _, err := tx.Exec(`DELETE FROM repos WHERE id = ?`, repoID); err != nil {
		return err
	}
	return tx.Commit()
}

// GetGitBugIdentity returns the cached git-bug USER_ID for the given (userID, repoID) pair.
// Returns ErrNotFound if no identity has been created yet.
func (s *Store) GetGitBugIdentity(userID, repoID int64) (string, error) {
	var id string
	err := s.db.QueryRow(
		`SELECT git_bug_id FROM git_bug_identities WHERE user_id = ? AND repo_id = ?`,
		userID, repoID,
	).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", err
	}
	return id, nil
}

// PutGitBugIdentity stores (or overwrites) the git-bug USER_ID for the given
// (userID, repoID) pair.
func (s *Store) PutGitBugIdentity(userID, repoID int64, gitBugID string) error {
	_, err := s.db.Exec(
		`INSERT OR REPLACE INTO git_bug_identities(user_id, repo_id, git_bug_id) VALUES (?, ?, ?)`,
		userID, repoID, gitBugID,
	)
	return err
}

// CIRun represents one CI job record.
type CIRun struct {
	ID         int64
	RepoID     int64
	SHA        string
	Ref        string
	Event      string
	Status     string
	ExitCode   *int
	Image      string
	QueuedAt   time.Time
	StartedAt  *time.Time
	FinishedAt *time.Time
}

// EnqueueRun inserts a new ci_runs row with status 'queued'.
func (s *Store) EnqueueRun(repoID int64, sha, ref, event, image string) (CIRun, error) {
	now := time.Now().UTC().Format(time.RFC3339)
	res, err := s.db.Exec(
		`INSERT INTO ci_runs(repo_id, sha, ref, event, status, image, queued_at)
		 VALUES (?, ?, ?, ?, 'queued', ?, ?)`,
		repoID, sha, ref, event, image, now,
	)
	if err != nil {
		return CIRun{}, err
	}
	id, _ := res.LastInsertId()
	queuedAt, _ := time.Parse(time.RFC3339, now)
	return CIRun{
		ID: id, RepoID: repoID, SHA: sha, Ref: ref, Event: event,
		Status: "queued", Image: image, QueuedAt: queuedAt,
	}, nil
}

// ClaimNextRun atomically picks the oldest queued run and marks it 'running'.
// Returns (run, true, nil) on success, (zero, false, nil) when the queue is empty.
func (s *Store) ClaimNextRun() (CIRun, bool, error) {
	tx, err := s.db.Begin()
	if err != nil {
		return CIRun{}, false, err
	}
	defer tx.Rollback()

	var run CIRun
	var exitCode sql.NullInt64
	var startedAt, finishedAt sql.NullString
	var queuedAtStr string
	err = tx.QueryRow(`
		SELECT id, repo_id, sha, ref, event, status, exit_code, image, queued_at, started_at, finished_at
		FROM ci_runs WHERE status = 'queued' ORDER BY id LIMIT 1
	`).Scan(
		&run.ID, &run.RepoID, &run.SHA, &run.Ref, &run.Event, &run.Status,
		&exitCode, &run.Image, &queuedAtStr, &startedAt, &finishedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return CIRun{}, false, nil
	}
	if err != nil {
		return CIRun{}, false, err
	}

	now := time.Now().UTC().Format(time.RFC3339)
	if _, err := tx.Exec(
		`UPDATE ci_runs SET status = 'running', started_at = ? WHERE id = ?`, now, run.ID,
	); err != nil {
		return CIRun{}, false, err
	}
	if err := tx.Commit(); err != nil {
		return CIRun{}, false, err
	}

	run.Status = "running"
	run.QueuedAt, _ = time.Parse(time.RFC3339, queuedAtStr)
	t := time.Now().UTC()
	run.StartedAt = &t
	if exitCode.Valid {
		n := int(exitCode.Int64)
		run.ExitCode = &n
	}
	return run, true, nil
}

// MarkRunStarted updates started_at for a run already claimed (used when the
// caller needs to record start time separately from claiming).
func (s *Store) MarkRunStarted(id int64) error {
	now := time.Now().UTC().Format(time.RFC3339)
	_, err := s.db.Exec(`UPDATE ci_runs SET started_at = ? WHERE id = ?`, now, id)
	return err
}

// MarkRunFinished updates status, exit_code, and finished_at.
func (s *Store) MarkRunFinished(id int64, status string, exitCode int) error {
	now := time.Now().UTC().Format(time.RFC3339)
	_, err := s.db.Exec(
		`UPDATE ci_runs SET status = ?, exit_code = ?, finished_at = ? WHERE id = ?`,
		status, exitCode, now, id,
	)
	return err
}

// ListRunsForRepo returns the most recent limit runs for the given repo, newest first.
func (s *Store) ListRunsForRepo(repoID int64, limit int) ([]CIRun, error) {
	rows, err := s.db.Query(`
		SELECT id, repo_id, sha, ref, event, status, exit_code, image, queued_at, started_at, finished_at
		FROM ci_runs WHERE repo_id = ? ORDER BY id DESC LIMIT ?
	`, repoID, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	return scanCIRuns(rows)
}

// GetRun returns a single CIRun by ID. Returns ErrNotFound when absent.
func (s *Store) GetRun(id int64) (CIRun, error) {
	var run CIRun
	var exitCode sql.NullInt64
	var startedAt, finishedAt sql.NullString
	var queuedAtStr string
	err := s.db.QueryRow(`
		SELECT id, repo_id, sha, ref, event, status, exit_code, image, queued_at, started_at, finished_at
		FROM ci_runs WHERE id = ?
	`, id).Scan(
		&run.ID, &run.RepoID, &run.SHA, &run.Ref, &run.Event, &run.Status,
		&exitCode, &run.Image, &queuedAtStr, &startedAt, &finishedAt,
	)
	if errors.Is(err, sql.ErrNoRows) {
		return CIRun{}, ErrNotFound
	}
	if err != nil {
		return CIRun{}, err
	}
	run.QueuedAt, _ = time.Parse(time.RFC3339, queuedAtStr)
	if exitCode.Valid {
		n := int(exitCode.Int64)
		run.ExitCode = &n
	}
	if startedAt.Valid {
		t, _ := time.Parse(time.RFC3339, startedAt.String)
		run.StartedAt = &t
	}
	if finishedAt.Valid {
		t, _ := time.Parse(time.RFC3339, finishedAt.String)
		run.FinishedAt = &t
	}
	return run, nil
}

// RecoverStaleRuns marks any 'running' rows as 'error' (left over from a crash).
// Returns the number of rows updated.
func (s *Store) RecoverStaleRuns() (int64, error) {
	res, err := s.db.Exec(`UPDATE ci_runs SET status = 'error' WHERE status = 'running'`)
	if err != nil {
		return 0, err
	}
	return res.RowsAffected()
}

func scanCIRuns(rows *sql.Rows) ([]CIRun, error) {
	var runs []CIRun
	for rows.Next() {
		var run CIRun
		var exitCode sql.NullInt64
		var startedAt, finishedAt sql.NullString
		var queuedAtStr string
		if err := rows.Scan(
			&run.ID, &run.RepoID, &run.SHA, &run.Ref, &run.Event, &run.Status,
			&exitCode, &run.Image, &queuedAtStr, &startedAt, &finishedAt,
		); err != nil {
			return nil, err
		}
		run.QueuedAt, _ = time.Parse(time.RFC3339, queuedAtStr)
		if exitCode.Valid {
			n := int(exitCode.Int64)
			run.ExitCode = &n
		}
		if startedAt.Valid {
			t, _ := time.Parse(time.RFC3339, startedAt.String)
			run.StartedAt = &t
		}
		if finishedAt.Valid {
			t, _ := time.Parse(time.RFC3339, finishedAt.String)
			run.FinishedAt = &t
		}
		runs = append(runs, run)
	}
	if runs == nil {
		runs = []CIRun{}
	}
	return runs, rows.Err()
}

// Bootstrap creates the admin user and registers their key if the user doesn't exist yet.
// Idempotent: if the user already exists, it only adds the key (INSERT OR IGNORE).
func (s *Store) Bootstrap(username, fingerprint, comment string) error {
	u, err := s.UserByUsername(username)
	if errors.Is(err, ErrNotFound) {
		u, err = s.AddUser(username, true)
	}
	if err != nil {
		return err
	}
	return s.AddKey(u.ID, fingerprint, comment)
}
