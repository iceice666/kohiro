use rusqlite::{params, Connection, Error as SqliteError};
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct Repo {
    pub id: i64,
    pub owner_id: i64,
    pub name: String,
    pub public: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> StoreResult<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute_batch(
            r#"
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
            "#,
        )?;
        Ok(())
    }

    pub fn bootstrap(&self, username: &str, fingerprint: &str, comment: &str) -> StoreResult<()> {
        let user = match self.user_by_username(username)? {
            Some(user) => user,
            None => self.add_user(username, true)?,
        };
        self.add_key(user.id, fingerprint, comment)
    }

    pub fn add_user(&self, username: &str, is_admin: bool) -> StoreResult<User> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO users(username, is_admin) VALUES (?, ?)",
            params![username, is_admin as i64],
        )?;
        Ok(User {
            id: conn.last_insert_rowid(),
            username: username.to_owned(),
            is_admin,
        })
    }

    pub fn add_key(&self, user_id: i64, fingerprint: &str, comment: &str) -> StoreResult<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO ssh_keys(user_id, fingerprint, comment) VALUES (?, ?, ?)",
            params![user_id, fingerprint, comment],
        )?;
        Ok(())
    }

    pub fn user_by_fingerprint(&self, fp: &str) -> StoreResult<Option<User>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let result = conn.query_row(
            r#"
            SELECT u.id, u.username, u.is_admin
            FROM users u
            JOIN ssh_keys k ON k.user_id = u.id
            WHERE k.fingerprint = ?
            "#,
            params![fp],
            |row| {
                let is_admin: i64 = row.get(2)?;
                Ok(User {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    is_admin: is_admin != 0,
                })
            },
        );
        match result {
            Ok(user) => Ok(Some(user)),
            Err(SqliteError::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(StoreError::Sqlite(err)),
        }
    }

    pub fn user_by_username(&self, username: &str) -> StoreResult<Option<User>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let result = conn.query_row(
            "SELECT id, username, is_admin FROM users WHERE username = ?",
            params![username],
            |row| {
                let is_admin: i64 = row.get(2)?;
                Ok(User {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    is_admin: is_admin != 0,
                })
            },
        );
        match result {
            Ok(user) => Ok(Some(user)),
            Err(SqliteError::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(StoreError::Sqlite(err)),
        }
    }

    pub fn ensure_repo(&self, owner_id: i64, name: &str) -> StoreResult<Repo> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO repos(owner_id, name, public) VALUES (?, ?, 0)",
            params![owner_id, name],
        )?;
        drop(conn);
        self.repo_by_owner_and_name(owner_id, name)
    }

    pub fn get_repo(&self, owner_username: &str, name: &str) -> StoreResult<Option<Repo>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let result = conn.query_row(
            r#"
            SELECT r.id, r.owner_id, r.name, r.public
            FROM repos r
            JOIN users u ON u.id = r.owner_id
            WHERE u.username = ? AND r.name = ?
            "#,
            params![owner_username, name],
            |row| {
                let public: i64 = row.get(3)?;
                Ok(Repo {
                    id: row.get(0)?,
                    owner_id: row.get(1)?,
                    name: row.get(2)?,
                    public: public != 0,
                })
            },
        );
        match result {
            Ok(repo) => Ok(Some(repo)),
            Err(SqliteError::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(StoreError::Sqlite(err)),
        }
    }

    fn repo_by_owner_and_name(&self, owner_id: i64, name: &str) -> StoreResult<Repo> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let result = conn.query_row(
            "SELECT id, owner_id, name, public FROM repos WHERE owner_id = ? AND name = ?",
            params![owner_id, name],
            |row| {
                let public: i64 = row.get(3)?;
                Ok(Repo {
                    id: row.get(0)?,
                    owner_id: row.get(1)?,
                    name: row.get(2)?,
                    public: public != 0,
                })
            },
        );
        match result {
            Ok(repo) => Ok(repo),
            Err(SqliteError::QueryReturnedNoRows) => Err(StoreError::NotFound),
            Err(err) => Err(StoreError::Sqlite(err)),
        }
    }

    pub fn has_write_access(&self, user_id: i64, repo_id: i64) -> bool {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let result: rusqlite::Result<i64> = conn.query_row(
            "SELECT write FROM repo_perms WHERE repo_id = ? AND user_id = ?",
            params![repo_id, user_id],
            |row| row.get(0),
        );
        result.is_ok_and(|write| write != 0)
    }

    pub fn grant_write(&self, user_id: i64, repo_id: i64) -> StoreResult<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO repo_perms(repo_id, user_id, write) VALUES (?, ?, 1)",
            params![repo_id, user_id],
        )?;
        Ok(())
    }

    pub fn set_public(&self, owner_username: &str, name: &str, public: bool) -> StoreResult<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let affected = conn.execute(
            r#"
            UPDATE repos SET public = ?
            WHERE name = ? AND owner_id = (SELECT id FROM users WHERE username = ?)
            "#,
            params![public as i64, name, owner_username],
        )?;
        if affected == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}
