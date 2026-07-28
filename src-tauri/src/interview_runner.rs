use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use crate::db::Database;
use crate::interview_plan::InterviewPlan;
use crate::tts;

// ---------------------------------------------------------------------------
// Events emitted to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct InterviewQuestionEvent {
    question_index: usize,
    total_questions: usize,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct InterviewStatusEvent {
    status: String, // "speaking", "listening", "finished"
}

// ---------------------------------------------------------------------------
// Interview runner commands (from frontend)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum InterviewCommand {
    Start { session_id: String, plan: InterviewPlan },
    NextQuestion,
    Stop,
}

// ---------------------------------------------------------------------------
// Shared handle
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct InterviewRunnerHandle {
    tx: Sender<InterviewCommand>,
}

impl InterviewRunnerHandle {
    pub fn send(&self, cmd: InterviewCommand) -> Result<(), String> {
        self.tx.send(cmd).map_err(|_| "Interview runner disconnected".to_string())
    }
}

// ---------------------------------------------------------------------------
// The runner itself
// ---------------------------------------------------------------------------

pub struct InterviewRunner {
    db: Database,
    handle: Option<JoinHandle<()>>,
    cmd_tx: Sender<InterviewCommand>,
}

impl InterviewRunner {
    pub fn new(app_handle: tauri::AppHandle, db: Database) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<InterviewCommand>();

        let mut runner = Self {
            db,
            handle: None,
            cmd_tx: cmd_tx.clone(),
        };

        let handle = {
            let db_clone = runner.db.clone();
            thread::Builder::new()
                .name("kue-interview-runner".into())
                .spawn(move || run_interview_loop(app_handle, db_clone, cmd_rx))
                .expect("failed to spawn interview runner thread")
        };

        runner.handle = Some(handle);
        runner
    }

    pub fn handle(&self) -> InterviewRunnerHandle {
        InterviewRunnerHandle {
            tx: self.cmd_tx.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn stop(&mut self) {
        let _ = self.cmd_tx.send(InterviewCommand::Stop);
        if let Some(h) = self.handle.take() {
            // Give the thread a moment to process Stop and exit cleanly.
            // Don't block indefinitely — the thread's recv_timeout will let
            // it notice the channel is closed within ~300ms.
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Interview state machine
// ---------------------------------------------------------------------------

enum Phase {
    /// Speaking: TTS is running in a background thread.
    Speaking { tts_handle: Option<JoinHandle<()>> },
    /// Listening: waiting for the user's answer budget to expire.
    Listening { started_at: Instant },
}

struct RunningInterview {
    session_id: String,
    plan: InterviewPlan,
    question_index: usize,
    phase: Phase,
}

impl RunningInterview {
    fn new(session_id: String, plan: InterviewPlan) -> Self {
        Self {
            session_id,
            plan,
            question_index: 0,
            phase: Phase::Speaking { tts_handle: None },
        }
    }

    fn current_question(&self) -> Option<&crate::interview_plan::PlannedQuestion> {
        self.plan.questions.get(self.question_index)
    }

    fn is_done(&self) -> bool {
        self.question_index >= self.plan.questions.len()
    }
}

/// The main interview loop — uses recv_timeout so commands (Skip, Stop) are
/// processed promptly instead of waiting for the entire TTS + listening cycle
/// to complete.
fn run_interview_loop(
    app_handle: tauri::AppHandle,
    db: Database,
    cmd_rx: Receiver<InterviewCommand>,
) {
    let mut state: Option<RunningInterview> = None;

    loop {
        match cmd_rx.recv_timeout(Duration::from_millis(300)) {
            // -- Command received ------------------------------------------------
            Ok(InterviewCommand::Start { session_id, plan }) => {
                // Clean up previous state if any
                if let Some(prev) = state.take() {
                    finish_interview(&app_handle, &db, &prev.session_id, "restarted");
                }

                let mut running = RunningInterview::new(session_id, plan);
                start_current_question(&app_handle, &mut running);
                state = Some(running);
            }

            Ok(InterviewCommand::NextQuestion) => {
                if let Some(ref mut running) = state {
                    advance_question(&app_handle, running);
                    if running.is_done() {
                        finish_interview(&app_handle, &db, &running.session_id, "completed");
                        state = None;
                    }
                }
            }

            Ok(InterviewCommand::Stop) => {
                if let Some(running) = state.take() {
                    finish_interview(&app_handle, &db, &running.session_id, "stopped");
                }
                break;
            }

            Err(RecvTimeoutError::Disconnected) => {
                if let Some(running) = state.take() {
                    finish_interview(&app_handle, &db, &running.session_id, "disconnected");
                }
                break;
            }

            // -- Timeout (no command) — tick the state machine -------------------
            Err(RecvTimeoutError::Timeout) => {
                if let Some(ref mut running) = state {
                    tick(&app_handle, &db, running);
                    if running.is_done() {
                        finish_interview(&app_handle, &db, &running.session_id, "completed");
                        state = None;
                    }
                }
            }
        }
    }

    log::info!("Interview runner thread ended");
}

// ---------------------------------------------------------------------------
// State machine helpers
// ---------------------------------------------------------------------------

/// Starts the TTS for the current question and transitions to Speaking.
fn start_current_question(app_handle: &tauri::AppHandle, running: &mut RunningInterview) {
    let Some(q) = running.current_question() else {
        return;
    };

    let _ = app_handle.emit("interview-question", InterviewQuestionEvent {
        question_index: running.question_index,
        total_questions: running.plan.questions.len(),
        text: q.text.clone(),
    });

    let _ = app_handle.emit("interview-status", InterviewStatusEvent {
        status: "speaking".to_string(),
    });

    let text = q.text.clone();
    let ah = app_handle.clone();
    let tts_handle = thread::spawn(move || {
        if let Err(e) = tts::speak(&text) {
            log::warn!("TTS speak failed: {e}");
        }
        let _ = ah.emit("interview-status", InterviewStatusEvent {
            status: "listening".to_string(),
        });
    });

    running.phase = Phase::Speaking { tts_handle: Some(tts_handle) };
}

/// Called on each tick (~300ms). Checks whether TTS has finished (→Listening)
/// and whether the listening budget has elapsed (→advance).
fn tick(app_handle: &tauri::AppHandle, db: &Database, running: &mut RunningInterview) {
    match &running.phase {
        Phase::Speaking { tts_handle } => {
            if tts_handle.as_ref().map_or(true, |h| h.is_finished()) {
                // TTS finished — transition to listening.
                running.phase = Phase::Listening { started_at: Instant::now() };
            }
        }
        Phase::Listening { started_at } => {
            let budget = running.current_question().map(|q| q.budget_seconds.max(10)).unwrap_or(10);
            if started_at.elapsed().as_secs() >= budget as u64 {
                advance_question(app_handle, running);
                if running.is_done() {
                    finish_interview(app_handle, db, &running.session_id, "completed");
                }
            }
        }
    }
}

/// Moves to the next question and starts its TTS.
fn advance_question(app_handle: &tauri::AppHandle, running: &mut RunningInterview) {
    running.question_index += 1;
    if !running.is_done() {
        start_current_question(app_handle, running);
    }
}

fn finish_interview(
    app_handle: &tauri::AppHandle,
    db: &Database,
    session_id: &str,
    reason: &str,
) {
    let _ = app_handle.emit("interview-status", InterviewStatusEvent {
        status: "finished".to_string(),
    });
    let _ = app_handle.emit("interview-finished", serde_json::json!({
        "session_id": session_id,
        "reason": reason,
    }));

    if let Ok(conn) = db.conn.lock() {
        let _ = conn.execute(
            "UPDATE sessions SET current_question_index = -1 WHERE id = ?1",
            rusqlite::params![session_id],
        );
    }

    log::info!("Interview finished for session {} (reason: {})", session_id, reason);
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_ai_interview(
    session_id: String,
    _job_description: String,
    interview_plan_json: String,
    app_handle: tauri::AppHandle,
    db: tauri::State<'_, Database>,
    interview_runner: tauri::State<'_, Mutex<InterviewRunner>>,
) -> Result<(), String> {
    let plan: InterviewPlan = serde_json::from_str(&interview_plan_json)
        .map_err(|e| format!("Invalid interview plan JSON: {e}"))?;

    if plan.questions.is_empty() {
        return Err("Interview plan has no questions".to_string());
    }

    // Persist the plan to the session
    {
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sessions SET interview_plan = ?1, current_question_index = 0 WHERE id = ?2",
            rusqlite::params![interview_plan_json, session_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let mut runner = interview_runner.lock().map_err(|e| e.to_string())?;

    // Grab the old thread handle and sender before replacing the runner.
    let old_handle = runner.handle.take();
    let old_tx = runner.cmd_tx.clone();
    drop(runner);

    // Stop the old runner outside the lock
    if let Some(old_handle) = old_handle {
        let _ = old_tx.send(InterviewCommand::Stop);
        let _ = old_handle.join();
    }

    // Re-lock and install the new runner
    let mut runner = interview_runner.lock().map_err(|e| e.to_string())?;
    let new_runner = InterviewRunner::new(app_handle.clone(), Database::clone(db.inner()));
    *runner = new_runner;
    let handle = runner.handle();
    handle.send(InterviewCommand::Start { session_id, plan })
}

#[tauri::command]
pub fn skip_ai_question(
    interview_runner: tauri::State<'_, Mutex<InterviewRunner>>,
) -> Result<(), String> {
    let runner = interview_runner.lock().map_err(|e| e.to_string())?;
    runner.handle().send(InterviewCommand::NextQuestion)
}

#[tauri::command]
pub fn stop_ai_interview(
    interview_runner: tauri::State<'_, Mutex<InterviewRunner>>,
) -> Result<(), String> {
    let runner = interview_runner.lock().map_err(|e| e.to_string())?;
    let _ = runner.handle().send(InterviewCommand::Stop);
    // Don't call runner.stop() here — that calls h.join() which blocks while
    // the old loop was stuck. The loop now uses recv_timeout and will exit
    // promptly when it receives Stop.
    Ok(())
}
