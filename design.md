# Kue — Technical Design

> Current status: **Sprint 0–3 completed, Sprint 4 (overlay) partially implemented** (base infrastructure + dual audio capture + STT + RAG engine + classifier + orchestrator + overlay window + hint display + mic VAD gating). This document describes the living architecture of the project, indicating which parts are implemented and which are planned.

---

## 1. General architecture

```mermaid
graph TD
    subgraph "Frontend (React + TypeScript)"
        A[MainApp<br/>debug RAG UI]
        OV[Overlay<br/>hint display component<br/>3s auto-dismiss]
    end

    subgraph "Tauri Bridge (IPC)"
        B1[tauri::command<br/>get_db_status]
        B2[tauri::command<br/>toggle_audio_capture]
        B3[tauri::command<br/>index_folder_cmd]
        B4[tauri::command<br/>search_context]
        B5[tauri::command<br/>overlay::show_overlay]
    end

    subgraph "Rust Backend (lib.rs)"
        C[db::init_db]
        D[db::register_vec_extension]
        E[setup handler<br/>DB + AudioCapture + Model + overlay config]
        F[audio::capture<br/>AudioCapture]
        EM[rag::embeddings<br/>EmbeddingModel (Mutex)]
        CL[cleanup_orphaned_temp_dirs]
    end

    subgraph "Database Layer (db/mod.rs)"
        G[(SQLite + sqlite-vec)]
        H[sessions]
        I[transcript_lines]
        J[documents]
        K[chunks]
        L[chunks_vec]
        S_KEYS[settings]
    end

    subgraph "Audio Capture (implemented)"
        N[cpal - microphone<br/>Channel A]
        O[screencapturekit-rs - loopback<br/>Channel B]
        P[hound - WAV writer<br/>Background threads]
        MV[audio::mic_vad::MicVadState<br/>VAD on Channel A<br/>tracks speech start timestamps]
    end

    subgraph "Shared Types"
        TT[types::TranscriptLine<br/>types::Speaker]
    end

    subgraph "Overlay Window"
        OW[tauri.conf.json window<br/>transparent, always-on-top<br/>click-through, 400x100]
    end

    subgraph "RAG Engine (implemented)"
        S[rag::embeddings<br/>snowflake-arctic-embed-s + Metal]
        T[rag::indexer<br/>ingest / search / chunk / folder]
    end

    subgraph "STT Module (implemented, lifecycle-integrated)"
        Q1[stt::MoonshineFFIEngine<br/>libmoonshine.dylib via libloading]
        Q2[stt::MoonshineCLIEngine<br/>moonshine-voice CLI fallback]
        Q3[stt::SimpleVAD<br/>energy-based]
        Q4[stt::STTPipeline<br/>thread VAD→STT→classify→events→DB]
    end

    subgraph "Classifier (implemented, lifecycle-integrated)"
        CL1[classifier::classify_text<br/>heuristic + regex + Tauri cmd]
    end

    subgraph "Orchestrator / Hint Engine (implemented)"
        O1[orchestrator::HintScheduler<br/>pending hints with delay]
        O2[orchestrator::HintJob<br/>text + qtype + mode + session]
        O3[orchestrator::worker::<br/>start_hint_worker<br/>kue-hint-worker thread]
        O4[orchestrator::generate_and_emit_hint<br/>classify→RAG→hint→emit]
        OC[orchestrator::should_cancel_hint<br/>mic VAD gating for Shadow mode]
    end

    subgraph "Tauri Events"
        EV1[new-transcript]
        EV2[question-detected]
        EV3[new-hint]
    end

    A -->|invoke| B1
    A -->|invoke| B2
    A -->|invoke| B3
    A -->|invoke| B4
    B1 --> C
    B2 --> F
    B3 --> T
    B4 --> T
    C --> G
    D --> G
    E --> C
    E --> F
    E --> EM
    E --> OW
    N --> P
    N --> MV
    O --> P
    O -->|audio samples| Q4
    Q4 --> CL1
    Q4 --> I
    Q4 -->|HintJob via mpsc| O3
    CL1 --> I
    O3 --> O1
    O3 --> O4
    O4 --> T
    O4 --> EV3
    O3 --> OC
    OC -->|reads| MV
    OC -->|cancels if speaking| O1
    OV -->|listens| EV3
    Q4 --> EV1
    Q4 --> EV2
    G --> H
    G --> I
    G --> J
    G --> K
    G --> L
    G --> S_KEYS
    EM --> S
    S --> T
    T --> G

    style A fill:#e1f5fe,stroke:#0288d1
    style OV fill:#fff3e0,stroke:#f57c00
```

**Legend:** Solid line = implemented. The STT pipeline integrates classification (VAD → STT → classify → events → DB). When a question is detected, the pipeline pushes a `HintJob` to the orchestrator worker thread via an mpsc channel. The worker queries RAG, builds a hint, and either emits it immediately (Practice) or schedules it via `HintScheduler` (Shadow, 2.5s delay). Expired hints are drained every 500ms in the worker's poll loop. In Shadow mode, before emitting each expired hint, the worker checks `MicVadState` (Channel A VAD) — if the user started speaking since the hint was scheduled, the hint is silently cancelled. The `Overlay` React component listens for `new-hint` Tauri events and displays the hint with a 3s auto-dismiss timer.

---

## 2. Layer breakdown

### 2.1 Frontend (`src/`)

- **`main.tsx`** — Entry point React 18, mounts `<App />` on `#root`.
- **`App.tsx`** — App router that detects the window label via `getCurrentWebviewWindow()`. If the label is `"overlay"`, renders the `<Overlay />` component; otherwise renders `<MainApp />` (debug RAG UI).
- **`MainApp`** (inside `App.tsx`) — Debug UI with controls to index folders (RAG) and search vector context. Buttons and text input connected via `invoke()` to the Tauri commands `index_folder_cmd` and `search_context`.
- **`Overlay.tsx`** — Hint display component rendered inside the overlay window. Listens for `new-hint` Tauri events via `listen()`. Shows the hint text in a semi-transparent backdrop-blur container, auto-dismisses after 3 seconds via `setTimeout`.
- **`index.css`** — Tailwind directives (`@tailwind base/components/utilities`).

### 2.2 Tauri Shell (`lib.rs`)

File `src-tauri/src/lib.rs` (~68 lines):

```rust
mod audio;
mod classifier;
mod db;
mod orchestrator;    // Hint engine — wired into lifecycle
mod overlay;         // Overlay window show/hide command
mod rag;
mod stt;             // STT module (Moonshine) — integrated via toggle_audio_capture
mod types;

run() {
    db::register_vec_extension();                     // Registers sqlite-vec before any connection
    audio::capture::AudioCapture::cleanup_orphaned_temp_dirs();  // Cleans orphan WAVs from previous crashes

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            // Configure overlay window for click-through behavior
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
            }

            let database = db::init_db(app)?;
            app.manage(database);

            let recordings_dir = app.path().app_data_dir()?.join("recordings");
            app.manage(audio::capture::AudioCapture::new(recordings_dir));

            let model = Arc::new(std::sync::Mutex::new(
                rag::embeddings::load_embedding_model()?,
            ));
            app.manage(model.clone());

            let scheduler = Arc::new(orchestrator::HintScheduler::new());
            app.manage(scheduler.clone());

            let (hint_tx, hint_rx) = std::sync::mpsc::channel();
            let hint_job_tx: orchestrator::HintJobSender = Arc::new(hint_tx);
            app.manage(hint_job_tx);

            let db_for_worker = db::Database::clone(app.state::<db::Database>().inner());
            orchestrator::worker::start_hint_worker(
                hint_rx,
                app.handle().clone(),
                db_for_worker,
                model,
                scheduler,
            );

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::get_db_status,
            audio::capture::toggle_audio_capture,
            rag::indexer::index_folder_cmd,
            rag::indexer::search_context,
            classifier::classify_text,
            overlay::show_overlay,
        ])
        .run(tauri::generate_context!())
}
```

- **`mod audio`**, **`mod classifier`**, **`mod db`**, **`mod orchestrator`**, **`mod overlay`**, **`mod rag`**, **`mod stt`**, **`mod types`** — Backend submodules.
- **`cleanup_orphaned_temp_dirs()`** — Removes temporary `kue-session-*` directories left by crashed sessions.
- **`plugin(tauri_plugin_shell)`** — Necessary to invoke external processes (planned for BYOK).
- **Overlay click-through:** In `setup()`, the overlay window is configured with `set_ignore_cursor_events(true)` so mouse events pass through to the video call underneath.
- **`app.manage(database)`**, **`app.manage(AudioCapture)`**, **`app.manage(Arc<Mutex<EmbeddingModel>>)`** — Injects `Database`, `AudioCapture` and the embeddings model (wrapped in `Arc` for sharing with the hint worker) as Tauri state.
- **Orchestrator setup:** `HintScheduler` (manages delayed hints in Shadow mode), `HintJobSender` (mpsc channel for dispatching hint jobs), and `start_hint_worker` (spawns `kue-hint-worker` thread that processes jobs and drains expired hints every 500ms).
- **`index_folder_cmd`**, **`search_context`**, **`classify_text`**, and **`show_overlay`** — Tauri commands for RAG, classifier, and overlay window control.
- **STT integration:** The STT pipeline is fully integrated into `toggle_audio_capture`. When `start=true`, the command creates a DB session, spawns an `STTPipeline` thread that consumes audio from the loopback channel, performs VAD → STT → classify → events → persistence → hint job dispatch. The `new-transcript` and `question-detected` events are emitted to the frontend; question-detected lines also push a `HintJob` to the orchestrator worker.

### 2.3 Database Module (`db/mod.rs`)

The substantial module of the app (~850 lines, 27 tests). See §3 for schema details and §4 for tests.

### 2.4 Audio Module (`audio/`)

#### `audio/capture.rs`

Substantial module (~1230 lines, 50 tests) that implements dual audio capture:

- **Microphone (Channel A):** via `cpal`, supports i16 and f32 sample formats with automatic conversion to i16.
- **Loopback (Channel B):** via `screencapturekit-rs` (ScreenCaptureKit), captures the system output audio (interviewer's voice). `excludes_current_process_audio: true` to avoid echo.
- **WAV writer:** two background threads (`kue-wav-mic-A`, `kue-wav-loopback-B`) that write the channels to separate WAV files (16 kHz, mono, 16-bit) using `hound`.
- **`AudioCapture` struct:** managed as Tauri state via `app.manage()`. Exposes `start(mode)`, `stop()`, `toggle(start, mode)`. Exposes `mic_vad_state()` returning `Arc<Mutex<MicVadState>>` for the orchestrator's shadow-mode hint cancellation.
- **`toggle_audio_capture` command:** Tauri command that starts/stops both captures, validating mode (`practice`|`shadow`) against the DB CHECK constraint.
- **51 tests:** cover f32→i16 conversion (edge cases: NaN, infinity, clamping, very small values), state serialization (4 combinations), directory creation, path format, invalid mode (multiple variants), toggle (4 paths), WAV writer (creation, multiple buffers, invalid path, empty buffer, cyclic lifecycle), and consistency between mode validation and DB.

#### `audio/mic_vad.rs`

Moderate module (~365 lines, 22 tests) that wraps `SimpleVAD` for Channel A (mic):

- **`MicVadState`** — Thread-safe struct wrapping an energy-based VAD (`SimpleVAD` from the STT module). Tracks silence→speech transition timestamps on the mic channel.
- **`feed_audio(samples)`** — Feeds i16 mic samples and updates the VAD state. Must be called from the mic capture thread.
- **`has_speech_since(instant)`** — Returns `true` if a silence→speech transition occurred after the given `Instant`. Used by the orchestrator to determine if the user started answering before a Shadow-mode hint's delay expired.
- **`is_currently_speaking()`** — Returns `true` if VAD currently considers the user to be speaking.
- **`reset()`** — Clears all state (called when mic capture restarts).

### 2.5 STT + Classifier Module (`stt/`, `classifier/`)

**STT Module (`stt/`, ~815 non-test lines, 84 tests)** — implements real-time transcription of Channel B, integrated into the app lifecycle via `toggle_audio_capture`:

- **`stt/mod.rs`** — `STTEngine` trait with `load()` and `transcribe_audio_chunk()`, `STTConfig`, blanket impl for `Box<T>`.
- **`stt/ffi.rs`** — `MoonshineFFIEngine`: loads `libmoonshine.dylib` at runtime via `libloading`, declares FFI bindings with `#[repr(C)]` for `CTranscript`, `CTranscriptLine`, etc. Calls the Moonshine streaming API: `create_stream` → `start_stream` → `add_audio_to_stream` → `transcribe_stream`. Frees resources in `Drop`.
- **`stt/cli.rs`** — `MoonshineCLIEngine` (fallback): writes WAV segments to temp with UUID, invokes `moonshine-voice transcribe --wav-path <file>` as a subprocess, parses the last line of output.
- **`stt/vad.rs`** — `SimpleVAD`: voice activity detection by RMS energy with configurable threshold, minimum speech duration, and silence timeout.
- **`stt/pipeline.rs`** — `STTPipeline`: orchestrator that receives audio from loopback via `mpsc::Receiver`, runs VAD + segment buffer + STT engine + calls `classifier::classify()` on each transcribed line + emits `new-transcript` and `question-detected` events + persists in `transcript_lines`.

**Lifecycle integration:** When `toggle_audio_capture` is called with `start=true`, the command creates a DB session (in `sessions` table), spawns an `STTPipeline` thread via `spawn_processing_thread()`, and connects it to the loopback audio stream. The pipeline runs until `stop()` is called, at which point the session is finalized, any pending shadow hints are cancelled via `HintCommand::CancelSession`, and temp WAV files are cleaned up (or retained if `retain_audio` is enabled).

**Auto-detection:** At runtime, tries FFI first (`libmoonshine.dylib` in `MOONSHINE_LIB_DIR` or standard paths), falls back to CLI if the library is not found. No Whisper — Moonshine is the only option.

**84 tests:** cover `parse_transcript` (null ptr, 0 lines, completed/incomplete, empty text, preferred line), `rms` (8 cases), VAD (24 cases: silence, speech, timeout, reset, minimum duration, empty, threshold, boundary, accumulation, reset during speech, etc.), temp WAV writing (3 cases: data, empty, unique names), `STTPipeline` (engine selection, load delegation, start/end session, process chunk, flush segment, DB persistence, poisoned mutex, special characters, multiple lines, hint job dispatch).

**Classifier Module (`classifier/mod.rs`, 48 tests)** — heuristics-based question classifier, no LLM:

- **`classifier::classify()`** — pure function that receives text and returns `QuestionType` (Technical, Star, Architecture, Trap, None).
- **`classifier::classify_text`** — Tauri command wrapper exposing classification to the frontend.
- **Detection:** question mark (`?`) OR imperative verb triggers (bilingual EN/ES), exclusion list for small talk, 4 keyword lists (40-80 terms each) for type classification, tie-breaking (Trap > Architecture > Star > Technical), experience question heuristic, and zero-score fallback.
- **Integration:** Called from `STTPipeline::flush_segment()` — each transcribed line is classified, and if not `None`, a `question-detected` event is emitted AND a `HintJob` is pushed to the orchestrator worker via `HintJobSender`. Also registered as a standalone Tauri command for direct frontend use.

### 2.7 Orchestrator Module (`orchestrator/`)

**Orchestrator module (`orchestrator/mod.rs` + `orchestrator/worker.rs`, ~400 non-test lines, 73 tests)** — binds classifier + RAG + hint emission into a cohesive hint engine:

- **`orchestrator::HintJob`** — data struct carrying `session_id`, `text` (the transcribed question), `qtype` (classified question type), and `mode` (`"practice"` or `"shadow"`).
- **`orchestrator::HintCommand`** — enum for the hint worker's message protocol: `Process(HintJob)` and `CancelSession(String)`.
- **`orchestrator::HintJobSender`** — type alias `Arc<Sender<HintCommand>>`, shared sender handle injected as Tauri state.
- **`orchestrator::HintScheduler`** — manages pending hints for Shadow mode. Hints are scheduled with a 2.5s delay (`SHADOW_DELAY_MS`); `tick(now)` returns expired hints. Supports `cancel_all(session_id)` to cancel pending hints when a session ends.
- **`orchestrator::generate_and_emit_hint()`** — the core hint generation function. Guards against `None`/empty text, runs `build_hint_text()` (RAG search top_k=1, tag+metric formatting, generic fallback), and either emits `new-hint` event immediately (Practice) or schedules via `HintScheduler` (Shadow).
- **`orchestrator::build_hint_text()`** — queries RAG via `search()`, if results have both `tag` and `metric` → formats as `"💡 {tag}: {metric}"` (max 8 words), otherwise truncates the chunk text to 8 words, falls back to generic hint per question type.
- **`orchestrator::generic_hint()`** — per-type fallback strings (e.g., `"💡 Usa STAR: Situación, Tarea, Acción, Resultado"` for Star).
- **`orchestrator::should_cancel_hint()`** — pure function that receives a `PendingHint` and optional `MicVadState`. Returns `true` if the user is currently speaking or has spoken since the hint was scheduled. Used to gate Shadow-mode hints.
- **`orchestrator::worker::start_hint_worker()`** — spawns the `kue-hint-worker` thread. Polls `HintCommand` channel with 500ms timeout. On `Process(job)`, calls `generate_and_emit_hint`. On timeout, calls `emit_expired_hints()` to drain the scheduler. On `CancelSession(sid)`, calls `scheduler.cancel_all()`.
- **`orchestrator::emit_expired_hints()`** — drains all expired hints from the scheduler. For each hint, checks `should_cancel_hint()` against `AudioCapture.mic_vad_state()`. If the user started speaking on Channel A since the hint was scheduled, the hint is silently dropped. Otherwise, emits it as `new-hint` Tauri event.

**Integration:** The STT pipeline's `flush_segment()` pushes `HintCommand::Process` into the mpsc channel when a question is detected. The pipeline's shutdown path sends `HintCommand::CancelSession` to cancel any pending shadow hints. The hint worker reads `MicVadState` from `AudioCapture` (injected via `app.try_state`) to implement mic-gated cancellation for Shadow mode.

### 2.8 Overlay Module (`overlay.rs`)

Small module (~88 lines, 8 tests) that controls the overlay window:

- **`overlay::show_overlay()`** — Tauri command `fn show_overlay(show: bool, app_handle: AppHandle) -> Result<(), String>`. If `show=true`, shows the overlay window and sets focus; if `show=false`, hides it. Returns an error if the `"overlay"` webview window doesn't exist (misconfigured `tauri.conf.json`).
- **`overlay::ERR_OVERLAY_WINDOW_NOT_FOUND`** — Public error constant for test assertions.
- **Window lifecycle:** The overlay window is defined in `tauri.conf.json` with `"visible": false` — it is created at app start but hidden. Click-through is enabled in `lib.rs::setup()` via `set_ignore_cursor_events(true)`.
- **Frontend counterpart:** `src/Overlay.tsx` React component listens for `new-hint` Tauri events, displays the hint text in a semi-transparent backdrop-blur container, and auto-dismisses after 3 seconds via `setTimeout`.

### 2.6 Configuration

- **`tauri.conf.json`** — Tauri v2, two windows: `main` (800×600, debug RAG UI) and `overlay` (400×100, transparent, always-on-top, click-through, skip-taskbar, hidden by default). DMG bundle (macOS only), dev URL on port 1420.
- **`vite.config.ts`** — Vite 6 with React plugin, HMR on port 1421, ignores changes in `src-tauri/`.
- **`capabilities/default.json`** — Main window permissions: `core:default` + `shell:allow-open`.
- **`capabilities/overlay.json`** — Overlay window permissions: `core:default` (minimal — no shell access).

---

## 3. Database schema

```sql
-- Interview sessions
CREATE TABLE sessions (
    id TEXT PRIMARY KEY DEFAULT (hex(randomblob(16))),
    started_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    ended_at DATETIME,
    company TEXT,
    role TEXT,
    mode TEXT CHECK(mode IN ('practice', 'shadow'))
);

-- Transcription lines per session
CREATE TABLE transcript_lines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    speaker TEXT CHECK(speaker IN ('user', 'interviewer')),
    text TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER NOT NULL
);

-- Documents uploaded by the user (CV, projects)
CREATE TABLE documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    filename TEXT NOT NULL,
    type TEXT NOT NULL,
    added_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Document chunks with metadata (tag + metric)
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES documents(id),
    text TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    tag TEXT,           -- e.g. 'nestjs', 'redis', 'star'
    metric TEXT         -- e.g. '10k req/seg', '40% reducción'
);

-- Vector index (sqlite-vec)
CREATE VIRTUAL TABLE chunks_vec USING vec0(embedding float[384]);

-- Key-value table for settings (does NOT include API keys)
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

**Implementation details:**

- WAL mode + foreign keys ON + busy timeout 5000ms.
- `chunks_vec` uses 384-dimensional embedding (`snowflake-arctic-embed-s`).
- The `sqlite-vec` extension is registered via `sqlite3_auto_extension` before opening any connection.
- No ORM — direct SQL queries via `rusqlite`.

---

## 4. Design data

### 4.1 Concurrent connections

`Database.conn` is `Mutex<Connection>` — a single thread accesses SQLite at a time. The `database_mutex_allows_concurrent_locks` test verifies that two threads can lock sequentially and see the same state.

### 4.2 Error handling

- `open_and_migrate` returns `Result<Database, Box<dyn std::error::Error>>`.
- `get_db_status_inner` returns `Result<DbStatus, String>` (flat errors for Tauri IPC).
- The poisoned mutex is handled as a catchable error (test `get_db_status_handles_poisoned_mutex`).

### 4.3 Idempotency

All DDL uses `IF NOT EXISTS`. The `open_and_migrate_is_idempotent` test runs the migration twice and verifies that the tables don't change.

---

## 5. Planned vs implemented

| Component                | Status           | Dependency in Cargo.toml                                                             | Code                                |
| ----------------------- | ---------------- | ------------------------------------------------------------------------------------- | ----------------------------------- |
| DB schema + migrations  | **Implemented**  | `rusqlite`, `sqlite-vec`                                                              | `db/mod.rs`                         |
| sqlite-vec register     | **Implemented**  | `sqlite-vec`                                                                          | `db/mod.rs`                         |
| Tauri setup + commands  | **Implemented**  | `tauri 2`                                                                             | `lib.rs`                            |
| Frontend (debug RAG UI) | **Implemented**  | React 18                                                                              | `App.tsx`                           |
| Mic capture (cpal)      | **Implemented**  | `cpal` (active)                                                                       | `audio/capture.rs`                  |
| Loopback capture (SCK)  | **Implemented**  | `screencapturekit` (active)                                                           | `audio/capture.rs`                  |
| WAV writing (hound)     | **Implemented**  | `hound` (active)                                                                      | `audio/capture.rs`                  |
| RAG embeddings + indexer | **Implemented**  | `candle-core`, `candle-nn`, `candle-transformers`, `hf-hub`, `tokenizers`, `bytemuck` | `rag/embeddings.rs`, `rag/indexer.rs` |
| STT (Moonshine)         | **Implemented** (integrated in lifecycle via `toggle_audio_capture`) | `libloading` (dynamically loaded `libmoonshine.dylib`), `uuid`                       | `stt/mod.rs`, `stt/ffi.rs`, `stt/cli.rs`, `stt/vad.rs`, `stt/pipeline.rs` |
| Question classifier     | **Implemented** (integrated in STT pipeline, standalone Tauri cmd) | —                                                                                     | `classifier/mod.rs`                  |
| Hint generator          | **Implemented** (orchestrator module: HintScheduler + hint worker + mpsc dispatch) | —                                                                                     | `orchestrator/mod.rs`, `orchestrator/worker.rs` |
| Overlay window + hint display | **Partially implemented** (window config, click-through, Overlay.tsx component, 3s auto-dismiss, show/hide command) | `tauri.conf.json` window config     | `overlay.rs`, `Overlay.tsx`         |
| Mic VAD (Shadow gating)       | **Implemented** | —                                                                                     | `audio/mic_vad.rs`                   |
| Post-call BYOK          | **Not started**  | `tauri-plugin-shell` present                                                          | —                                    |

---

## 6. Design patterns

- **Command pattern (Tauri):** `#[tauri::command]` as entry point for functionality (`get_db_status`, `toggle_audio_capture`, `index_folder_cmd`, `search_context`).
- **State management via Tauri:** `app.manage()` injects dependencies accessible by state (Database, AudioCapture, Mutex<EmbeddingModel>).
- **Inner function pattern:** Inner functions (e.g. `get_db_status_inner`, `ingest_documents`, `search`) separated from Tauri wrappers for testability.
- **Trait pattern (Embedder):** `Embedder` trait abstraction with `EmbeddingModel` (real) and `MockEmbeddingModel`/`TestEmbedder` (tests) implementations, allowing the indexer to be tested without GPU.
- **Mutex-guarded model:** `EmbeddingModel` wrapped in `std::sync::Mutex` for thread-safe access from multiple Tauri commands; the `Embedder` trait is also implemented for `Mutex<EmbeddingModel>`.
- **Once pattern:** `std::sync::Once` to register sqlite-vec only once in tests.
- **Temporary directory isolation:** Each test uses its own temporary directory (`TempDir` struct, with atomic counter).
- **Transactional ingestion:** Each file is processed within a SQLite transaction (`BEGIN...COMMIT`) for atomicity; if the embedding fails, the entire file is rolled back.
- **Session-scoped temp dirs:** Capture WAVs are written to `temp_dir()/kue-session-{timestamp}/`, streamed by Moonshine and deleted at session end. The `settings` table with `retain_audio` (opt-in, default `false`) controls persistence. `cleanup_orphaned_temp_dirs()` in `lib.rs` cleans orphan directories from crashed sessions.
- **Safe Send wrappers:** `MicHandle` and `LoopbackHandle` manually implement `Send` for `cpal::Stream` and `SCStream`, justified with safety comments by ownership invariant.
- **FFI auto-detection pattern:** `MoonshineFFIEngine::is_available()` checks for `libmoonshine.dylib` at runtime; if not available, falls back to `MoonshineCLIEngine` without user intervention.
- **Pipeline-integrated classification:** The classifier is not a separate service — it's called inline from `STTPipeline::flush_segment()` after each transcription, emitting a `question-detected` event if the text is a recognized question type.
- **Pure function with Tauri wrapper (classifier):** `classify()` is a pure `fn(&str) -> QuestionType` (testable without Tauri), while `classify_text()` is a thin `#[tauri::command]` wrapper exposing it over IPC.
- **Worker thread + channel (orchestrator):** The hint engine runs in a dedicated `kue-hint-worker` thread that receives `HintCommand` messages over an `mpsc` channel from the STT pipeline. Decouples the latency-sensitive audio path from the hint generation path (which hits RAG).
- **Scheduler pattern (HintScheduler):** Shadow mode hints are stored in a `Vec<PendingHint>` with an `Instant` deadline. The worker polls via `tick(now)` every 500ms. This avoids timers/threads per hint — a single poll loop processes all expired hints at once.
- **Cancel on session end:** The pipeline sends `HintCommand::CancelSession` when its thread exits, and the scheduler immediately removes all pending hints for that session. This prevents stale hints from appearing after the interview has ended.
- **Arc<Mutex> shared model:** The `EmbeddingModel` is wrapped in `Arc<Mutex<...>>` and cloned into the hint worker, allowing both Tauri commands (IPC) and the worker thread to generate embeddings without contention on a single state slot.
- **Multi-window architecture (Tauri):** Two separate webview windows (`main` and `overlay`) share the same Rust backend. The `App.tsx` component detects which window it's running in via `getCurrentWebviewWindow().label` and renders the appropriate UI (debug UI vs. overlay). The overlay window has its own capability file with minimal permissions.
- **Mic VAD monitor pattern (MicVadState):** A lightweight VAD wrapper runs alongside the mic capture pipeline. It doesn't control anything directly — it simply tracks speech transition timestamps. The orchestrator reads this state passively when deciding whether to emit a shadow hint. This separates the concerns of "detect speech" (mic_vad) from "decide whether to cancel" (orchestrator) without coupling the audio path to the hint path.
- **Microphone-gated hint delivery:** In Shadow mode, hints are not emitted unconditionally after the 2.5s delay — they are gated on the user's current speech state. `emit_expired_hints()` retrieves `MicVadState` from `AudioCapture` and calls `should_cancel_hint()` before each emission. This prevents the app from showing a hint when the user is already answering, making Shadow mode feel non-intrusive.

---

## 7. External dependencies

| Crate                  | Purpose                                           | Version |
| ---------------------- | ------------------------------------------------- | ------- |
| `tauri`                | Native application shell                          | 2       |
| `tauri-plugin-shell`   | External process invocation (BYOK)                | 2       |
| `rusqlite`             | SQLite client with bundled                        | 0.33    |
| `sqlite-vec`           | Vector index within SQLite                        | 0.1.9   |
| `cpal`                 | Microphone audio capture                          | 0.15    |
| `screencapturekit-rs`  | System loopback capture                           | git     |
| `hound`                | WAV encoding/decoding                             | 3.5     |
| `candle-core`          | ML framework for embeddings                       | 0.8     |
| `candle-nn`            | Neural network primitives                         | 0.8     |
| `candle-transformers`  | Transformer models (BERT)                         | 0.8     |
| `hf-hub`               | Model download from HuggingFace                   | 0.4     |
| `tokenizers`           | BERT tokenizer for embeddings                     | 0.21    |
| `bytemuck`             | Safe byte casting for sqlite-vec vectors          | 1       |
| `serde` / `serde_json` | IPC serialization                                 | 1       |
| `libloading`           | Dynamic loading of libmoonshine.dylib (STT FFI)    | 0.8     |
| `uuid`                 | Session IDs and unique temp file names            | 1       |
| `anyhow` / `thiserror` | Idiomatic error handling                          | 1 / 2   |
