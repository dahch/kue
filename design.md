# Kue — Technical Design

> Current status: **Sprint 0–6 completed** (base infrastructure + dual audio capture + STT + RAG engine + classifier + orchestrator + overlay window + hint display + mic VAD gating + Panic button + mode selection UI + post-call BYOK analysis + Moonshine auto-provisioning). All v1 features implemented and tested. This document describes the living architecture of the project.

---

## 1. General architecture

```mermaid
graph TD
    subgraph "Frontend (React + TypeScript)"
        A[MainApp<br/>session control UI<br/>mode selector + panic button]
        OV[Overlay<br/>hint display component<br/>3s auto-dismiss<br/>auto-show/hide on session events]
    end

    subgraph "Tauri Bridge (IPC)"
        B1[tauri::command<br/>get_db_status]
        B2[tauri::command<br/>start_session]
        B2B[tauri::command<br/>stop_session]
        B2C[tauri::command<br/>panic_mode]
        B3[tauri::command<br/>index_folder_cmd]
        B4[tauri::command<br/>search_context]
        B5[tauri::command<br/>overlay::show_overlay]
        B6[tauri::command<br/>db::get_sessions]
        B7[tauri::command<br/>db::get_session_transcript]
        B8[tauri::command<br/>audio::capture::is_transcript_ready]
        B9[tauri::command<br/>keys::save_key]
        B10[tauri::command<br/>keys::has_key]
        B11[tauri::command<br/>analyze::analyze_session]
    end

    subgraph "Rust Backend (lib.rs)"
        C[db::init_db]
        D[db::register_vec_extension]
        E[setup handler<br/>DB + AudioCapture + Model + overlay config + PanicState]
        F[audio::capture<br/>AudioCapture]
        EM[rag::embeddings<br/>EmbeddingModel (Mutex)]
        CL[cleanup_orphaned_temp_dirs]
        PS[orchestrator::PanicState<br/>10s hint silence]
        BT[BatchTracker<br/>tracks completed batch<br/>transcriptions per session]
    end

    subgraph "Post-call Analysis (implemented)"
        AN[analyze::analyze_session<br/>sends transcript + RAG context<br/>to user-configured LLM]
        KEYS[keys::save_key<br/>keys::has_key<br/>keyring (OS Keychain)]
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
        PS2[orchestrator::PanicState<br/>silences hints for 10s]
    end

    subgraph "Tauri Events"
        EV1[new-transcript]
        EV2[question-detected]
        EV3[new-hint]
        EV4[panic-mode]
        EV5[session-started]
        EV6[session-stopped]
        EV7[post-call-transcript-ready]
    end

    A -->|invoke| B1
    A -->|invoke| B2
    A -->|invoke| B2B
    A -->|invoke| B2C
    A -->|invoke| B3
    A -->|invoke| B4
    A -->|invoke| B6
    A -->|invoke| B7
    A -->|invoke| B8
    A -->|invoke| B9
    A -->|invoke| B10
    A -->|invoke| B11
    B1 --> C
    B2 --> F
    B2B --> F
    B2C --> PS
    B3 --> T
    B4 --> T
    B6 --> G
    B7 --> G
    B8 --> BT
    B9 --> KEYS
    B10 --> KEYS
    B11 --> AN
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
    OV -->|listens| EV4
    OV -->|listens| EV5
    OV -->|listens| EV6
    Q4 --> EV1
    Q4 --> EV2
    E --> PS
    PS2 <-->|hint_silenced_by_panic| O3
    PS -->|panic-mode event| EV4
    F -->|session-started| EV5
    F -->|session-stopped| EV6
    F -->|post-call-transcript-ready| EV7
    AN --> G
    AN --> T
    BT -->|tracks completion| EV7
    A -->|listens| EV7
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

**Legend:** Solid line = implemented. The STT pipeline integrates classification (VAD → STT → classify → events → DB). When a question is detected, the pipeline pushes a `HintJob` to the orchestrator worker thread via an mpsc channel. The worker queries RAG, builds a hint, and either emits it immediately (Practice) or schedules it via `HintScheduler` (Shadow, 2.5s delay). Expired hints are drained every 500ms in the worker's poll loop. In Shadow mode, before emitting each expired hint, the worker checks `MicVadState` (Channel A VAD) — if the user started speaking since the hint was scheduled, the hint is silently cancelled. `PanicState` (registered in Tauri state) can silence all hints for 10s; the worker checks `hint_silenced_by_panic()` before processing any hint job or emitting expired hints. At session end, Channel A (mic) audio is sent to a batch transcription thread (`kue-batch-transcribe`) via `spawn_batch_transcription()` which runs offline VAD+STT and persists user responses with `speaker='user'`. The `BatchTracker` tracks which sessions have completed batch transcription; the `is_transcript_ready` command and `post-call-transcript-ready` event communicate status to the frontend. Once batch transcription completes, the post-call analysis module (`analyze.rs`) can send the full transcript + RAG context to a user-configured LLM (Anthropic/OpenAI/Gemini/OpenRouter/Ollama) via the `analyze_session` command. API keys are stored in the OS keychain via `keys.rs` (`keyring` crate), never in the `settings` table. The `Overlay` React component listens for `new-hint` Tauri events, auto-shows in Shadow mode on `session-started`, auto-hides on `session-stopped`, and shows a panic indicator on `panic-mode`. The `MainApp` component is a full session control UI with mode selector, start/stop buttons, panic button, session history, and a post-call analysis panel with provider selection and result display.

---

## 2. Layer breakdown

### 2.1 Frontend (`src/`)

- **`main.tsx`** — Entry point React 18, mounts `<App />` on `#root`.
- **`App.tsx`** — App router that detects the window label via `getCurrentWebviewWindow()`. If the label is `"overlay"`, renders the `<Overlay />` component; otherwise renders `<MainApp />` (session control UI).
- **`MainApp`** (inside `App.tsx`) — Full session control UI with:
  - Mode selector (Practice/Shadow toggle)
  - Start/Stop session buttons connected via `invoke("start_session")` / `invoke("stop_session")`
  - Panic button connected via `invoke("panic_mode")`, displays a 10s mute indicator
  - Transcript and hint log display
  - Session history list with selection
  - `PostCallPanel` for post-call BYOK analysis (provider/model selection, API key input, analyze button, result display with summary/weak_questions/forgotten_projects/star_improvements)
  - Listens for `new-transcript`, `new-hint`, `panic-mode`, and `post-call-transcript-ready` events
- **`Overlay.tsx`** — Hint display component rendered inside the overlay window. Listens for:
  - `new-hint` — shows hint with 3s auto-dismiss via `setTimeout`
  - `session-started` — auto-shows overlay window in Shadow mode
  - `session-stopped` — auto-hides overlay window and clears state
  - `panic-mode` — shows a panic indicator (🔇) for the duration
  - Hint text displayed in a semi-transparent backdrop-blur container
- **`index.css`** — Tailwind directives (`@tailwind base/components/utilities`).

### 2.2 Tauri Shell (`lib.rs`)

File `src-tauri/src/lib.rs` (~113 lines):

```rust
mod analyze;         // Post-call BYOK analysis (Sprint 5)
mod audio;
mod classifier;
mod db;
mod keys;            // Keychain API key storage
mod orchestrator;    // Hint engine + PanicState — wired into lifecycle
mod overlay;         // Overlay window show/hide command
mod rag;
mod stt;             // STT module (Moonshine) — includes provisioning (Sprint 6)
mod types;

/// Shared state tracking which sessions have completed Channel A batch
/// transcription. The batch thread writes to this set when done; the
/// `is_transcript_ready` command and `analyze_session` read from it.
#[derive(Clone)]
struct BatchTracker(Arc<Mutex<HashSet<String>>>);

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

            // Register panic state
            app.manage(orchestrator::PanicState::new());

            // Register batch transcription tracker
            let batch_tracker = BatchTracker(Arc::new(Mutex::new(HashSet::new())));
            app.manage(batch_tracker);

            // Prepend the managed moonshine lib dir to DYLD_LIBRARY_PATH so
            // that @rpath/libonnxruntime.*.dylib is found alongside
            // libmoonshine.dylib when loaded by the FFI engine.
            if let Ok(app_data) = app.path().app_data_dir() {
                let managed_lib = app_data.join("moonshine").join("lib");
                let current = std::env::var("DYLD_LIBRARY_PATH").unwrap_or_default();
                let new = if current.is_empty() {
                    managed_lib.to_string_lossy().to_string()
                } else {
                    format!("{}:{}", managed_lib.to_string_lossy(), current)
                };
                std::env::set_var("DYLD_LIBRARY_PATH", &new);
            }

            // Spawn background Moonshine provisioning (dylibs + model download
            // on first launch, no-op otherwise). Progress via events.
            stt::provisioning::ensure_moonshine_installed(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            db::get_db_status,
            db::get_sessions,
            db::get_session_transcript,
            audio::capture::start_session,
            audio::capture::stop_session,
            audio::capture::panic_mode,
            audio::capture::is_transcript_ready,
            rag::indexer::index_folder_cmd,
            rag::indexer::search_context,
            classifier::classify_text,
            overlay::show_overlay,
            keys::save_key,
            keys::has_key,
            analyze::analyze_session,
            stt::provisioning::retry_moonshine_download,
        ])
        .run(tauri::generate_context!())
}
```

- **`mod analyze`**, **`mod audio`**, **`mod classifier`**, **`mod db`**, **`mod keys`**, **`mod orchestrator`**, **`mod overlay`**, **`mod rag`**, **`mod stt`**, **`mod types`** — Backend submodules. `stt` includes `provisioning` (Sprint 6) for first-launch Moonshine auto-download.
- **`cleanup_orphaned_temp_dirs()`** — Removes temporary `kue-session-*` directories left by crashed sessions.
- **`plugin(tauri_plugin_shell)`** — Standard Tauri v2 plugin for shell operations (not used for BYOK — that uses `reqwest`).
- **Overlay click-through:** In `setup()`, the overlay window is configured with `set_ignore_cursor_events(true)` so mouse events pass through to the video call underneath.
- **`app.manage(database)`**, **`app.manage(AudioCapture)`**, **`app.manage(Arc<Mutex<EmbeddingModel>>)`** — Injects `Database`, `AudioCapture` and the embeddings model (wrapped in `Arc` for sharing with the hint worker) as Tauri state.
- **`BatchTracker`** — `Arc<Mutex<HashSet<String>>>` registered as Tauri state, tracks which sessions have completed Channel A batch transcription. Checked by `is_transcript_ready` command and `analyze_session`.
- **Moonshine provisioning setup:** Prepends the managed moonshine lib dir to `DYLD_LIBRARY_PATH` (so `@rpath/libonnxruntime.*.dylib` is found at load time), then spawns a background `kue-moonshine-provision` thread via `ensure_moonshine_installed()` that downloads dylibs from PyPI and model files (~482 MB total) on first launch. Progress is reported via `moonshine-download-progress` events (with stage, file index/count, and downloaded/total bytes, throttled to 250ms); success emits `moonshine-provisioned`, failure emits `moonshine-provision-error`. The `retry_moonshine_download` Tauri command allows retry on failure.
- **Orchestrator setup:** `HintScheduler` (manages delayed hints in Shadow mode), `HintJobSender` (mpsc channel for dispatching hint jobs), and `start_hint_worker` (spawns `kue-hint-worker` thread that processes jobs and drains expired hints every 500ms).
- **`PanicState`** — Registered via `app.manage(PanicState::new())`. Stores an optional `Instant` deadline; when set, `hint_silenced_by_panic()` in the orchestrator returns `true` for all hint operations until the deadline expires (10s), effectively muting all hints.
- **`start_session`**, **`stop_session`**, **`panic_mode`** — Tauri commands replacing the old `toggle_audio_capture`. `start_session` creates a DB session, spawns the dual capture and STP pipeline. `stop_session` stops capture, triggers batch transcription for Channel A, and emits `session-stopped`. `panic_mode` activates `PanicState` for 10s and emits `panic-mode` event.
- **`index_folder_cmd`**, **`search_context`**, **`classify_text`**, and **`show_overlay`** — Tauri commands for RAG, classifier, and overlay window control.
- **`get_sessions`**, **`get_session_transcript`** — Tauri commands in `db/mod.rs` for retrieving session history and full transcripts from the frontend (used by the post-call panel).
- **`is_transcript_ready`** — Tauri command in `audio/capture.rs` that checks `BatchTracker` to see if Channel A batch transcription has completed for a given session.
- **`save_key`**, **`has_key`** — Tauri commands in `keys.rs` for storing/checking API keys in the OS keychain via the `keyring` crate.
- **`analyze_session`** — Tauri command in `analyze.rs` that sends the full session transcript + RAG context to a user-configured LLM (Anthropic/OpenAI/Gemini/OpenRouter/Ollama) and returns a structured analysis (summary, weak questions, forgotten projects, STAR improvements).
- **STT integration:** The STT pipeline is fully integrated into `start_session`. The command creates a DB session, spawns an `STTPipeline` thread that consumes audio from the loopback channel, performs VAD → STT → classify → events → persistence → hint job dispatch. The `new-transcript` and `question-detected` events are emitted to the frontend; question-detected lines also push a `HintJob` to the orchestrator worker.

### 2.3 Database Module (`db/mod.rs`)

The substantial module of the app (~1185 lines, 34 tests). See §3 for schema details and §4 for tests.

### 2.4 Audio Module (`audio/`)

#### `audio/capture.rs`

Substantial module (~1470+ lines, 51 tests) that implements dual audio capture:

- **Microphone (Channel A):** via `cpal`, supports i16 and f32 sample formats with automatic conversion to i16.
- **Loopback (Channel B):** via `screencapturekit-rs` (ScreenCaptureKit), captures the system output audio (interviewer's voice). `excludes_current_process_audio: true` to avoid echo.
- **WAV writer:** two background threads (`kue-wav-mic-A`, `kue-wav-loopback-B`) that write the channels to separate WAV files (16 kHz, mono, 16-bit) using `hound`.
- **`AudioCapture` struct:** managed as Tauri state via `app.manage()`. Exposes `start(mode)`, `stop()`, `toggle(start, mode)`. Exposes `mic_vad_state()` returning `Arc<Mutex<MicVadState>>` for the orchestrator's shadow-mode hint cancellation.
- **`start_session` / `stop_session` / `panic_mode` commands:** Three separate Tauri commands replacing the old `toggle_audio_capture`. `start_session(mode)` starts both captures and creates a DB session. `stop_session()` stops both captures and triggers batch transcription. `panic_mode()` activates `PanicState` (10s hint silence) and emits `panic-mode` event.
- **51 tests:** cover f32→i16 conversion (edge cases: NaN, infinity, clamping, very small values), state serialization (4 combinations), directory creation, path format, invalid mode (multiple variants), toggle (4 paths), WAV writer (creation, multiple buffers, invalid path, empty buffer, cyclic lifecycle), consistency between mode validation and DB, batch transcription spawning, and `is_transcript_ready`.

#### `audio/mic_vad.rs`

Moderate module (~365 lines, 18 tests) that wraps `SimpleVAD` for Channel A (mic):

- **`MicVadState`** — Thread-safe struct wrapping an energy-based VAD (`SimpleVAD` from the STT module). Tracks silence→speech transition timestamps on the mic channel.
- **`feed_audio(samples)`** — Feeds i16 mic samples and updates the VAD state. Must be called from the mic capture thread.
- **`has_speech_since(instant)`** — Returns `true` if a silence→speech transition occurred after the given `Instant`. Used by the orchestrator to determine if the user started answering before a Shadow-mode hint's delay expired.
- **`is_currently_speaking()`** — Returns `true` if VAD currently considers the user to be speaking.
- **`reset()`** — Clears all state (called when mic capture restarts).

### 2.5 STT + Classifier Module (`stt/`, `classifier/`)

**STT Module (`stt/`, ~1400 non-test lines, ~153 tests)** — implements real-time transcription of Channel B and batch transcription of Channel A, integrated into the app lifecycle via `start_session`/`stop_session`:

- **`stt/mod.rs`** — `STTEngine` trait with `load()` and `transcribe_audio_chunk()`, `STTConfig`, blanket impl for `Box<T>`. Also exposes `persist_transcript_line()` and `create_engine()` at module level (extracted from `pipeline.rs` and `ffi.rs`).
- **`stt/ffi.rs`** — `MoonshineFFIEngine`: loads `libmoonshine.dylib` at runtime via `libloading`, declares FFI bindings with `#[repr(C)]` for `CTranscript`, `CTranscriptLine`, etc. Calls the Moonshine streaming API: `create_stream` → `start_stream` → `add_audio_to_stream` → `transcribe_stream`. Frees resources in `Drop`.
- **`stt/cli.rs`** — `MoonshineCLIEngine` (fallback): writes WAV segments to temp with UUID, invokes `moonshine-voice transcribe --wav-path <file>` as a subprocess, parses the last line of output.
- **`stt/vad.rs`** — `SimpleVAD`: voice activity detection by RMS energy with configurable threshold, minimum speech duration, and silence timeout.
- **`stt/pipeline.rs`** — `STTPipeline`: orchestrator that receives audio from loopback via `mpsc::Receiver`, runs VAD + segment buffer + STT engine + calls `classifier::classify()` on each transcribed line + emits `new-transcript` and `question-detected` events + persists in `transcript_lines`.
- **`stt/batch.rs`** — `transcribe_channel_batch()`: offline batch transcription of a full-channel WAV file. Used post-session to transcribe Channel A (mic, user voice) via Moonshine + SimpleVAD chunking. Reads the entire WAV into memory, segments by VAD, transcribes each segment, persists with `speaker='user'` in `transcript_lines`. Runs in a dedicated `kue-batch-transcribe` thread spawned from `stop_session`'s stop path. Emits `post-call-transcript-ready` event on completion. Handles empty/corrupt WAVs, whitespace-only results, uneven sample lengths, and non-standard sample rates. Includes 27 tests covering edge cases (empty, corrupt, all-silence, short speech, multiple segments, partial chunks, different sample rates, DB timestamp/session-id verification).

**Lifecycle integration:** When `start_session` is called, the command creates a DB session (in `sessions` table), spawns an `STTPipeline` thread via `spawn_processing_thread()`, and connects it to the loopback audio stream. The pipeline runs until `stop()` is called, at which point the session is finalized, any pending shadow hints are cancelled via `HintCommand::CancelSession`, and temp WAV files are cleaned up (or retained if `retain_audio` is enabled). On stop, Channel A (mic WAV) is sent to a batch transcription thread (`kue-batch-transcribe`) via `spawn_batch_transcription()`, which transcribes the entire Channel A recording offline using `stt::batch::transcribe_channel_batch()` and persists user responses with `speaker='user'`. The `post-call-transcript-ready` event is emitted on completion.

**Auto-detection:** At runtime, tries FFI first (`libmoonshine.dylib` in `MOONSHINE_LIB_DIR` or standard paths), falls back to CLI if the library is not found. No Whisper — Moonshine is the only option.

**~153 tests:** cover `parse_transcript` (null ptr, 0 lines, completed/incomplete, empty text, preferred line), `rms` (8 cases), VAD (24 cases: silence, speech, timeout, reset, minimum duration, empty, threshold, boundary, accumulation, reset during speech, etc.), temp WAV writing (3 cases: data, empty, unique names), FFI (11 cases: transcript parsing, multi-line, C wraparound), CLI (15 cases: subprocess, parse, edge cases), `STTPipeline` (engine selection, load delegation, start/end session, process chunk, flush segment, DB persistence, poisoned mutex, special characters, multiple lines, hint job dispatch), and batch transcription (6 cases: empty/corrupt/all-silence WAVs, user/interviewer speaker, multi-segment, silent engine, whitespace-only text, mixed segments, partial chunks, different sample rates, DB timestamp/session-id verification, chunk_size, sample_offset_to_ms, Send trait, trailing segment edge cases).

**Classifier Module (`classifier/mod.rs`, 76 tests)** — heuristics-based question classifier, no LLM:

- **`classifier::classify()`** — pure function that receives text and returns `QuestionType` (Technical, Star, Architecture, Trap, None).
- **`classifier::classify_text`** — Tauri command wrapper exposing classification to the frontend.
- **Detection:** question mark (`?`) OR imperative verb triggers (bilingual EN/ES), exclusion list for small talk, 4 keyword lists (40-80 terms each) for type classification, tie-breaking (Trap > Architecture > Star > Technical), experience question heuristic, and zero-score fallback.
- **Integration:** Called from `STTPipeline::flush_segment()` — each transcribed line is classified, and if not `None`, a `question-detected` event is emitted AND a `HintJob` is pushed to the orchestrator worker via `HintJobSender`. Also registered as a standalone Tauri command for direct frontend use.

### 2.9 Post-call Analysis Module (`analyze.rs` + `keys.rs`)

**Post-call analysis (`analyze.rs`, ~800 lines, 32 tests)** — implements the `analyze_session` Tauri command for BYOK post-call analysis:

- **`analyze::analyze_session`** — Tauri command `fn analyze_session(session_id, provider, model, db, model_state, batch_tracker)`. Guards: rejects if batch transcription is not yet complete (checks `BatchTracker`).
- **`build_analysis_prompt`** — Builds a structured Spanish-language prompt that includes:
  - The full transcript (both speakers, labeled as "Candidato" and "Entrevistador")
  - RAG context from the user's documents (queried via `search()`)
  - A strict JSON schema requirement for the response
- **Provider-specific URLs and headers:** Maps provider names (`"anthropic"`, `"openai"`, `"gemini"`, `"openrouter"`, `"ollama"`) to their API endpoints and header formats.
- **Response parsing:** Attempts to extract valid JSON from the LLM response (handles markdown code block wrapping), then deserializes into `AnalyzeResult` with four fields: `summary`, `weak_questions`, `forgotten_projects`, `star_improvements`.
- **Error handling:** Returns structured `String` errors for failed HTTP requests, JSON parsing failures, or missing API keys.

**Keychain storage (`keys.rs`, ~110 lines, 6 ignored tests)** — manages API key storage in the OS native keychain:

- **`keys::save_api_key(provider, key)`** — Stores a key for a given provider in the OS keychain via the `keyring` crate.
- **`keys::get_api_key(provider)`** — Retrieves a key from the keychain; returns `Err` if no key exists.
- **`keys::delete_api_key(provider)`** — Deletes a key (idempotent — succeeds even if the key doesn't exist).
- **`save_key` / `has_key`** — Tauri command wrappers for the frontend.

**Integration:** The `App.tsx` `PostCallPanel` component:
1. Checks `is_transcript_ready` on mount to see if batch transcription is complete
2. Listens for `post-call-transcript-ready` event to update state when transcription finishes
3. Shows `ApiKeyInput` for the selected provider (checks `has_key` on mount)
4. On "Analyze" click, invokes `analyze_session` with session ID, provider, and optional model name
5. Displays the returned `AnalyzeResult` in categorized sections (summary, weak questions, forgotten projects, STAR improvements)

### 2.6 Configuration

- **`tauri.conf.json`** — Tauri v2, two windows: `main` (800×600, session control UI) and `overlay` (400×100, transparent, always-on-top, click-through, skip-taskbar, hidden by default). DMG bundle (macOS only), dev URL on port 1420.
- **`vite.config.ts`** — Vite 6 with React plugin, HMR on port 1421, ignores changes in `src-tauri/`.
- **`capabilities/default.json`** — Main window permissions: `core:default` + `shell:allow-open`.
- **`capabilities/overlay.json`** — Overlay window permissions: `core:default` (minimal — no shell access).

### 2.7 Orchestrator Module (`orchestrator/`)

**Orchestrator module (`orchestrator/mod.rs` + `orchestrator/worker.rs`, >1400 lines including tests, 73 tests)** — binds classifier + RAG + hint emission into a cohesive hint engine:

- **`orchestrator::HintJob`** — data struct carrying `session_id`, `text` (the transcribed question), `qtype` (classified question type), and `mode` (`"practice"` or `"shadow"`).
- **`orchestrator::HintCommand`** — enum for the hint worker's message protocol: `Process(HintJob)` and `CancelSession(String)`.
- **`orchestrator::HintJobSender`** — type alias `Arc<Sender<HintCommand>>`, shared sender handle injected as Tauri state.
- **`orchestrator::HintScheduler`** — manages pending hints for Shadow mode. Hints are scheduled with a 2.5s delay (`SHADOW_DELAY_MS`); `tick(now)` returns expired hints. Supports `cancel_all(session_id)` to cancel pending hints when a session ends.
- **`orchestrator::generate_and_emit_hint()`** — the core hint generation function. Guards against `None`/empty text, runs `build_hint_text()` (RAG search top_k=1, tag+metric formatting, generic fallback), and either emits `new-hint` event immediately (Practice) or schedules via `HintScheduler` (Shadow).
- **`orchestrator::build_hint_text()`** — queries RAG via `search()`, if results have both `tag` and `metric` → formats as `"💡 {tag}: {metric}"` (max 8 words), otherwise truncates the chunk text to 8 words, falls back to generic hint per question type.
- **`orchestrator::generic_hint()`** — per-type fallback strings (e.g., `"💡 Usa STAR: Situación, Tarea, Acción, Resultado"` for Star).
- **`orchestrator::PanicState`** — shared state struct wrapping `Arc<Mutex<Option<Instant>>>`. When set to a future `Instant`, `is_panicking()` returns `true` until the deadline passes. Used by `hint_silenced_by_panic()` to gate all hint operations during the 10s mute window. Registered as Tauri state in `lib.rs::setup()`.
- **`orchestrator::hint_silenced_by_panic()`** — checks `PanicState` from Tauri state; returns `true` if panic mode is active. Called at the top of both `generate_and_emit_hint()` and `emit_expired_hints()` to silently drop hints during mute.
- **`orchestrator::should_cancel_hint()`** — pure function that receives a `PendingHint` and optional `MicVadState`. Returns `true` if the user is currently speaking or has spoken since the hint was scheduled. Used to gate Shadow-mode hints.
- **`orchestrator::worker::start_hint_worker()`** — spawns the `kue-hint-worker` thread. Polls `HintCommand` channel with 500ms timeout. On `Process(job)`, calls `generate_and_emit_hint` (which first checks panic state). On timeout, calls `emit_expired_hints()` (which also checks panic state). On `CancelSession(sid)`, calls `scheduler.cancel_all()`.
- **`orchestrator::emit_expired_hints()`** — drains all expired hints from the scheduler. For each hint, checks `should_cancel_hint()` against `AudioCapture.mic_vad_state()`. If the user started speaking on Channel A since the hint was scheduled, the hint is silently dropped. Otherwise, emits it as `new-hint` Tauri event.

**Integration:** The STT pipeline's `flush_segment()` pushes `HintCommand::Process` into the mpsc channel when a question is detected. The pipeline's shutdown path sends `HintCommand::CancelSession` to cancel any pending shadow hints. The hint worker reads `MicVadState` from `AudioCapture` (injected via `app.try_state`) to implement mic-gated cancellation for Shadow mode.

### 2.8 Overlay Module (`overlay.rs`)

Small module (~88 lines, 6 tests) that controls the overlay window:

- **`overlay::show_overlay()`** — Tauri command `fn show_overlay(show: bool, app_handle: AppHandle) -> Result<(), String>`. If `show=true`, shows the overlay window and sets focus; if `show=false`, hides it. Returns an error if the `"overlay"` webview window doesn't exist (misconfigured `tauri.conf.json`).
- **`overlay::ERR_OVERLAY_WINDOW_NOT_FOUND`** — Public error constant for test assertions.
- **Window lifecycle:** The overlay window is defined in `tauri.conf.json` with `"visible": false` — it is created at app start but hidden. Click-through is enabled in `lib.rs::setup()` via `set_ignore_cursor_events(true)`.
- **Frontend counterpart:** `src/Overlay.tsx` (85 lines) React component listens for:
  - `new-hint` — displays the hint text in a semi-transparent backdrop-blur container, auto-dismisses after 3 seconds via `setTimeout`.
  - `session-started` — auto-shows the overlay window in Shadow mode (so the user sees the overlay appear when the session starts).
  - `session-stopped` — auto-hides the overlay window and clears hint state.
  - `panic-mode` — shows a panic indicator (🔇) inside the hint container for the 10s mute duration; changes container background to orange during panic.

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
| Frontend (session control UI + post-call panel) | **Implemented**  | React 18                                                                              | `App.tsx` (MainApp + PostCallPanel) |
| Mic capture (cpal)      | **Implemented**  | `cpal` (active)                                                                       | `audio/capture.rs`                  |
| Loopback capture (SCK)  | **Implemented**  | `screencapturekit` (active)                                                           | `audio/capture.rs`                  |
| WAV writing (hound)     | **Implemented**  | `hound` (active)                                                                      | `audio/capture.rs`                  |
| RAG embeddings + indexer | **Implemented**  | `candle-core`, `candle-nn`, `candle-transformers`, `hf-hub`, `tokenizers`, `bytemuck` | `rag/embeddings.rs`, `rag/indexer.rs` |
| STT (Moonshine)         | **Implemented** (integrated in lifecycle via `start_session`/`stop_session`) | `libloading` (dynamically loaded `libmoonshine.dylib`), `uuid`                       | `stt/mod.rs`, `stt/ffi.rs`, `stt/cli.rs`, `stt/vad.rs`, `stt/pipeline.rs` |
| Moonshine auto-provisioning | **Implemented** (Sprint 6 — first-launch download of dylibs + model, progress events, retry command) | `sha2`, `hex`, `zip`, `reqwest` (blocking)                                          | `stt/provisioning.rs`            |
| Channel A batch transcription | **Implemented** (ADR-015 — runs offline post-session in `kue-batch-transcribe` thread) | `hound`                                                                               | `stt/batch.rs`, `audio/capture.rs` (spawn_batch_transcription) |
| Question classifier     | **Implemented** (integrated in STT pipeline, standalone Tauri cmd) | —                                                                                     | `classifier/mod.rs`                  |
| Hint generator          | **Implemented** (orchestrator module: HintScheduler + hint worker + mpsc dispatch) | —                                                                                     | `orchestrator/mod.rs`, `orchestrator/worker.rs` |
| Overlay window + hint display | **Implemented** (window config, click-through, Overlay.tsx with 3s auto-dismiss, show/hide command, auto-show/hide on session events, panic indicator) | `tauri.conf.json` window config     | `overlay.rs`, `Overlay.tsx`         |
| Mic VAD (Shadow gating)       | **Implemented** | —                                                                                     | `audio/mic_vad.rs`                   |
| Panic button + PanicState    | **Implemented** (~10s mute via `panic_mode` command + `panic-mode` event) | `PanicState` in Tauri state                                | `orchestrator/mod.rs`, `audio/capture.rs`, `App.tsx`, `Overlay.tsx` |
| Mode selection UI             | **Implemented** (Practice/Shadow toggle in `App.tsx`) | —                                                                                     | `App.tsx`                           |
| Post-call BYOK          | **Implemented** (Sprint 5 — `analyze_session` command, `keys.rs` keychain storage, `analyze.rs` prompt builder + LLM API calls for Anthropic/OpenAI/Gemini/OpenRouter/Ollama) | `tauri-plugin-shell` present + `keyring` (keychain), `reqwest` (HTTP), `serde_json` (response parsing) | `analyze.rs`, `keys.rs`, `App.tsx` (PostCallPanel) |

---

## 6. Design patterns

- **Command pattern (Tauri):** `#[tauri::command]` as entry point for functionality (`get_db_status`, `start_session`, `stop_session`, `panic_mode`, `index_folder_cmd`, `search_context`). Session lifecycle is split into three separate commands (start/stop/panic) instead of a single `toggle_audio_capture`, giving the frontend explicit control over each phase.
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
- **Module-level helpers with shared types:** `persist_transcript_line()` and `create_engine()` are extracted to `stt/mod.rs` (module level, not inside any sub-struct) so they can be shared across `pipeline.rs` (streaming STT) and `batch.rs` (offline batch STT) without code duplication. `TranscriptLine` and `Speaker` live in `types.rs` as the shared contract between the STT, classifier, and orchestrator modules.
- **Worker thread + channel (orchestrator):** The hint engine runs in a dedicated `kue-hint-worker` thread that receives `HintCommand` messages over an `mpsc` channel from the STT pipeline. Decouples the latency-sensitive audio path from the hint generation path (which hits RAG).
- **Scheduler pattern (HintScheduler):** Shadow mode hints are stored in a `Vec<PendingHint>` with an `Instant` deadline. The worker polls via `tick(now)` every 500ms. This avoids timers/threads per hint — a single poll loop processes all expired hints at once.
- **Cancel on session end:** The pipeline sends `HintCommand::CancelSession` when its thread exits, and the scheduler immediately removes all pending hints for that session. This prevents stale hints from appearing after the interview has ended.
- **Arc<Mutex> shared model:** The `EmbeddingModel` is wrapped in `Arc<Mutex<...>>` and cloned into the hint worker, allowing both Tauri commands (IPC) and the worker thread to generate embeddings without contention on a single state slot.
- **Multi-window architecture (Tauri):** Two separate webview windows (`main` and `overlay`) share the same Rust backend. The `App.tsx` component detects which window it's running in via `getCurrentWebviewWindow().label` and renders the appropriate UI (debug UI vs. overlay). The overlay window has its own capability file with minimal permissions.
- **Mic VAD monitor pattern (MicVadState):** A lightweight VAD wrapper runs alongside the mic capture pipeline. It doesn't control anything directly — it simply tracks speech transition timestamps. The orchestrator reads this state passively when deciding whether to emit a shadow hint. This separates the concerns of "detect speech" (mic_vad) from "decide whether to cancel" (orchestrator) without coupling the audio path to the hint path.
- **Microphone-gated hint delivery:** In Shadow mode, hints are not emitted unconditionally after the 2.5s delay — they are gated on the user's current speech state. `emit_expired_hints()` retrieves `MicVadState` from `AudioCapture` and calls `should_cancel_hint()` before each emission. This prevents the app from showing a hint when the user is already answering, making Shadow mode feel non-intrusive.
- **Post-session batch transcription (ADR-015):** Channel A (mic, user voice) is not transcribed in real time — it's transcribed offline at session end via `stt::batch::transcribe_channel_batch()`. The entire WAV is read into memory, segmented by VAD, and transcribed segment-by-segment. This runs in a dedicated `kue-batch-transcribe` thread spawned from the `stop` path of `stop_session`, so the Tauri command returns immediately without blocking. The batch thread takes ownership of the session temp directory and applies audio retention policy (ADR-011) after transcription completes. Results are persisted with `speaker='user'`, and a `post-call-transcript-ready` event notifies the frontend.
- **Panic/Mute state pattern (PanicState):** A shared `Arc<Mutex<Option<Instant>>>` is registered as Tauri state. When the user presses the panic button, `panic_mode` command sets the Instant to `now + 10s`. The `hint_silenced_by_panic()` function checks this state at the top of both `generate_and_emit_hint()` and `emit_expired_hints()`, silently dropping all hints until the timer expires. This is a lightweight, timer-free mute mechanism — no threads, no async, just an Instant comparison before each hint operation.
- **BYOK analysis pattern (analyze.rs):** Post-call analysis is a standalone Tauri command (`analyze_session`) that:
  1. Reads the full transcript from `transcript_lines` via `get_transcript_lines()`
  2. Queries RAG for relevant context via `search()` on the session's transcript text
  3. Builds a structured prompt requesting JSON output (summary, weak_questions, forgotten_projects, star_improvements)
  4. Makes an HTTP request (`reqwest`) to the selected provider (Anthropic/OpenAI/Gemini/OpenRouter/Ollama), using a per-provider URL builder and header formatter
  5. Parses the JSON response into `AnalyzeResult`
  This runs in the Tauri command thread (not in the audio/orchestrator thread), so it doesn't affect real-time performance. The prompt is in Spanish and specifies a strict JSON schema.

- **Keychain storage pattern (keys.rs):** API keys are never stored in the `settings` SQLite table. Instead, the `keyring` crate stores them in the OS native keychain (macOS Keychain), identified by the service name `"kue"` and the provider name as the user identifier. The `save_key` and `has_key` Tauri commands expose this to the frontend. The `key_never_in_settings_table` test explicitly verifies that no kue-related file on disk contains the key value.

- **Batch transcription completion tracking (BatchTracker):** A `HashSet<String>` of session IDs wrapped in `Arc<Mutex<...>>` is registered as Tauri state (`BatchTracker`). When the `kue-batch-transcribe` thread finishes processing Channel A, it writes the session ID to this set. The `is_transcript_ready` Tauri command checks the set, and a `post-call-transcript-ready` event is emitted to notify the frontend. This decouples the batch thread completion from the `stop_session` command — the command returns immediately, and the frontend polls or listens for the completion event.

---

## 7. External dependencies

| Crate                  | Purpose                                           | Version |
| ---------------------- | ------------------------------------------------- | ------- |
| `tauri`                | Native application shell                          | 2       |
| `tauri-plugin-shell`   | Tauri v2 shell plugin (general)                   | 2       |
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
| `keyring`              | OS native keychain for API key storage (macOS)    | 2       |
| `sha2`                 | SHA-256 verification for provisioned files        | 0.10    |
| `hex`                  | Hex encoding for SHA-256 hashes                   | 0.4     |
| `zip`                  | ZIP extraction for Moonshine wheel dylibs         | 2       |
| `reqwest`              | HTTP client for post-call BYOK analysis           | 0.12    |
