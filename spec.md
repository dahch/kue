# Kue — spec.md (v1)

> Codename: **Kue**.

> **Implementation status:** Sprints 0–6 completed. All v1 features implemented and tested. Channel A batch transcription (ADR-015) implemented — user responses transcribed at session end. The DB schema is implemented (34 tests), the audio capture module (mic + loopback + mic_vad, ~1857 lines total / ~809 non-test, 70 tests), the RAG engine (embeddings + indexer, ~485 non-test lines, 65 tests), the STT module (7 files including batch and provisioning, ~3100 non-test lines, ~153 tests), the classifier module (1 file, 76 tests), the orchestrator module (2 files, 342 non-test lines, 73 tests), the overlay module (1 file, 6 tests), the mic_vad module (1 file, ~365 lines, 18 tests), the post-call analysis module (1 file, ~800 lines, 32 tests), the keys module (1 file, ~110 lines, 6 tests — includes `key_never_in_settings_table` invariant test), and the types module (1 file, 19 tests — `Speaker` enum + `TranscriptLine`, shared between STT and classifier). The STT module integrates Moonshine via FFI (`libmoonshine.dylib`) with CLI fallback (`moonshine-voice`), simple VAD, pipeline in its own thread, `new-transcript` events to the frontend and persistence in `transcript_lines`. Moonshine dylibs + model are auto-downloaded on first launch (Sprint 6 — `stt/provisioning.rs`, 896 lines, 32 tests): the Python wheel (v0.0.73, 51.9 MB) is fetched from PyPI and extracts `libmoonshine.dylib` + `libonnxruntime.1.23.2.dylib`, while the Medium Streaming English model (~429 MB, 8 ONNX files) is fetched from `download.moonshine.ai`; progress is reported via `moonshine-download-progress` events, with retry via the `retry_moonshine_download` Tauri command. All paths resolve to `{app_data_dir}/moonshine/` (managed), with dev paths (`~/.local/share/moonshine/`, `~/moonshine-models/`) as fallback. The STT pipeline is integrated into the app lifecycle via `start_session`/`stop_session` commands, which create DB sessions, spawn the pipeline thread, and persist transcripts. The classifier receives each transcribed line (called from `STTPipeline::flush_segment`) and emits a `question-detected` event when a question is recognized. When a question is detected, the pipeline pushes a `HintJob` to the orchestrator module, which runs classifier + RAG search → hint text → immediate emit (Practice) or delayed emit (Shadow) via a dedicated worker thread (`kue-hint-worker`). The delayed hints in Shadow mode are gated by `audio::mic_vad::MicVadState` — if the user starts speaking on Channel A (mic) before the 2.5s delay expires, the hint is silently cancelled. The overlay window (transparent, always-on-top, click-through, 400×100) displays hints via a React `Overlay` component that listens for `new-hint` Tauri events and auto-dismisses after 3s. At session end, Channel A (mic) audio is batch-transcribed and persisted with `speaker='user'`, producing a complete dual-speaker transcript for post-call analysis. `session-started` and `session-stopped` Tauri events are emitted at session boundaries (consumed by the overlay for auto-show/hide). The panic/mute feature silences hints for 10s via `PanicState` + `panic-mode` event. Post-call analysis (`analyze_session`) supports 5 BYOK providers with `BatchTracker` for async transcript readiness. Sections §3–§9 describe the complete planned product; see [`design.md`](./design.md) for what is actually built.

## 1. Objective

Desktop application (macOS-only in v1) that functions as a "memory copilot" for technical interviews and mock interviews. It doesn't answer for the user: it extracts information from their own context (CV, projects, metrics) and displays ultra-short hints (5-8 words) that help maintain fluency and structure under pressure. When finished, it saves the full transcript for on-demand post-call analysis.

## 2. Product overview

**Target market:** software engineers, data scientists, and technical professionals preparing for interviews, or wanting memory support during real hiring processes.

**Value proposition:**

- **No cheating:** doesn't generate answers; only recalls your own metrics, projects, and structure.
- **Total privacy:** STT, RAG, and classification run 100% locally. The only data that leaves the machine is the post-call transcript, and only if the user decides to send it to an external LLM (BYOK).
- **Low stress:** reduces performance anxiety by recalling key points at the right moment, not before or after.

## 3. Goals / non-goals (v1)

**Yes:**

- Real-time transcription of interviewer audio (Channel B) with speaker separation by audio channel; user responses (Channel A) are transcribed in batch at session end (ADR-015).
- **Practice** mode (mock interview, instructive and immediate hints) and **Shadow** mode (real interview, hints only if stuck >2.5s).
- Own context ingestion (PDF/TXT/MD) indexed locally via RAG.
- Full transcript saved per session.
- On-demand post-call analysis with BYOK.

**No (v1):**

- Windows and Linux — **macOS-only for now**, evaluated for v2 based on how well this v1 works.
- Full answers generated live — out of scope by design, not by deadline.
- Multi-device sync or cloud backend.
- Voice cloning / live TTS.

## 4. Main features

| Module        | Description                                                                                                       | Mode  |
| ------------- | ----------------------------------------------------------------------------------------------------------------- | ----- |
| **Practice**  | Mock interview with generous feedback; more instructive hints, the classifier explains the structure.             | Local |
| **Shadow**    | Real interview; sparse hints, only appear if the user gets stuck (delay > 2.5s after the question).               | Local |
| **Panic/Mute** | Button that silences all hints for 10s when the user feels overwhelmed or distracted; `PanicState` + `panic-mode` event. | Local |
| **Post-Call** | Button that analyzes the full transcript: summary, weak questions, forgotten projects, improvable STAR structure. | BYOK  |

## 5. Technology stack

| Layer               | Technology                                                         | Justification                                                                                                                                                                                                                        |
| ------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Frontend            | React + TypeScript                                                 | Dynamic UI, rapid prototyping.                                                                                                                                                                                                       |
| App Core            | Rust (Tauri)                                                       | Native audio access, transparent windows (overlay), single binary.                                                                                                                                                                   |
| Audio capture       | `cpal` (mic) + `screencapturekit-rs` (system loopback)             | ScreenCaptureKit (macOS 13+) captures system audio without virtual drivers — avoids depending on BlackHole. Requires user permission in Settings → Privacy → Screen & System Audio Recording (no signing entitlement can bypass it). |
| STT                 | Moonshine (Medium)                                                 | Local, streaming, <260ms latency, native diarization as fallback.                                                                                                                                                                    |
| Embeddings (RAG)    | candle (HuggingFace Rust)                                          | Native inference in Rust. Decided: `snowflake-arctic-embed-s` (384-d, same scheme as MiniLM, better performance on MTEB/BEIR benchmarks).                                                                                            |
| Vector DB / Storage | SQLite + sqlite-vec                                                | Vector search within the same `.db` file as transcripts/sessions.                                                                                                                                                                    |
| Question classifier | Rust, heuristics + regex                                           | Without external LLM — see §7 for rule details.                                                                                                                                                                                      |
| Post-call analysis  | BYOK (Anthropic/OpenAI/Gemini/OpenRouter/Ollama/OpenAI-compatible) | No latency pressure, user controls cost and privacy.                                                                                                                                                                                 |
| Secrets (API keys)  | Native OS keychain via `keyring` crate        | Never plain text in the `settings` table.                                                                                                                                                                                            |

## 6. Module architecture

```text
┌──────────────────────────────────────────────────────────────────────┐
│                        TAURI SHELL (Rust)                             │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │   Audio Capture                                              │    │
│  │   - Channel A: Microphone (cpal) — your voice               │    │
│  │     → MicVadState tracks silence→speech transitions         │    │
│  │   - Channel B: System Loopback (ScreenCaptureKit)            │    │
│  │             — interviewer                                    │    │
│  └────┬─────────────────────────────────────┬────────────────┘    │
│       │ Channel A (mic samples)             │ Channel B            │
│       ▼                                     ▼                      │
│  ┌──────────────┐               ┌──────────────────────┐          │
│  │ MicVadState  │               │ STT (Moonshine)       │          │
│  │ (VAD on mic) │               │ Only channel B in     │          │
│  │ tracks speech│               │ real time             │          │
│  │ start times  │               └──────┬───────────────┘          │
│  └──────┬───────┘                      ▼ streaming text            │
│         │                         ┌───────────────────────────┐   │
│         │ (for Shadow             │ Question Classifier        │   │
│         │  mode hint              │ (heuristics + regex)       │   │
│         │  cancellation)          │ - bilingual EN/ES          │   │
│         │                         │ - Type: Technical/STAR/    │   │
│         │                         │   Architecture/Trap/None   │   │
│         │                         └──────────┬────────────────┘   │
│         │                                    ▼ if question         │
│         │                    (HintJob via mpsc channel)            │
│         │     ┌──────────────────────────────────────────────┐   │
│         │     │ Orchestrator (HintScheduler + hint worker)    │   │
│         │     │ ┌────────────────────────────────────────┐   │   │
│         │     │ │ 1. RAG search (top_k=1)                │   │   │
│         │     │ │ 2. Build hint format (tag+metric/chunk)│   │   │
│         │     │ │ 3a. Practice → emit "new-hint" now     │   │   │
│         │     │ │ 3b. Shadow → schedule 2.5s delay       │   │   │
│         │     │ │ 4. On fire: check MicVadState —        │   │   │
│         │ ◄───│─│─ if user spoke since scheduled → CANCEL │   │   │
│         │     │ └────────────────────────────────────────┘   │   │
│         │     └──────────────────┬───────────────────────────┘   │
│         │                        ▼ Tauri event ("new-hint")       │
│         │     ┌──────────────────────────────────────────────┐ │ │
│         │     │  Overlay (Tauri window)                       │ │ │
│         │     │  - transparent, always-on-top, click-through  │ │ │
│         │     │  - 400×100, hint auto-dismiss after 3s       │ │ │
│         │     │  - Panic button (planned)                     │ │ │
│         │     └──────────────────────────────────────────────┘ │ │
│         │                                                       │ │
│  ┌──────┴────────────────────────────────────────────────────┐ │ │
│  │  SQLite (single source of truth)                           │ │ │
│  │  transcripts · docs (vectors) · sessions · settings         │ │ │
│  └───────────────────────────────────────────────────────────┘ │ │
└──────────────────────────────────────────────────────────────────────┘
```

## 7. Question classifier — details

Heuristic rules, no LLM. Implemented at `src-tauri/src/classifier/mod.rs`:

- **Question signal:** question mark (`?`) **OR** imperative verb near sentence start — "cuéntame", "dime", "descríbeme", "explícame", "camínenme por", "tell me", "describe", "explain", "walk me through", and variants. Leading filler words ("Bueno", "So") are tolerated.
- **Exclusion list:** small talk phrases like "¿cómo estás?", "¿me escuchas bien?", "how are you?", "can you hear me?" → `None`.
- **Type classification (Technical / STAR / Architecture / Trap):** keyword density scoring across four categories, each with ~20–40 keywords in Spanish and English:
  - **Technical:** code, debug, API, database, algorithm, performance, concurrency, async, framework, "how did you implement", etc.
  - **STAR:** leadership, team, conflict, situation, negotiation, collaboration, failure, "tell me about a time", "give me an example", etc.
  - **Architecture:** scalability, design, pattern, microservices, event sourcing, hexagonal, DDD, coupling, distributed, high availability, etc.
   - **Trap:** weakness, failure, worst, mistake, regret, frustration, "what would you do differently", "why should we hire you", professional weakness/failure, worst project, negative experience/feedback, awkward situation, what did you learn from your mistakes, etc.
- **Experience question heuristic:** If the text matches "cuéntame", "tell me about", "walk me through" (without "code"), "dime", etc., it defaults to STAR.
- **Fallback:** Questions without any keyword match default to Technical.
- **Tie-breaking:** Trap > Architecture > STAR > Technical when multiple categories have the same score.
- **Bilingual:** All keyword lists and imperative triggers cover both Spanish and English.
- **Naming convention (Rust):** The internal `QuestionType` enum uses `Star` (not `STAR`) — Rust/serde `rename_all = "lowercase"` serialises it as `"star"` in JSON events. Semantically identical to STAR in this document.

## 8. Data model

```sql
sessions(id, started_at, ended_at, company, role, mode)  -- mode: practice|shadow
transcript_lines(id, session_id, speaker, text, started_at_ms, ended_at_ms)
documents(id, filename, type, added_at)
chunks(id, document_id, text, chunk_index, tag, metric)
chunks_vec(chunk_id, embedding)  -- vía sqlite-vec
settings(key, value)  -- includes retain_audio (bool, default false); does NOT include API keys (see §5, Keychain)
```

## 9. User flow

 1. **Initial setup:** the user uploads CV/projects, the app indexes (candle generates embeddings → sqlite-vec). If they want post-call, they configure their API key (stored in keychain, not in `settings`). On first launch, a background thread (`kue-moonshine-provision`) auto-downloads Moonshine dylibs + model (~482 MB total) from PyPI and `download.moonshine.ai`. The frontend's `ProvisioningProgress` component (`src/ProvisioningProgress.tsx`) blocks the main UI and provides a download progress UI with a real progress bar, file counter, stage labels, error display, and a "Reintentar" (retry) button. Progress is reported via `moonshine-download-progress` Tauri events; if the download fails, the `retry_moonshine_download` Tauri command retries. The `is_moonshine_provisioned` Tauri command lets the frontend check whether Moonshine is already installed, skipping the download UI if already provisioned.
2. **During the interview:** chooses Practice or Shadow. Dual audio capture starts. Moonshine transcribes channel B. The classifier detects questions and triggers RAG. The overlay window (transparent, always-on-top, click-through) shows the hint for ~3s and disappears (immediate in Practice, after 2.5s of stalling in Shadow). In Shadow mode, if the user starts speaking before the 2.5s delay expires (detected via mic VAD on Channel A), the hint is silently cancelled. Session lifecycle events: `session-started` is emitted when the user clicks "Start" (includes `mode` and `session_id` in the payload) — the overlay auto-shows. `session-stopped` is emitted when the user clicks "Stop" — the overlay auto-hides. **Panic/Mute button:** if at any point the user feels overwhelmed or the hints are distracting, pressing the panic button silences all hints for 10s (`PanicState` in Tauri state); a `panic-mode` event is emitted and both the main and overlay UIs show a mute indicator.
 3. **Post-call:** full transcript is saved (both channels — Channel A transcribed in batch at session end via ADR-015, Channel B transcribed in real time). A post-call panel in the session control UI shows batch transcription status (via `is_transcript_ready` command and `post-call-transcript-ready` event). Once ready, the user configures a provider (Anthropic/OpenAI/Gemini/OpenRouter/Ollama) and optional model, then clicks "Analyze." The `analyze_session` command sends transcript + RAG context to the chosen LLM (BYOK) and returns a structured analysis (summary, weak questions, unmentioned projects, STAR structure improvements). API keys are stored in the OS keychain via `keyring`, never in the `settings` table. The provisioning backend emits `moonshine-download-progress` (`stage`, `file_index`, `file_count`, `downloaded_bytes`, `total_bytes`), `moonshine-provision-error`, and `moonshine-provisioned` events. The frontend's `ProvisioningProgress` component (at `src/ProvisioningProgress.tsx`) renders a real progress bar, stage label, file counter, error display, and a "Reintentar" button wired to `retry_moonshine_download`. On mount, it checks `is_moonshine_provisioned` and skips the UI if already provisioned.

## 10. Development plan (MVP)

| Sprint | Objective           | Deliverables                                                                                                                                                                                                                                                                                                                                                                              | Status           |
| ------ | ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| 0      | Base infrastructure | Tauri + React project. Rust dependencies (`cpal`, `screencapturekit-rs`, `tauri`, `rusqlite`+`sqlite-vec`, `candle`). Complete SQLite schema (sessions, transcript_lines, documents, chunks, chunks_vec, settings) with migrations. sqlite-vec registered. Dual audio capture (mic cpal + loopback SCK) with WAV writing. `get_db_status`, `start_session`, `stop_session`, and `panic_mode` commands. 48+ tests. | ✅ **Completed** |
| 1      | STT (Moonshine)     | Moonshine integration on channel B. STTEngine trait + FFI engine (`libmoonshine.dylib`) + CLI fallback (`moonshine-voice transcribe`). Simple VAD (energy-based). Pipeline thread that receives audio from loopback, segments by VAD, transcribes, emits `new-transcript` event and persists in `transcript_lines`. `persist_transcript_line` and `create_engine` extracted to module level in `stt/mod.rs`.                                                                       | ✅ **Completed** |
| 2      | RAG Engine          | Local document indexing. sqlite-vec + candle generating and searching embeddings. Goal: query <20ms.                                                                                                                                                                                                                                                                                      | ✅ **Completed** |
| 3 | Classifier & hints | Rules from §7 in Rust. `QuestionType` enum (Technical/STAR/Architecture/Trap/None) with heuristic + keyword-density scoring. `classify_text` Tauri command registered in invoke_handler. Classifier wired into STT pipeline — each transcribed line is classified and emits `question-detected` event (see `stt/pipeline.rs`). Orchestrator module (`orchestrator/mod.rs` + `orchestrator/worker.rs`) integrates classifier + RAG to produce hints: RAG search (top_k=1), `💡 {tag}: {metric}` format (max 8 words), generic fallback per type. Dedicated hint worker thread (`kue-hint-worker`) processes HintJobs via mpsc channel, uses `HintScheduler` for Shadow mode's 2.5s delay, gates shadow hints via mic VAD, and emits `new-hint` events via Tauri. | ✅ **Completed** (48 classifier tests + 73 orchestrator tests) |
| 4      | Overlay & UI        | Transparent window, always-on-top, click-through (implemented). React Overlay component with 3s auto-dismiss, auto-show/hide on session events (implemented). Mic VAD for Shadow-mode hint cancellation (implemented). Practice vs Shadow mode routing (implemented). Channel A batch transcription (implemented — `stt/batch.rs`). Panic button (`panic_mode` command + `PanicState` + `panic-mode` event with 10s hint silence, implemented). Mode selection UI in `App.tsx` with Practice/Shadow toggle (implemented). Hint positioned at top-center (transparent floating window, `items-start justify-center pt-8`).                                                                                                                                                                                                                                                                                                       | ✅ **Completed** |
| 5      | Post-call & BYOK    | SQLite export/query. External API call (`analyze_session` Tauri command, Anthropic/OpenAI/Gemini/OpenRouter/Ollama). Secure key storage in OS Keychain (`keys.rs` via `keyring` crate). Analysis prompt with RAG context injection + JSON response parsing. `BatchTracker` for async batch-transcription completion tracking. Post-call panel in `App.tsx` with provider selection and result display (summary, weak questions, forgotten projects, STAR improvements). | ✅ **Completed** |
| 6      | Moonshine auto-provision | First-launch download of `libmoonshine.dylib` + `libonnxruntime.*.dylib` from PyPI wheel (v0.0.73, macOS arm64) + Medium Streaming English model (~429 MB, 8 ONNX files) from `download.moonshine.ai`. `stt/provisioning.rs` with progress events (`moonshine-download-progress`), integrity verification (size checks), idempotency, offline error handling, `retry_moonshine_download` and `is_moonshine_provisioned` Tauri commands. Frontend `ProvisioningProgress.tsx` with progress bar, file counter, stage labels, error display, and retry button — gates access to `MainApp` until provisioning completes. Updated `stt/ffi.rs` / `stt/mod.rs` to prefer `{app_data_dir}/moonshine/` with dev-path fallback. | ✅ **Completed** |

## 11. Open Questions / risks

- **Legality of recording without explicit consent from the other party** — review for Spain at minimum before this becomes a regular habit. Partially mitigated by ADR-011 (audio doesn't persist by default), but the underlying legal question remains open for when the user enables retention or for the text transcript itself.
- ~~Own benchmark between `all-MiniLM-L6-v2` and `snowflake-arctic-embed-s`~~ — Resolved (ADR-012): chose snowflake-arctic-embed-s based on public retrieval benchmarks (MTEB/BEIR published by Snowflake), not an own test on the user's documents. Same 384 dims as MiniLM, no impact on the schema.
- ~~Chunk size / overlap for context RAG~~ — Resolved (ADR-013): `CHUNK_SIZE=150` words, `CHUNK_OVERLAP=20` words, empirically validated against the 512-token limit of `snowflake-arctic-embed-s` (BERT WordPiece) — see `test_chunk_size_fits_in_model_context` in `rag/indexer.rs`.
- ScreenCaptureKit stability on macOS <14 — there are reports of intermittent segfaults on older versions; validate the minimum supported version before committing to it in onboarding.
- ~~Hint positioning in overlay — resolved with top-center (`items-start justify-center pt-8` in `Overlay.tsx`); unobtrusive for a 400×100 transparent floating window.~~
- 2.5s threshold in Shadow — empirically validate that it doesn't feel too anxious nor too late.
