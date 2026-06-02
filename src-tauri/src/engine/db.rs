use rusqlite::{params, Connection, Result};
use std::path::Path;

pub fn init_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // Order matters: set busy_timeout FIRST so the journal-mode switch below
    // (which briefly locks the db/-wal header) waits-and-retries under
    // contention instead of returning SQLITE_BUSY immediately. With many
    // concurrent download tasks opening connections, the reverse order races.
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // synchronous=NORMAL is durable+safe under WAL and cuts fsync churn across
    // the many short-lived per-document write connections.
    conn.pragma_update(None, "synchronous", "NORMAL")?;

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

    /// Open a connection to an already-initialized library DB without re-running
    /// the journal-mode switch or the CREATE TABLE/INDEX statements. Used by the
    /// per-document write tasks: the schema + WAL are established once at run
    /// start (via `new`), so each task only needs a busy_timeout so concurrent
    /// writers wait rather than failing with SQLITE_BUSY. Falls back implicitly
    /// to creating the file if it somehow doesn't exist (open is permissive),
    /// but the schema is expected to be present.
    pub fn open_existing(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        // synchronous=NORMAL (durable under the already-established WAL) cuts
        // fsync churn on these many short-lived per-document write connections —
        // this is the path the init_db comment's NORMAL note is really about.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
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
