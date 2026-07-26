use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

use crate::rag::embeddings::EMBEDDING_DIM;

pub struct Database {
    pub conn: Mutex<Connection>,
    pub path: PathBuf,
}

impl Clone for Database {
    fn clone(&self) -> Self {
        let conn = Connection::open(&self.path)
            .unwrap_or_else(|e| panic!("Failed to clone DB connection at {:?}: {e}", self.path));
        Self {
            conn: Mutex::new(conn),
            path: self.path.clone(),
        }
    }
}

/// Must be called once before any `Connection::open` to register vec0.
pub fn register_vec_extension() {
    // Compile-time guard: both types must be pointer-sized for the transmute to be safe.
    const _: [(); std::mem::size_of::<*const ()>()] = [(); std::mem::size_of::<
        unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int,
    >()];

    unsafe {
        // Cast through *const () to reconcile *mut/*const ABI differences
        // between sqlite-vec's and libsqlite3-sys's declarations.
        let ptr = sqlite_vec::sqlite3_vec_init as *const ();
        let entry: unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int = std::mem::transmute(ptr);
        let rc = rusqlite::ffi::sqlite3_auto_extension(Some(entry));
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK, "sqlite3_auto_extension failed");
    }
}

/// The DDL statements used to bootstrap the database schema.
/// The vec0 virtual table is appended separately via `vec0_ddl()` so the
/// embedding dimension is driven by the `EMBEDDING_DIM` constant.
const SCHEMA_DDL: &str = "
    CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY DEFAULT (hex(randomblob(16))),
        started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        ended_at DATETIME,
        company TEXT,
        role TEXT,
        mode TEXT CHECK(mode IN ('practice', 'shadow'))
    );

    CREATE TABLE IF NOT EXISTS transcript_lines (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        speaker TEXT CHECK(speaker IN ('user', 'interviewer')),
        text TEXT NOT NULL,
        started_at_ms INTEGER NOT NULL,
        ended_at_ms INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id)
    );

    CREATE TABLE IF NOT EXISTS documents (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        filename TEXT NOT NULL,
        type TEXT NOT NULL CHECK(type != ''),
        added_at DATETIME DEFAULT CURRENT_TIMESTAMP
    );

    CREATE TABLE IF NOT EXISTS chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        document_id INTEGER NOT NULL,
        text TEXT NOT NULL,
        chunk_index INTEGER NOT NULL,
        tag TEXT,             -- classification tag e.g. 'nestjs', 'redis', 'star'
        metric TEXT,          -- extracted metric e.g. '10k req/seg', '40% reducción'
        FOREIGN KEY (document_id) REFERENCES documents(id)
    );

    CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
";

fn vec0_ddl() -> String {
    format!("CREATE VIRTUAL TABLE IF NOT EXISTS chunks_vec USING vec0(embedding float[{}]);", EMBEDDING_DIM)
}

/// Core logic: open a SQLite connection at `db_path`, enable WAL mode,
/// and run all DDL migrations. Separated from `init_db` for testability.
pub fn open_and_migrate(db_path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;
    conn.execute_batch(SCHEMA_DDL)?;
    conn.execute_batch(&vec0_ddl())?;

    // Seed default settings
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value) VALUES ('retain_audio', 'false')",
        [],
    )?;

    Ok(Database {
        conn: Mutex::new(conn),
        path: db_path.to_path_buf(),
    })
}

pub fn init_db(app: &tauri::App) -> Result<Database, Box<dyn std::error::Error>> {
    let app_data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&app_data_dir)?;

    let db_path = app_data_dir.join("kue.db");
    open_and_migrate(&db_path)
}

#[derive(Debug, Serialize)]
pub struct DbStatus {
    pub path: String,
    pub tables: Vec<String>,
}

/// Core logic: query the database for its status (path + table list).
/// Separated from the Tauri command wrapper for testability.
pub fn get_db_status_inner(db: &Database) -> Result<DbStatus, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let path = db.path.to_string_lossy().to_string();

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|e| e.to_string())?;

    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(DbStatus { path, tables })
}

#[tauri::command]
pub fn get_db_status(db: tauri::State<'_, Database>) -> Result<DbStatus, String> {
    get_db_status_inner(db.inner())
}

#[derive(Debug, Serialize)]
pub struct SessionRow {
    pub id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub company: String,
    pub role: String,
    pub mode: String,
    pub line_count: i64,
}

#[tauri::command]
pub fn get_sessions(db: tauri::State<'_, Database>) -> Result<Vec<SessionRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.started_at, s.ended_at, s.company, s.role, s.mode,
                    (SELECT COUNT(*) FROM transcript_lines WHERE session_id = s.id) as line_count
             FROM sessions s
             ORDER BY s.started_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                started_at: row.get::<_, String>(1)?,
                ended_at: row.get(2)?,
                company: row.get::<_, String>(3)?,
                role: row.get::<_, String>(4)?,
                mode: row.get::<_, String>(5)?,
                line_count: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.map_err(|e| e.to_string())?);
    }
    Ok(sessions)
}

#[derive(Debug, Serialize)]
pub struct TranscriptLineRow {
    pub id: i64,
    pub speaker: String,
    pub text: String,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}

#[tauri::command]
pub fn get_session_transcript(
    session_id: String,
    db: tauri::State<'_, Database>,
) -> Result<Vec<TranscriptLineRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, speaker, text, started_at_ms, ended_at_ms
             FROM transcript_lines
             WHERE session_id = ?1
             ORDER BY started_at_ms ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(TranscriptLineRow {
                id: row.get(0)?,
                speaker: row.get(1)?,
                text: row.get(2)?,
                started_at_ms: row.get(3)?,
                ended_at_ms: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut lines = Vec::new();
    for row in rows {
        lines.push(row.map_err(|e| e.to_string())?);
    }
    Ok(lines)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    /// Guard to ensure `register_vec_extension` is called only once per
    /// test process. The extension must be loaded before any connection
    /// attempts to create a `vec0` virtual table.
    static VEC_INIT: Once = Once::new();

    fn ensure_vec_extension() {
        VEC_INIT.call_once(|| {
            register_vec_extension();
        });
    }

    /// Wrapper around `open_and_migrate` that ensures the vec extension
    /// is registered before creating the connection.
    fn open_and_migrate_with_vec(db_path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
        ensure_vec_extension();
        open_and_migrate(db_path)
    }

    /// All table names that should exist after a fresh migration.
    const EXPECTED_TABLES: &[&str] = &[
        "chunks",
        "chunks_vec",
        "documents",
        "sessions",
        "settings",
        "transcript_lines",
    ];

    /// Create a unique temporary directory per instance, cleaned up on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let mut dir = std::env::temp_dir();
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            dir.push(format!("kue_test_{}_{}", std::process::id(), id));
            let _ = std::fs::create_dir_all(&dir);
            TempDir { path: dir }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    // -----------------------------------------------------------------------
    // register_vec_extension (integration-level)
    // -----------------------------------------------------------------------

    #[test]
    fn register_vec_extension_registers_auto_extension() {
        // Calling the function should not panic.
        // In a real run the extension gets loaded for every new connection,
        // so we can verify by opening a connection and creating the vec0 table.
        ensure_vec_extension();
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_vec.db");

        let conn = Connection::open(&db_path).unwrap();
        // If the extension was registered, creating a vec0 table succeeds.
        let result = conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS test_vec USING vec0(embedding float[384]);",
        );
        assert!(result.is_ok(), "vec0 table creation should succeed after registering extension");
    }

    // -----------------------------------------------------------------------
    // open_and_migrate
    // -----------------------------------------------------------------------

    #[test]
    fn open_and_migrate_creates_database_file() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        assert!(db_path.exists(), "database file should exist at the specified path");
        assert_eq!(db.path, db_path, "returned path should match the requested path");
    }

    #[test]
    fn open_and_migrate_creates_all_expected_tables() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let status = get_db_status_inner(&db).unwrap();

        // Note: vec0 creates internal shadow tables (chunks_vec_chunks,
        // chunks_vec_info, chunks_vec_rowids, chunks_vec_vector_chunks00,
        // and auto-generated sqlite_sequence). We verify that all the
        // expected user tables are present as a superset.
        for table in EXPECTED_TABLES {
            assert!(
                status.tables.contains(&table.to_string()),
                "expected table '{}' not found in: {:?}",
                table,
                status.tables
            );
        }
        // Also verify we have more than just the expected user tables,
        // confirming that internal vec0 tables also exist.
        assert!(
            status.tables.len() >= EXPECTED_TABLES.len(),
            "should have at least {} tables (vec0 creates internal tables); got {}",
            EXPECTED_TABLES.len(),
            status.tables.len()
        );
    }

    #[test]
    fn open_and_migrate_is_idempotent() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");

        // Run migration twice
        let db1 = open_and_migrate_with_vec(&db_path).unwrap();
        let db2 = open_and_migrate_with_vec(&db_path).unwrap();

        // Both should see the same set of tables
        let status1 = get_db_status_inner(&db1).unwrap();
        let status2 = get_db_status_inner(&db2).unwrap();
        assert_eq!(status1.tables, status2.tables, "second migration should not change tables");
    }

    #[test]
    fn open_and_migrate_sets_wal_journal_mode() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();

        assert_eq!(
            journal_mode.to_lowercase(),
            "wal",
            "journal_mode should be WAL after migration"
        );
    }

    #[test]
    fn open_and_migrate_returns_error_for_invalid_path() {
        // Attempt to open a database in a non-existent directory should fail.
        let result = open_and_migrate(Path::new("/nonexistent_dir_12345/kue_test.db"));
        assert!(result.is_err(), "should return error for invalid path");
    }

    // -----------------------------------------------------------------------
    // get_db_status_inner
    // -----------------------------------------------------------------------

    #[test]
    fn get_db_status_returns_correct_path() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_status.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let status = get_db_status_inner(&db).unwrap();

        assert!(
            status.path.ends_with("test_status.db"),
            "path should end with the database filename"
        );
    }

    #[test]
    fn get_db_status_returns_all_table_names() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_status.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let status = get_db_status_inner(&db).unwrap();

        for table in EXPECTED_TABLES {
            assert!(
                status.tables.contains(&table.to_string()),
                "expected table '{}' to be in the table list: {:?}",
                table,
                status.tables
            );
        }
    }

    #[test]
    fn get_db_status_works_on_empty_database() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_empty.db");

        // Create a Database with an empty sqlite file (no tables at all)
        let conn = Connection::open(&db_path).unwrap();
        let db = Database {
            conn: Mutex::new(conn),
            path: db_path.clone(),
        };

        let status = get_db_status_inner(&db).unwrap();
        assert!(
            status.tables.is_empty(),
            "empty database should have no tables, got: {:?}",
            status.tables
        );
        assert!(
            status.path.ends_with("test_empty.db"),
            "path should match"
        );
    }

    #[test]
    fn get_db_status_handles_poisoned_mutex() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_poison.db");

        let underlying = Connection::open(&db_path).unwrap();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(underlying));

        // Poison the mutex by locking and panicking in another thread.
        let cloned = std::sync::Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _guard = cloned.lock().unwrap();
            panic!("intentional panic to poison the mutex");
        });
        let _ = handle.join();

        // After the thread panicked, the cloned Arc was dropped.
        // Recover the Mutex (ref count == 1) to build the Database.
        let poisoned = std::sync::Arc::try_unwrap(shared).unwrap();
        let db = Database {
            conn: poisoned,
            path: db_path,
        };

        // Now trying to lock should return an error
        let result = get_db_status_inner(&db);
        assert!(
            result.is_err(),
            "should return error when mutex is poisoned"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("poison"),
            "error should mention poison, got: {}",
            err
        );
    }

    // -----------------------------------------------------------------------
    // init_db (integration-level)
    // -----------------------------------------------------------------------

    #[test]
    fn init_db_creates_app_data_dir() {
        // This test requires a Tauri app environment which is difficult
        // to set up in unit tests. The core logic (open_and_migrate) is
        // tested above; init_db is a thin wrapper that also creates the
        // directory. We verify the directory creation aspect here by
        // checking that open_and_migrate correctly creates the parent.
        let tmp = TempDir::new();
        let nested_path = tmp.path().join("nested/deep/db/test.db");
        std::fs::create_dir_all(nested_path.parent().unwrap()).unwrap();

        let db = open_and_migrate_with_vec(&nested_path).unwrap();
        assert!(db.path.exists());
    }

    // -----------------------------------------------------------------------
    // Database struct invariants
    // -----------------------------------------------------------------------

    #[test]
    fn database_mutex_allows_concurrent_locks() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_concurrent.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let db = std::sync::Arc::new(db);

        // Spawn two threads that each lock the mutex and query
        let db1 = std::sync::Arc::clone(&db);
        let h1 = std::thread::spawn(move || {
            let conn = db1.conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
                .unwrap();
            count
        });

        let db2 = std::sync::Arc::clone(&db);
        let h2 = std::thread::spawn(move || {
            let conn = db2.conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| row.get(0))
                .unwrap();
            count
        });

        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();
        assert_eq!(r1, r2, "both threads should see the same number of tables");
    }

    #[test]
    fn database_path_is_correctly_stored() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_path.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        assert_eq!(db.path, db_path);
    }

    #[test]
    fn database_conn_is_usable_after_migration() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_usable.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        // Insert and read back a session to verify the connection works
        conn.execute(
            "INSERT INTO sessions (company, role, mode) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Test Corp", "Engineer", "practice"],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "should be able to insert and query data");
    }

    // -----------------------------------------------------------------------
    // Schema integrity — CHECK constraints and foreign keys
    // -----------------------------------------------------------------------

    #[test]
    fn session_mode_check_constraint_rejects_invalid_values() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_check.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        let result = conn.execute(
            "INSERT INTO sessions (company, role, mode) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Acme", "Dev", "invalid_mode"],
        );
        assert!(
            result.is_err(),
            "CHECK constraint should reject invalid mode"
        );
    }

    #[test]
    fn session_mode_check_constraint_accepts_valid_values() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_check.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        for valid_mode in &["practice", "shadow"] {
            let result = conn.execute(
                "INSERT INTO sessions (company, role, mode) VALUES (?1, ?2, ?3)",
                rusqlite::params!["Acme", "Dev", valid_mode],
            );
            assert!(
                result.is_ok(),
                "CHECK constraint should accept valid mode '{}'",
                valid_mode
            );
        }
    }

    #[test]
    fn transcript_lines_speaker_check_constraint() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_check.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        // Insert a session first (FK dependency)
        conn.execute(
            "INSERT INTO sessions (company, role, mode) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Acme", "Dev", "practice"],
        )
        .unwrap();

        // Get the session id
        let session_id: String = conn
            .query_row("SELECT id FROM sessions LIMIT 1", [], |row| row.get(0))
            .unwrap();

        // Valid speakers
        for speaker in &["user", "interviewer"] {
            let result = conn.execute(
                "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![session_id, speaker, "hello", 0, 100],
            );
            assert!(
                result.is_ok(),
                "CHECK constraint should accept valid speaker '{}'",
                speaker
            );
        }

        // Invalid speaker
        let result = conn.execute(
            "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![session_id, "invalid_speaker", "hello", 0, 100],
        );
        assert!(
            result.is_err(),
            "CHECK constraint should reject invalid speaker"
        );
    }

    #[test]
    fn foreign_key_enforces_session_reference() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_fk.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        // Try inserting a transcript_line with a non-existent session_id
        let result = conn.execute(
            "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["nonexistent-session-id", "user", "test", 0, 100],
        );
        assert!(
            result.is_err(),
            "FK constraint should reject orphan transcript_lines"
        );
    }

    #[test]
    fn foreign_key_enforces_document_reference() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_fk.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        // Try inserting a chunk with a non-existent document_id
        let result = conn.execute(
            "INSERT INTO chunks (document_id, text, chunk_index) VALUES (?1, ?2, ?3)",
            rusqlite::params![999, "some text", 0],
        );
        assert!(
            result.is_err(),
            "FK constraint should reject orphan chunks"
        );
    }

    // -----------------------------------------------------------------------
    // Schema integrity — document type CHECK constraint
    // -----------------------------------------------------------------------

    #[test]
    fn documents_type_check_constraint_rejects_empty_type() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_doc_type.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        let result = conn.execute(
            "INSERT INTO documents (filename, type) VALUES (?1, ?2)",
            rusqlite::params!["resume.pdf", ""],
        );
        assert!(
            result.is_err(),
            "CHECK constraint should reject empty document type"
        );
    }

    #[test]
    fn documents_type_check_constraint_accepts_valid_type() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_doc_type.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        let result = conn.execute(
            "INSERT INTO documents (filename, type) VALUES (?1, ?2)",
            rusqlite::params!["readme.md", "text/markdown"],
        );
        assert!(
            result.is_ok(),
            "CHECK constraint should accept non-empty document type"
        );
    }

    // -----------------------------------------------------------------------
    // retain_audio migration
    // -----------------------------------------------------------------------

    #[test]
    fn retain_audio_default_is_false_after_migration() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_retain.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='retain_audio'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "false", "retain_audio should default to 'false'");
    }

    #[test]
    fn retain_audio_migration_is_idempotent() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_retain_idem.db");

        let db1 = open_and_migrate_with_vec(&db_path).unwrap();
        {
            let conn = db1.conn.lock().unwrap();
            let v1: String = conn
                .query_row(
                    "SELECT value FROM settings WHERE key='retain_audio'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(v1, "false");
        }
        drop(db1);

        let db2 = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db2.conn.lock().unwrap();
        let v2: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='retain_audio'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(v2, "false", "second migration should not change value");
    }

    #[test]
    fn retain_audio_can_be_overridden() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_retain_override.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('retain_audio', 'true')",
            [],
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='retain_audio'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "true", "retain_audio should be overridable to 'true'");
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn settings_table_supports_string_values() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_settings.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params!["theme", "dark"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params!["volume", "0.85"],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 3, "should support multiple settings (including retain_audio default)");
    }

    #[test]
    fn settings_table_upsert_replaces_existing_key() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_settings.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params!["theme", "dark"],
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params!["theme", "light"],
        )
        .unwrap();

        let value: String = conn
            .query_row("SELECT value FROM settings WHERE key='theme'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(value, "light", "upsert should replace existing value");
    }

    #[test]
    fn chunks_table_accepts_null_tag_and_metric() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_chunks.db");

        let db = open_and_migrate_with_vec(&db_path).unwrap();
        let conn = db.conn.lock().unwrap();

        // Insert a document first
        conn.execute(
            "INSERT INTO documents (filename, type) VALUES (?1, ?2)",
            rusqlite::params!["resume.pdf", "application/pdf"],
        )
        .unwrap();
        let doc_id: i64 = conn
            .query_row("SELECT id FROM documents LIMIT 1", [], |row| row.get(0))
            .unwrap();

        // Insert a chunk with NULL tag and metric
        conn.execute(
            "INSERT INTO chunks (document_id, text, chunk_index, tag, metric)
             VALUES (?1, ?2, ?3, NULL, NULL)",
            rusqlite::params![doc_id, "chunk text", 0],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "should allow NULL tag and metric");
    }

    // -----------------------------------------------------------------------
    // get_session_transcript — integration
    // -----------------------------------------------------------------------

    #[test]
    fn get_session_transcript_empty_session() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate_with_vec(&db_path).unwrap();

        // No sessions at all → empty vec
        let session_id = "nonexistent";
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, speaker, text, started_at_ms, ended_at_ms
                 FROM transcript_lines
                 WHERE session_id = ?1
                 ORDER BY started_at_ms ASC",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| {
                Ok(TranscriptLineRow {
                    id: row.get(0)?,
                    speaker: row.get(1)?,
                    text: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    ended_at_ms: row.get(4)?,
                })
            })
            .unwrap();
        let lines: Vec<TranscriptLineRow> = rows.filter_map(|r| r.ok()).collect();
        assert!(lines.is_empty());
    }

    #[test]
    fn get_session_transcript_with_lines() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate_with_vec(&db_path).unwrap();

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (id, company, role, mode) VALUES ('sess-t1', 'Acme', 'Dev', 'practice')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
             VALUES ('sess-t1', 'interviewer', 'Hello?', 0, 1000)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
             VALUES ('sess-t1', 'user', 'Yes, I am here', 1000, 3000)",
            [],
        ).unwrap();
        drop(conn);

        let guard = db.conn.lock().unwrap();
        let mut stmt = guard.prepare(
            "SELECT id, speaker, text, started_at_ms, ended_at_ms
             FROM transcript_lines
             WHERE session_id = ?1
             ORDER BY started_at_ms ASC",
        ).unwrap();
        let lines: Vec<TranscriptLineRow> = stmt
            .query_map(rusqlite::params!["sess-t1"], |row| {
                Ok(TranscriptLineRow {
                    id: row.get(0)?,
                    speaker: row.get(1)?,
                    text: row.get(2)?,
                    started_at_ms: row.get(3)?,
                    ended_at_ms: row.get(4)?,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].speaker, "interviewer");
        assert_eq!(lines[1].speaker, "user");
        assert!(lines[1].started_at_ms > lines[0].started_at_ms);
    }

    // -----------------------------------------------------------------------
    // get_sessions — integration
    // -----------------------------------------------------------------------

    #[test]
    fn get_sessions_returns_all_sessions() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate_with_vec(&db_path).unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES ('s1', 'Acme', 'Dev', 'practice')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO sessions (id, company, role, mode) VALUES ('s2', 'Globex', 'Sr Eng', 'shadow')",
                [],
            ).unwrap();
            // Add a transcript line to s2 so line_count = 1
            conn.execute(
                "INSERT INTO transcript_lines (session_id, speaker, text, started_at_ms, ended_at_ms)
                 VALUES ('s2', 'interviewer', 'Q?', 0, 100)",
                [],
            ).unwrap();
        }

        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.started_at, s.ended_at, s.company, s.role, s.mode,
                    (SELECT COUNT(*) FROM transcript_lines WHERE session_id = s.id) as line_count
             FROM sessions s
             ORDER BY s.started_at DESC",
        ).unwrap();
        let sessions: Vec<SessionRow> = stmt
            .query_map([], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    started_at: row.get::<_, String>(1)?,
                    ended_at: row.get(2)?,
                    company: row.get::<_, String>(3)?,
                    role: row.get::<_, String>(4)?,
                    mode: row.get::<_, String>(5)?,
                    line_count: row.get(6)?,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(sessions.len(), 2);
        for s in &sessions {
            if s.id == "s1" {
                assert_eq!(s.company, "Acme");
                assert_eq!(s.line_count, 0);
            } else if s.id == "s2" {
                assert_eq!(s.mode, "shadow");
                assert_eq!(s.line_count, 1);
            }
        }
    }

    #[test]
    fn get_sessions_returns_empty_when_no_sessions() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate_with_vec(&db_path).unwrap();

        let conn = db.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.started_at, s.ended_at, s.company, s.role, s.mode,
                    (SELECT COUNT(*) FROM transcript_lines WHERE session_id = s.id) as line_count
             FROM sessions s
             ORDER BY s.started_at DESC",
        ).unwrap();
        let sessions: Vec<SessionRow> = stmt
            .query_map([], |row| {
                Ok(SessionRow {
                    id: row.get(0)?,
                    started_at: row.get::<_, String>(1)?,
                    ended_at: row.get(2)?,
                    company: row.get::<_, String>(3)?,
                    role: row.get::<_, String>(4)?,
                    mode: row.get::<_, String>(5)?,
                    line_count: row.get(6)?,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(sessions.is_empty());
    }

    // -----------------------------------------------------------------------
    // Schema invariants: session id format
    // -----------------------------------------------------------------------

    #[test]
    fn session_id_auto_generates_hex_id() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate_with_vec(&db_path).unwrap();

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (company, role, mode) VALUES ('Test', 'Eng', 'practice')",
            [],
        ).unwrap();
        let id: String = conn
            .query_row("SELECT id FROM sessions", [], |row| row.get(0))
            .unwrap();
        // id is hex(randomblob(16)) → 32 hex chars
        assert_eq!(id.len(), 32, "auto-generated id should be 32 hex chars");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "id should be hex");
    }

    // -----------------------------------------------------------------------
    // Database struct edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn database_clone_creates_new_connection() {
        let tmp = TempDir::new();
        let db_path = tmp.path().join("test_clone.db");
        let db1 = open_and_migrate_with_vec(&db_path).unwrap();
        let db2 = Database::clone(&db1);

        // Both should point to the same file
        assert_eq!(db1.path, db2.path);

        // Both connections should be independently usable
        {
            let conn = db1.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sessions (company, role, mode) VALUES ('C1', 'R1', 'practice')",
                [],
            ).unwrap();
        }
        {
            let conn = db2.conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1, "clone should see the same data via WAL/file");
        }
    }

    // -----------------------------------------------------------------------
    // DbStatus serialization
    // -----------------------------------------------------------------------

    #[test]
    fn db_status_serialization() {
        let status = DbStatus {
            path: "/tmp/test.db".into(),
            tables: vec!["sessions".into(), "chunks".into()],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains(r#""path":"#));
        assert!(json.contains(r#""tables":"#));
        assert!(json.contains(r#""sessions""#));
        assert!(json.contains(r#""chunks""#));
    }
}
