use rusqlite::{Connection, params};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Target {
    pub id: i64,
    pub local_path: String,
    pub remote_path: String,
    pub mode: String,
    pub last_push: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct FileState {
    pub id: i64,
    pub target_id: i64,
    pub rel_path: String,
    pub mtime: f64,
    pub size: i64,
    pub uploaded_at: String,
}

pub struct Database {
    conn: Connection,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish()
    }
}

impl Database {
    pub fn new() -> Self {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(&path).expect("Failed to open database");
        Self { conn }
    }

    pub fn db_path() -> PathBuf {
        let data_dir = glib::user_data_dir();
        data_dir.join("simplesync").join("simplesync.db")
    }

    pub fn init(&self) {
        self.conn.execute_batch("PRAGMA journal_mode=WAL;").ok();
        self.conn.execute_batch("PRAGMA foreign_keys=ON;").ok();

        self.conn.execute_batch("
            CREATE TABLE IF NOT EXISTS targets (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                local_path  TEXT NOT NULL,
                remote_path TEXT NOT NULL,
                mode        TEXT NOT NULL DEFAULT 'upload',
                last_push   TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS file_state (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                target_id   INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
                rel_path    TEXT NOT NULL,
                mtime       REAL NOT NULL,
                size        INTEGER NOT NULL,
                uploaded_at TEXT NOT NULL,
                UNIQUE(target_id, rel_path)
            );
        ").expect("Failed to create tables");
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // --- Targets ---

    pub fn get_targets(&self) -> Result<Vec<Target>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, local_path, remote_path, mode, last_push, created_at FROM targets ORDER BY created_at"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Target {
                id: row.get(0)?,
                local_path: row.get(1)?,
                remote_path: row.get(2)?,
                mode: row.get(3)?,
                last_push: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_target(&self, id: i64) -> Result<Target, rusqlite::Error> {
        self.conn.query_row(
            "SELECT id, local_path, remote_path, mode, last_push, created_at FROM targets WHERE id = ?1",
            params![id],
            |row| {
                Ok(Target {
                    id: row.get(0)?,
                    local_path: row.get(1)?,
                    remote_path: row.get(2)?,
                    mode: row.get(3)?,
                    last_push: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )
    }

    pub fn add_target(&self, local_path: &str, remote_path: &str, mode: &str) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO targets (local_path, remote_path, mode) VALUES (?1, ?2, ?3)",
            params![local_path, remote_path, mode],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_target(&self, id: i64, local_path: &str, remote_path: &str, mode: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE targets SET local_path = ?1, remote_path = ?2, mode = ?3 WHERE id = ?4",
            params![local_path, remote_path, mode, id],
        )?;
        Ok(())
    }

    pub fn delete_target(&self, id: i64) -> Result<(), rusqlite::Error> {
        self.conn.execute("DELETE FROM targets WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn update_last_push(&self, target_id: i64) -> Result<(), rusqlite::Error> {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        self.conn.execute(
            "UPDATE targets SET last_push = ?1 WHERE id = ?2",
            params![now, target_id],
        )?;
        Ok(())
    }

    // --- File State ---

    pub fn get_file_state(&self, target_id: i64, rel_path: &str) -> Result<Option<FileState>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, target_id, rel_path, mtime, size, uploaded_at FROM file_state WHERE target_id = ?1 AND rel_path = ?2"
        )?;
        let mut rows = stmt.query_map(params![target_id, rel_path], |row| {
            Ok(FileState {
                id: row.get(0)?,
                target_id: row.get(1)?,
                rel_path: row.get(2)?,
                mtime: row.get(3)?,
                size: row.get(4)?,
                uploaded_at: row.get(5)?,
            })
        })?;
        match rows.next() {
            Some(Ok(fs)) => Ok(Some(fs)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn get_all_file_states(&self, target_id: i64) -> Result<Vec<FileState>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, target_id, rel_path, mtime, size, uploaded_at FROM file_state WHERE target_id = ?1"
        )?;
        let rows = stmt.query_map(params![target_id], |row| {
            Ok(FileState {
                id: row.get(0)?,
                target_id: row.get(1)?,
                rel_path: row.get(2)?,
                mtime: row.get(3)?,
                size: row.get(4)?,
                uploaded_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn upsert_file_state(&self, target_id: i64, rel_path: &str, mtime: f64, size: i64) -> Result<(), rusqlite::Error> {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        self.conn.execute(
            "INSERT INTO file_state (target_id, rel_path, mtime, size, uploaded_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(target_id, rel_path) DO UPDATE SET mtime = ?3, size = ?4, uploaded_at = ?5",
            params![target_id, rel_path, mtime, size, now],
        )?;
        Ok(())
    }

    pub fn clear_file_states(&self, target_id: i64) -> Result<(), rusqlite::Error> {
        self.conn.execute("DELETE FROM file_state WHERE target_id = ?1", params![target_id])?;
        Ok(())
    }
}

use gtk::glib;
