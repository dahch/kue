use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::db::Database;
use crate::rag::embeddings::EmbeddingModel;

use super::{emit_expired_hints, generate_and_emit_hint, HintCommand, HintScheduler};

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
            eprintln!("[kue] Hint worker thread started");

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

            eprintln!("[kue] Hint worker thread ended");
        })
        .expect("failed to spawn hint worker thread")
}
