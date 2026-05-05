use rusqlite::{params, Connection, Result};
use std::path::Path;

pub fn init_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // Enable WAL mode for better concurrency
    conn.execute("PRAGMA journal_mode = WAL", [])?;

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
