package store

import (
	"database/sql"
	"errors"
	"os"
	"path/filepath"

	_ "modernc.org/sqlite"
)

var ErrNotFound = errors.New("not found")

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

func (s *Store) HasWriteAccess(userID, repoID int64) bool {
	var w int
	_ = s.db.QueryRow(
		`SELECT write FROM repo_perms WHERE repo_id = ? AND user_id = ?`, repoID, userID,
	).Scan(&w)
	return w != 0
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
