use tauri::Manager;

use crate::db::{Database, SETTING_FIRST_RUN};

const FIRST_RUN_DONE: &str = "done";

/// Core logic: check whether the `first_run` setting is present and `"done"`.
/// Returns `true` if onboarding is needed. Separated from Tauri wrapper for
/// testability.
pub fn is_first_run_inner(db: &Database) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let result: Result<String, _> = conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [SETTING_FIRST_RUN],
        |row| row.get(0),
    );
    match result {
        Ok(v) => Ok(v != FIRST_RUN_DONE),
        Err(_) => Ok(true),
    }
}

#[tauri::command]
pub fn is_first_run(db: tauri::State<'_, Database>) -> Result<bool, String> {
    is_first_run_inner(db.inner())
}

/// Core logic: set the `first_run` setting to `"done"`. Separated from Tauri
/// wrapper for testability.
pub fn mark_onboarding_done_inner(db: &Database) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        [SETTING_FIRST_RUN, FIRST_RUN_DONE],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn mark_onboarding_done(db: tauri::State<'_, Database>) -> Result<(), String> {
    mark_onboarding_done_inner(db.inner())
}

/// Attempt to access shareable content via ScreenCaptureKit.
/// macOS shows the system permission dialog on first call.
/// Returns `true` if permission is already granted.
#[tauri::command]
pub fn check_screen_recording_permission() -> Result<bool, String> {
    let result = screencapturekit::sc_shareable_content::SCShareableContent::try_current();
    match result {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Check whether the embedding model has been loaded into Tauri state.
/// The model is loaded during app setup (synchronous). This command lets
/// the frontend poll and show a loading indicator while it finishes.
#[tauri::command]
pub fn is_embedding_model_loaded(app_handle: tauri::AppHandle) -> bool {
    app_handle
        .try_state::<std::sync::Arc<std::sync::Mutex<crate::rag::embeddings::EmbeddingModel>>>()
        .is_some()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_and_migrate;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Once;

    static VEC_INIT: Once = Once::new();
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn ensure_vec() {
        VEC_INIT.call_once(|| {
            crate::db::register_vec_extension();
        });
    }

    fn unique_dir() -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!("kue_test_onb_{}_{}", std::process::id(), id));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn setup_db(path: &Path) -> Database {
        ensure_vec();
        open_and_migrate(path).expect("test db migration should succeed")
    }

    struct CleanupDir(PathBuf);
    impl Drop for CleanupDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // -----------------------------------------------------------------------
    // is_first_run_inner
    // -----------------------------------------------------------------------

    #[test]
    fn is_first_run_returns_true_when_pending() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("pending.db");
        let db = setup_db(&db_path);
        // Migration seeds first_run = 'pending'
        let result = is_first_run_inner(&db).unwrap();
        assert!(result, "'pending' should mean first run");
    }

    #[test]
    fn is_first_run_returns_false_when_done() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("done.db");
        let db = setup_db(&db_path);
        mark_onboarding_done_inner(&db).unwrap();
        let result = is_first_run_inner(&db).unwrap();
        assert!(
            !result,
            "should not need onboarding after mark_onboarding_done"
        );
    }

    #[test]
    fn is_first_run_returns_true_for_arbitrary_value() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("arbitrary.db");
        let db = setup_db(&db_path);
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('first_run', 'in_progress')",
                [],
            )
            .unwrap();
        }
        let result = is_first_run_inner(&db).unwrap();
        assert!(
            result,
            "arbitrary value should be treated as first run (only 'done' skips)"
        );
    }

    #[test]
    fn is_first_run_handles_poisoned_mutex() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("poisoned.db");

        let underlying = rusqlite::Connection::open(&db_path).unwrap();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(underlying));
        let cloned = std::sync::Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _guard = cloned.lock().unwrap();
            panic!("intentional poison");
        });
        let _ = handle.join();
        let poisoned = std::sync::Arc::try_unwrap(shared).unwrap();
        let db = Database {
            conn: poisoned,
            path: db_path.clone(),
        };

        let result = is_first_run_inner(&db);
        assert!(result.is_err(), "poisoned mutex should return error");
        assert!(result.unwrap_err().contains("poison"));
    }

    // -----------------------------------------------------------------------
    // mark_onboarding_done_inner
    // -----------------------------------------------------------------------

    #[test]
    fn mark_onboarding_done_sets_value() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("set.db");
        let db = setup_db(&db_path);

        mark_onboarding_done_inner(&db).unwrap();

        let conn = db.conn.lock().unwrap();
        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='first_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "done");
    }

    #[test]
    fn mark_onboarding_done_is_idempotent() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("idem.db");
        let db = setup_db(&db_path);

        mark_onboarding_done_inner(&db).unwrap();
        mark_onboarding_done_inner(&db).unwrap();

        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key='first_run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idempotent: only one row should exist");
    }

    // -----------------------------------------------------------------------
    // mark_onboarding_done — error paths
    // -----------------------------------------------------------------------

    #[test]
    fn mark_onboarding_done_handles_poisoned_mutex() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("poison_done.db");

        let underlying = rusqlite::Connection::open(&db_path).unwrap();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(underlying));
        let cloned = std::sync::Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _guard = cloned.lock().unwrap();
            panic!("intentional poison for mark_onboarding_done");
        });
        let _ = handle.join();
        let poisoned = std::sync::Arc::try_unwrap(shared).unwrap();
        let db = Database {
            conn: poisoned,
            path: db_path.clone(),
        };

        let result = mark_onboarding_done_inner(&db);
        assert!(result.is_err(), "poisoned mutex should return error");
        assert!(result.unwrap_err().contains("poison"));
    }

    #[test]
    fn mark_onboarding_done_handles_execute_error() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("readonly.db");

        // Set up the database, then close it and make the file read-only.
        {
            let db = setup_db(&db_path);
            // Write the initial value so table/schema are established.
            mark_onboarding_done_inner(&db).unwrap();
        } // db is dropped, connection closed

        // Make the file read-only (remove write permission for owner).
        let mut perms = std::fs::metadata(&db_path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&db_path, perms).unwrap();

        // Open a fresh connection. On macOS, Connection::open on a
        // read-only file succeeds (file exists, flags allow read);
        // the INSERT later should fail with a readonly error.
        if let Ok(ro_conn) = rusqlite::Connection::open(&db_path) {
            let ro_db = Database {
                conn: std::sync::Mutex::new(ro_conn),
                path: db_path,
            };
            let result = mark_onboarding_done_inner(&ro_db);
            assert!(result.is_err(), "write to read-only database should fail");
            let err = result.unwrap_err();
            assert!(
                err.contains("readonly") || err.contains("unable to open"),
                "error should mention readonly: {}",
                err
            );
        }
        // If open fails entirely we can't test this path, but that's
        // platform-dependent — the test at least verifies the code
        // path is exercised when possible.
    }

    // -----------------------------------------------------------------------
    // is_first_run_inner — SQL error (not just missing key)
    // -----------------------------------------------------------------------

    #[test]
    fn is_first_run_returns_true_on_query_error() {
        // Create a Database with a bare connection (no settings table).
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("bare.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // Create only a dummy table — no `settings` table at all.
        conn.execute_batch("CREATE TABLE IF NOT EXISTS dummy (id INTEGER PRIMARY KEY);")
            .unwrap();

        let db = Database {
            conn: std::sync::Mutex::new(conn),
            path: db_path,
        };

        // The query on non-existent `settings` table returns an SQL error.
        // is_first_run_inner returns Ok(true) for any query error.
        let result = is_first_run_inner(&db).unwrap();
        assert!(
            result,
            "SQL error should be treated as first run (safe default)"
        );
    }

    // -----------------------------------------------------------------------
    // is_first_run_inner — first_run setting does not exist at all
    // -----------------------------------------------------------------------

    #[test]
    fn is_first_run_returns_true_when_setting_missing() {
        let _tmp = CleanupDir(unique_dir());
        let db_path = _tmp.0.join("missing.db");
        let db = setup_db(&db_path);

        // Delete the first_run setting so query_row finds no row
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("DELETE FROM settings WHERE key = 'first_run'", [])
                .unwrap();
        }

        let result = is_first_run_inner(&db).unwrap();
        assert!(result, "missing setting should mean first run");
    }
}
