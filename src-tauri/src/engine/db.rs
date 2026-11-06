use rusqlite::{params, Connection, OpenFlags, Result};
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
    migrate_schema(conn)?;
    Ok(())
}

/// Current schema version (stored in `PRAGMA user_version`).
pub const SCHEMA_VERSION: i64 = 2;

/// Columns added in v2 (0.3): richer metadata for AI export + search.
const V2_DOC_COLUMNS: &[(&str, &str)] = &[
    ("doi", "TEXT"),
    ("venue", "TEXT"),
    ("license", "TEXT"),
    ("cited_by", "INTEGER"),
    ("score", "REAL"),
    ("sources", "TEXT"),
    ("sha256", "TEXT"),
    ("pages", "INTEGER"),
    ("lang", "TEXT"),
    ("words", "INTEGER"),
    ("tokens", "INTEGER"),
    ("md_path", "TEXT"),
    ("retrieved_at", "TEXT"),
];

/// Forward-only, idempotent migrations keyed on `PRAGMA user_version`.
/// Libraries created by 0.2 (version 0) gain the v2 columns and the chunk
/// + full-text tables in place; existing rows keep working (NULL metadata).
fn migrate_schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(documents)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for (col, ty) in V2_DOC_COLUMNS {
        if !existing.iter().any(|c| c == col) {
            conn.execute(&format!("ALTER TABLE documents ADD COLUMN {col} {ty}"), [])?;
        }
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY,
            doc_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            ord INTEGER NOT NULL,
            heading TEXT,
            page_start INTEGER,
            page_end INTEGER,
            tokens INTEGER,
            text TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id, ord);
        CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
            text, heading, content='chunks', content_rowid='id',
            tokenize='porter unicode61 remove_diacritics 2'
        );
        CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON chunks BEGIN
            INSERT INTO chunks_fts(rowid, text, heading) VALUES (new.id, new.text, new.heading);
        END;
        CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON chunks BEGIN
            INSERT INTO chunks_fts(chunks_fts, rowid, text, heading)
                VALUES ('delete', old.id, old.text, old.heading);
        END;
        CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON chunks BEGIN
            INSERT INTO chunks_fts(chunks_fts, rowid, text, heading)
                VALUES ('delete', old.id, old.text, old.heading);
            INSERT INTO chunks_fts(rowid, text, heading) VALUES (new.id, new.text, new.heading);
        END;",
    )?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Everything persisted about one saved document.
#[derive(Debug, Clone, Default)]
pub struct DocRecord {
    pub title: String,
    pub url: String,
    pub source: String,
    pub sources: Vec<String>,
    pub authors: Vec<String>,
    pub year: Option<String>,
    pub abstract_: Option<String>,
    pub doi: Option<String>,
    pub venue: Option<String>,
    pub license: Option<String>,
    pub cited_by: Option<u32>,
    pub score: Option<f32>,
    pub local_path: String,
    pub text_path: Option<String>,
    pub md_path: Option<String>,
    pub extract_error: Option<String>,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub pages: Option<u32>,
    pub lang: Option<String>,
    pub words: Option<u64>,
    pub tokens: Option<u64>,
}

/// One retrievable passage of a document's text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChunkRecord {
    pub ord: usize,
    pub heading: Option<String>,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub tokens: usize,
    pub text: String,
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
    // Enforce the documents.run_id → runs(id) foreign key (off by default in
    // SQLite). All inserts reference a run created first, so this only guards
    // against a future delete-by-run leaving orphans.
    conn.pragma_update(None, "foreign_keys", "ON")?;

    create_schema(&conn)?;
    Ok(conn)
}

/// Open a library DB **read-only** for scanning (list/open). Avoids the DDL +
/// `journal_mode=WAL` writes that `init_db` performs on every scanned folder —
/// those needlessly contend for write locks on a `library.db` an active run is
/// writing. Sets only a busy_timeout; the schema is expected to already exist.
pub fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
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
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    /// Open a fresh DB for a one-shot legacy migration: schema only, DELETE
    /// journal mode (no `-wal`/`-shm` sidecars to orphan when the caller renames
    /// the file atomically into place). Pair with [`Self::migrate`] + rename.
    pub fn open_for_migration(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
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

    /// Upsert a document (keyed on its UNIQUE url) and replace its chunks, in
    /// one transaction. A re-run re-points an existing row at the current run
    /// instead of duplicating it. Returns the document's row id.
    pub fn upsert_document(
        &mut self,
        run_id: i64,
        d: &DocRecord,
        chunks: &[ChunkRecord],
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let id = upsert_in(&tx, run_id, d, chunks)?;
        tx.commit()?;
        Ok(id)
    }

    /// Batched variant used by the pipeline's single writer: many documents,
    /// one transaction (one WAL commit instead of one per document).
    pub fn upsert_batch(
        &mut self,
        run_id: i64,
        items: &[(DocRecord, Vec<ChunkRecord>)],
    ) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let mut ids = Vec::with_capacity(items.len());
        for (d, c) in items {
            ids.push(upsert_in(&tx, run_id, d, c)?);
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Simple insert used by tests and legacy callers.
    #[cfg(test)]
    fn insert_document(&mut self, run_id: i64, url: &str) -> Result<i64> {
        self.upsert_document(
            run_id,
            &DocRecord {
                title: "T".into(),
                url: url.into(),
                source: "s".into(),
                local_path: "p".into(),
                ..Default::default()
            },
            &[],
        )
    }
}

fn upsert_in(
    tx: &rusqlite::Transaction<'_>,
    run_id: i64,
    d: &DocRecord,
    chunks: &[ChunkRecord],
) -> Result<i64> {
    tx.execute(
        "INSERT INTO documents (
            run_id, title, url, source, authors, year, abstract, local_path, text_path,
            extract_error, size_bytes, doi, venue, license, cited_by, score, sources, sha256,
            pages, lang, words, tokens, md_path, retrieved_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                  ?18, ?19, ?20, ?21, ?22, ?23, datetime('now'))
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
            size_bytes = excluded.size_bytes,
            doi = excluded.doi,
            venue = excluded.venue,
            license = excluded.license,
            cited_by = excluded.cited_by,
            score = excluded.score,
            sources = excluded.sources,
            sha256 = excluded.sha256,
            pages = excluded.pages,
            lang = excluded.lang,
            words = excluded.words,
            tokens = excluded.tokens,
            md_path = excluded.md_path,
            retrieved_at = excluded.retrieved_at",
        params![
            run_id,
            d.title,
            d.url,
            d.source,
            d.authors.join(", "),
            d.year,
            d.abstract_,
            d.local_path,
            d.text_path,
            d.extract_error,
            d.size_bytes as i64,
            d.doi,
            d.venue,
            d.license,
            d.cited_by,
            d.score,
            d.sources.join(","),
            d.sha256,
            d.pages,
            d.lang,
            d.words.map(|w| w as i64),
            d.tokens.map(|t| t as i64),
            d.md_path,
        ],
    )?;
    let id: i64 = tx.query_row(
        "SELECT id FROM documents WHERE url = ?1",
        params![d.url],
        |r| r.get(0),
    )?;
    tx.execute("DELETE FROM chunks WHERE doc_id = ?1", params![id])?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO chunks (doc_id, ord, heading, page_start, page_end, tokens, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for c in chunks {
            stmt.execute(params![
                id,
                c.ord as i64,
                c.heading,
                c.page_start,
                c.page_end,
                c.tokens as i64,
                c.text
            ])?;
        }
    }
    Ok(id)
}

/// Make a user's free-text search safe for FTS5 `MATCH`: every term is
/// quoted (so punctuation can't form operators) and terms are OR-ed, with
/// prefix matching on the last term for type-ahead.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .take(12)
        .map(|t| format!("\"{}\"", t.to_lowercase()))
        .collect();
    if terms.is_empty() {
        return None;
    }
    let n = terms.len();
    let mut out: Vec<String> = terms;
    out[n - 1].push('*');
    Some(out.join(" OR "))
}

/// Library-wide content index at `<library root>/.document-finder-index.db`.
/// Maps every downloaded URL and file hash to where the file lives, so a later
/// search can copy an already-downloaded document instead of fetching it
/// again (cross-library reuse), and so identical files reached through
/// different URLs are detected.
pub struct LibraryIndex {
    conn: Connection,
}

pub const INDEX_FILE: &str = ".document-finder-index.db";

impl LibraryIndex {
    pub fn open(root: &Path) -> Result<Self> {
        let conn = Connection::open(root.join(INDEX_FILE))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                url TEXT PRIMARY KEY,
                sha256 TEXT NOT NULL,
                path TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_files_sha ON files(sha256);",
        )?;
        Ok(Self { conn })
    }

    /// Absolute paths previously stored for any of `urls`, best first.
    pub fn lookup_urls(&self, urls: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(mut stmt) = self
            .conn
            .prepare_cached("SELECT path FROM files WHERE url = ?1")
        else {
            return out;
        };
        for u in urls {
            if let Ok(p) = stmt.query_row(params![u], |r| r.get::<_, String>(0)) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    pub fn record(&self, urls: &[String], sha256: &str, path: &str) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO files (url, sha256, path) VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET sha256 = excluded.sha256, path = excluded.path",
        )?;
        for u in urls {
            stmt.execute(params![u, sha256, path])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins(mgr: &mut DbManager, run_id: i64, url: &str) {
        mgr.insert_document(run_id, url).unwrap();
    }

    #[test]
    fn rerun_repoints_doc_to_current_run_without_duplicating() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = DbManager::new(&tmp.path().join("library.db")).unwrap();
        let r1 = mgr.insert_run("q", "f").unwrap();
        ins(&mut mgr, r1, "https://x/a");
        // Re-run the same query: a new run, same doc URL.
        let r2 = mgr.insert_run("q", "f").unwrap();
        ins(&mut mgr, r2, "https://x/a");
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

    #[test]
    fn chunks_are_full_text_searchable_and_replaced_on_upsert() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = DbManager::new(&tmp.path().join("library.db")).unwrap();
        let run = mgr.insert_run("q", "f").unwrap();
        let rec = DocRecord {
            title: "Paper".into(),
            url: "https://x/p".into(),
            source: "arxiv".into(),
            local_path: "p.pdf".into(),
            ..Default::default()
        };
        let chunk = |t: &str| ChunkRecord {
            ord: 0,
            text: t.into(),
            tokens: 3,
            ..Default::default()
        };
        mgr.upsert_document(run, &rec, &[chunk("photosynthesis in algae")])
            .unwrap();
        let hits = |m: &DbManager, q: &str| -> i64 {
            m.conn
                .query_row(
                    "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                    params![fts_query(q).unwrap()],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(hits(&mgr, "algae"), 1);
        assert_eq!(hits(&mgr, "photosynth"), 1, "prefix + porter stemming");
        mgr.upsert_document(run, &rec, &[chunk("glaciers")])
            .unwrap();
        assert_eq!(hits(&mgr, "algae"), 0, "old chunks replaced");
        assert_eq!(hits(&mgr, "glacier"), 1);
    }

    #[test]
    fn migrates_a_v0_library_in_place() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("library.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT, query TEXT NOT NULL,
                    folder_path TEXT NOT NULL, created_at DATETIME DEFAULT CURRENT_TIMESTAMP);
                 CREATE TABLE documents (id INTEGER PRIMARY KEY AUTOINCREMENT, run_id INTEGER,
                    title TEXT NOT NULL, url TEXT NOT NULL UNIQUE, source TEXT NOT NULL,
                    authors TEXT, year TEXT, abstract TEXT, local_path TEXT, text_path TEXT,
                    extract_error TEXT, size_bytes INTEGER);
                 INSERT INTO runs (query, folder_path) VALUES ('q', 'f');
                 INSERT INTO documents (run_id, title, url, source) VALUES (1, 'Old', 'https://o', 'web');",
            )
            .unwrap();
        }
        let mgr = DbManager::new(&path).unwrap();
        let v: i64 = mgr
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        let doi: Option<String> = mgr
            .conn
            .query_row("SELECT doi FROM documents WHERE url='https://o'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(doi.is_none());
    }

    #[test]
    fn fts_query_is_injection_safe() {
        assert_eq!(fts_query("a"), None);
        assert_eq!(
            fts_query("heart OR \"x NEAR(").unwrap(),
            "\"heart\" OR \"or\" OR \"near\"*"
        );
    }

    #[test]
    fn library_index_round_trips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let idx = LibraryIndex::open(tmp.path()).unwrap();
        idx.record(&["https://a".into(), "https://b".into()], "abc", "/x/y.pdf")
            .unwrap();
        assert_eq!(idx.lookup_urls(&["https://b".into()]), vec!["/x/y.pdf"]);
        assert!(idx.lookup_urls(&["https://c".into()]).is_empty());
    }
}
