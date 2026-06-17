use rusqlite::{params, Connection, Result};
use std::path::Path;

/// Create the schema (tables + hot-path indexes). Idempotent. Shared by the
/// live `init_db` (WAL) and the one-shot migration open (DELETE journal).
fn create_schema(conn: &Connection) -> Result<()> {
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
    Ok(())
}

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

    create_schema(&conn)?;
    Ok(conn)
}

/// One owned document row for a legacy-manifest migration.
pub struct MigrateDoc {
    pub title: String,
    pub url: String,
    pub source: String,
    pub authors: String,
    pub year: Option<String>,
    pub abstract_: Option<String>,
    pub local_path: String,
    pub text_path: Option<String>,
    pub extract_error: Option<String>,
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

    /// Open a fresh DB for a one-shot legacy migration: schema only, DELETE
    /// journal mode (no `-wal`/`-shm` sidecars to orphan when the caller renames
    /// the file atomically into place). Pair with [`Self::migrate`] + rename.
    pub fn open_for_migration(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        create_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Migrate a legacy manifest into this (fresh, temp) DB in a SINGLE
    /// transaction, so a crash mid-migration rolls back to nothing — the caller
    /// only renames a fully-populated DB into place, so a partial library can
    /// never be left where the next scan would refuse to re-migrate it.
    pub fn migrate(&mut self, query: &str, folder_path: &str, docs: &[MigrateDoc]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO runs (query, folder_path) VALUES (?1, ?2)",
            params![query, folder_path],
        )?;
        let run_id = tx.last_insert_rowid();
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO documents (
                    run_id, title, url, source, authors, year, abstract,
                    local_path, text_path, extract_error, size_bytes
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            )?;
            for d in docs {
                stmt.execute(params![
                    run_id,
                    d.title,
                    d.url,
                    d.source,
                    d.authors,
                    d.year,
                    d.abstract_,
                    d.local_path,
                    d.text_path,
                    d.extract_error,
                ])?;
            }
        }
        tx.commit()
    }

    pub fn insert_run(&self, query: &str, folder_path: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO runs (query, folder_path) VALUES (?1, ?2)",
            params![query, folder_path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[cfg(test)]
    fn doc_count(&self) -> i64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap()
    }

    #[cfg(test)]
    fn run_id_for(&self, url: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT run_id FROM documents WHERE url = ?1",
                params![url],
                |r| r.get(0),
            )
            .unwrap()
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
        // Upsert keyed on the UNIQUE url: a re-run of the same query re-points an
        // already-saved doc at the CURRENT run (so the latest-run view counts it)
        // and refreshes its fields, instead of INSERT OR REPLACE deleting and
        // re-inserting (which churned the rowid every time).
        self.conn.execute(
            "INSERT INTO documents (
                run_id, title, url, source, authors, year, abstract, local_path, text_path, extract_error, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(url) DO UPDATE SET
                run_id = excluded.run_id,
                title = excluded.title,
                source = excluded.source,
                authors = excluded.authors,
                year = excluded.year,
                abstract = excluded.abstract,
                local_path = excluded.local_path,
                text_path = excluded.text_path,
                extract_error = excluded.extract_error,
                size_bytes = excluded.size_bytes",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(mgr: &DbManager, run_id: i64, url: &str) {
        mgr.insert_document(run_id, "T", url, "s", "", None, None, "p", None, None, 0)
            .unwrap();
    }

    #[test]
    fn rerun_repoints_doc_to_current_run_without_duplicating() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = DbManager::new(&tmp.path().join("library.db")).unwrap();
        let r1 = mgr.insert_run("q", "f").unwrap();
        ins(&mgr, r1, "https://x/a");
        // Re-run the same query: a new run, same doc URL.
        let r2 = mgr.insert_run("q", "f").unwrap();
        ins(&mgr, r2, "https://x/a");
        // Still one row, now attributed to the newest run (so latest-run/all-docs
        // counting can never make a re-run shrink the library).
        assert_eq!(mgr.doc_count(), 1);
        assert_eq!(mgr.run_id_for("https://x/a"), r2);
    }

    #[test]
    fn migrate_is_atomic_and_populates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("library.db.tmp");
        let docs = vec![
            MigrateDoc {
                title: "A".into(),
                url: "https://x/a".into(),
                source: "manifest".into(),
                authors: "Doe".into(),
                year: Some("2020".into()),
                abstract_: None,
                local_path: "a.pdf".into(),
                text_path: None,
                extract_error: None,
            },
            MigrateDoc {
                title: "B".into(),
                url: "https://x/b".into(),
                source: "manifest".into(),
                authors: String::new(),
                year: None,
                abstract_: None,
                local_path: "b.pdf".into(),
                text_path: None,
                extract_error: None,
            },
        ];
        let mut mgr = DbManager::open_for_migration(&path).unwrap();
        mgr.migrate("q", "f", &docs).unwrap();
        assert_eq!(mgr.doc_count(), 2);
    }
}
