use once_cell::sync::Lazy;
use parking_lot::Mutex;
use rusqlite::{params, Connection, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Per-database-path init mutex. Without this, two concurrent downloads in
/// the same run could both call `init_db` for the same SQLite file, race on
/// `PRAGMA journal_mode = WAL`, and one would surface SQLITE_BUSY before the
/// 5s busy_timeout kicked in. The mutex makes the first-touch race
/// deterministic; subsequent opens after init are fully concurrent under WAL.
static INIT_LOCKS: Lazy<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    Lazy::new(Default::default);

fn init_lock_for(path: &Path) -> Arc<Mutex<()>> {
    let mut g = INIT_LOCKS.lock();
    g.entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub fn init_db(path: &Path) -> Result<Connection> {
    // Serialize the create-tables-and-set-pragmas dance per file.
    let lock = init_lock_for(path);
    let _guard = lock.lock();

    let conn = Connection::open(path)?;

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            query TEXT NOT NULL,
            folder_path TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id INTEGER,
            title TEXT NOT NULL,
            url TEXT NOT NULL UNIQUE,
            source TEXT NOT NULL,
            authors TEXT,
            year TEXT,
            abstract TEXT,
            local_path TEXT,
            text_path TEXT,
            extract_error TEXT,
            size_bytes INTEGER,
            FOREIGN KEY(run_id) REFERENCES runs(id)
        )",
        [],
    )?;

    // Indexes for the queries that actually run hot:
    //   - `SELECT COUNT(*) FROM documents WHERE run_id = ?` (library list)
    //     would full-scan the documents table without an index.
    //   - `ORDER BY r.created_at DESC LIMIT 1` (latest-run lookup) gets
    //     an index too so it scales when the runs table grows past a few
    //     hundred entries.
    // The UNIQUE constraint on documents.url already creates an implicit
    // index, so URL lookups during dedup are already fast.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_documents_run_id ON documents(run_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_runs_created_at ON runs(created_at DESC)",
        [],
    )?;

    Ok(conn)
}

pub struct DbManager {
    conn: Connection,
}

impl DbManager {
    pub fn new(path: &Path) -> Result<Self> {
        let conn = init_db(path)?;
        Ok(Self { conn })
    }

    pub fn insert_run(&self, query: &str, folder_path: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO runs (query, folder_path) VALUES (?1, ?2)",
            params![query, folder_path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_document(
        &self,
        run_id: i64,
        title: &str,
        url: &str,
        source: &str,
        authors: &str,
        year: Option<&str>,
        abstract_: Option<&str>,
        local_path: &str,
        text_path: Option<&str>,
        extract_error: Option<&str>,
        size_bytes: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO documents (
                run_id, title, url, source, authors, year, abstract, local_path, text_path, extract_error, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                run_id,
                title,
                url,
                source,
                authors,
                year,
                abstract_,
                local_path,
                text_path,
                extract_error,
                size_bytes as i64,
            ],
        )?;
        Ok(())
    }
}
