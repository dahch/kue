use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::db::Database;
use crate::rag::embeddings::EmbeddingModel;

use super::{emit_expired_hints, generate_and_emit_hint, HintCommand, HintScheduler};

/// Spawns the hint-worker background thread.
///
/// The thread owns a loop that:
/// 1. Receives `HintCommand::Process(job)` → generates & emits a hint via RAG.
/// 2. Receives `HintCommand::CancelSession(sid)` → cancels all pending hints for that session.
/// 3. Times out every 500ms → drains expired shadow-mode hints from the scheduler.
/// 4. On disconnect from the sender → exits the loop cleanly.
///
/// # Why `start_hint_worker` cannot be tested in unit tests
///
/// Full testing requires a `tauri::AppHandle<Wry>` which cannot be constructed
/// without a running Tauri application. The component functions that the worker
/// delegates to (`generate_and_emit_hint`, `emit_expired_hints`, `HintScheduler`)
/// are individually unit-tested in `orchestrator/mod.rs`.
pub fn start_hint_worker(
    rx: Receiver<HintCommand>,
    app_handle: tauri::AppHandle,
    db: Database,
    model: Arc<Mutex<EmbeddingModel>>,
    scheduler: Arc<HintScheduler>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("kue-hint-worker".into())
        .spawn(move || {
            log::info!("Hint worker thread started");

            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(HintCommand::Process(job)) => {
                        generate_and_emit_hint(
                            &job.text,
                            job.qtype,
                            &job.mode,
                            &job.session_id,
                            Some(&app_handle),
                            &scheduler,
                            &db,
                            &*model,
                        );
                    }
                    Ok(HintCommand::CancelSession(sid)) => {
                        scheduler.cancel_all(&sid);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        emit_expired_hints(&app_handle, &scheduler);
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            log::info!("Hint worker thread ended");
        })
        .expect("failed to spawn hint worker thread")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::HintJob;

    // -----------------------------------------------------------------------
    // Compile-time signature checks
    // -----------------------------------------------------------------------

    /// Verifies `start_hint_worker` returns a `JoinHandle<()>`.
    #[test]
    fn start_hint_worker_signature_is_valid() {
        fn _check(
            _f: fn(
                Receiver<HintCommand>,
                tauri::AppHandle,
                Database,
                Arc<Mutex<EmbeddingModel>>,
                Arc<HintScheduler>,
            ) -> JoinHandle<()>,
        ) {
        }
        _check(start_hint_worker);
    }

    /// The function type must be `Send` because it's used across thread boundaries.
    #[test]
    fn start_hint_worker_fn_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<
            fn(
                Receiver<HintCommand>,
                tauri::AppHandle,
                Database,
                Arc<Mutex<EmbeddingModel>>,
                Arc<HintScheduler>,
            ) -> JoinHandle<()>,
        >();
    }

    // -----------------------------------------------------------------------
    // HintCommand enum completeness checks
    // -----------------------------------------------------------------------

    #[test]
    fn hint_command_variants_are_accessible() {
        // Verify both variants can be constructed — catches dead-code warnings
        let process = HintCommand::Process(HintJob {
            session_id: "test".into(),
            text: "test".into(),
            qtype: crate::classifier::QuestionType::Technical,
            mode: "practice".into(),
        });
        let cancel = HintCommand::CancelSession("test".into());

        match process {
            HintCommand::Process(ref job) => assert_eq!(job.session_id, "test"),
            _ => panic!("expected Process variant"),
        }
        match cancel {
            HintCommand::CancelSession(ref sid) => assert_eq!(sid, "test"),
            _ => panic!("expected CancelSession variant"),
        }
    }

    /// Verify HintCommand can be cloned (needed for channel sends).
    #[test]
    fn hint_command_is_clonable() {
        let a = HintCommand::CancelSession("sess-1".into());
        let b = a.clone();
        match (a, b) {
            (HintCommand::CancelSession(s1), HintCommand::CancelSession(s2)) => {
                assert_eq!(s1, s2);
            }
            _ => panic!("clone should preserve variant and data"),
        }
    }

    // -----------------------------------------------------------------------
    // HintJob completeness check
    // -----------------------------------------------------------------------

    #[test]
    fn hint_job_is_clone_and_debug() {
        let job = HintJob {
            session_id: "sess".into(),
            text: "text".into(),
            qtype: crate::classifier::QuestionType::Technical,
            mode: "shadow".into(),
        };
        let cloned = job.clone();
        assert_eq!(job.session_id, cloned.session_id);
        assert_eq!(job.text, cloned.text);
        assert_eq!(job.qtype, cloned.qtype);
        assert_eq!(job.mode, cloned.mode);
    }
}
