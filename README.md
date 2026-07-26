# Kue

> Memory copilot for technical interviews — helps you recall your own metrics, projects, and structure without generating answers for you.

Desktop application (macOS, Tauri v2) with real-time transcription, local RAG over your CV/projects, and ultra-short hints to maintain fluency under pressure. Post-call, optional analysis with your own LLM (BYOK).

**Current status:** Sprints 0–4 completed, Sprint 5 (post-call) next. All core modules are implemented and tested: base infrastructure (Tauri + SQLite/sqlite-vec), dual audio capture (microphone via `cpal` + system loopback via ScreenCaptureKit), RAG engine (embeddings with `candle` + vector search with `sqlite-vec`), STT module (Moonshine FFI + CLI fallback + VAD + pipeline + batch transcription for Channel A), question classifier (heuristics + regex with bilingual EN/ES keyword lists including trap keywords for regret/failure) wired into the STT pipeline, orchestrator (HintScheduler + hint worker + PanicState) producing hints from classifier+RAG, overlay window (transparent, always-on-top, click-through, 400×100) with hint display component (3s auto-dismiss via React `Overlay` component), mic VAD for Shadow-mode hint cancellation, panic/mute button (10s hint silence via `PanicState`), mode selection UI (Practice/Shadow in `App.tsx`), and Channel A batch transcription at session end (ADR-015). All implemented and tested (>400 tests in Rust). Remaining work for v1: hint positioning clean-ups (Sprint 4) and post-call BYOK analysis (Sprint 5) — see `spec.md`.

---

## Prerequisites

- **Rust** 1.79+ (`rustup install 1.79`)
- **Node.js** 18+ (recommended 20 LTS)
- **macOS 13+** (ScreenCaptureKit required for system audio loopback)

## Getting Started

```bash
# Clone the repository
git clone <repo-url> && cd kue

# Install frontend dependencies
npm install

# Start development mode (Tauri + Vite)
npm run tauri:dev
```

The app will open a window with debug controls to index folders (RAG) and search context. The Rust backend connects to SQLite and creates the schema at `~/Library/Application Support/com.kue.app/kue.db`.

## Tests

```bash
# Rust tests (database — all logic implemented)
npm run test:rust:db

# Rust tests (all modules, >400 tests)
npm run test:rust

# Rust coverage (requires cargo-tarpaulin)
npm run coverage:rust:db
npm run coverage:rust:full
```

## Available Scripts

| Command | Description |
|---|---|
| `npm run dev` | Standalone Vite server (without Tauri) |
| `npm run build` | Frontend build (TypeScript + Vite) |
| `npm run preview` | Preview of frontend build |
| `npm run tauri` | Tauri CLI directly |
| `npm run tauri:dev` | Tauri + Vite in development mode |
| `npm run tauri:build` | Production build (generates .dmg) |
| `npm run test:rust:db` | Tests for the database module only |
| `npm run test:rust` | Tests for all Rust modules (>400 tests) |
| `npm run coverage:rust` | Runs Rust tests (alias for `test:rust`) |
| `npm run coverage:rust:check` | Checks availability of coverage tools |
| `npm run coverage:rust:db` | Coverage for the database module (tarpaulin) |
| `npm run coverage:rust:full` | Full Rust coverage (tarpaulin) |
| `npm run coverage:rust:text` | Coverage on stdout (tarpaulin) |

## Architecture (high level)

```
┌────────────────────────────────────────────────────────┐
│                  Tauri v2 Shell                         │
│  ┌────────────┐ ┌─────────────────────────────────┐    │
│  │ Frontend   │ │  Rust Backend                    │    │
│  │            │ │  - db::init_db                   │    │
│  │ ┌────────┐ │ │  - db::get_db_status cmd        │    │
│  │ │MainApp │ │◄│  - audio::capture::start_session│    │
│  │ │(Session│ │ │  - audio::capture::stop_session │    │
│  │ │Control │ │ │  - audio::capture::panic_mode   │    │
│  │ │UI)     │ │ │  - audio::mic_vad (MicVadState) │    │
│  │ └────────┘ │ │  - rag::index_folder_cmd        │    │
│  │            │ │  - rag::search_context cmd      │    │
│  │ ┌────────┐ │ │  - overlay::show_overlay cmd   │    │
│  │ │ Overlay│ │ │  - stt::pipeline (VAD→STT→     │    │
│  │ │(hint   │ │ │    classify→events→DB)          │    │
│  │ │ window)│ │ │  - classifier::classify_text    │    │
│  │ └────────┘ │ │    (question classification)    │    │
│  │            │ │  - orchestrator (HintScheduler+ │    │
│  │ React +    │ │    PanicState)                  │    │
│  │ Tailwind   │ │  - types (TranscriptLine,       │    │
│  │            │ │    Speaker + Copy)              │    │
│  └────────────┘ │  - cpal (mic capture)           │    │
│                 │  - SCK (loopback)               │    │
│                 │  - hound (WAV writer)           │    │
│                 │  ┌───────────────────────────┐ │    │
│                 │  │  Overlay Window            │ │    │
│                 │  │  (transparent, always-on-  │ │    │
│                 │  │  top, click-through, 400x100) │ │
│                 │  └───────────────────────────┘ │    │
│                 │  ┌───────────────────────────┐ │    │
│                 │  │  RAG Engine                │ │    │
│                 │  │  - rag::embeddings         │ │    │
│                 │  │    (candle BERT, 384-d)    │ │    │
│                 │  │  - rag::indexer            │ │    │
│                 │  │    (ingest / search /      │ │    │
│                 │  │     chunk / folder)        │ │    │
│                 │  └───────────────────────────┘ │    │
│                 │  ┌───────────────────────────┐ │    │
│                 │  │  STT Module                │ │    │
│                 │  │  - stt::ffi               │ │    │
│                 │  │    (Moonshine FFI)        │ │    │
│                 │  │  - stt::cli (fallback)    │ │    │
│                 │  │  - stt::vad (energy)      │ │    │
│                 │  │  - stt::pipeline          │ │    │
│                 │  │    (VAD→STT→classify→     │ │    │
│                 │  │     events→DB)            │ │    │
│                 │  │  - stt::batch             │ │    │
│                 │  │    (post-session Ch. A    │ │    │
│                 │  │     VAD+STT, speaker=user)│ │    │
│                 │  │  - stt::mod (module-level │ │    │
│                 │  │    persist_transcript_    │ │    │
│                 │  │    line & create_engine)  │ │    │
│                 │  └───────────────────────────┘ │    │
│                 │  ┌───────────────────────────┐ │    │
│                 │  │  Orchestrator              │ │    │
│                 │  │  - HintScheduler           │ │    │
│                 │  │  - HintJob/HintCommand     │ │    │
│                 │  │  - hint worker thread      │ │    │
│                 │  │  - PanicState (10s silence)│ │    │
│                 │  │  - should_cancel_hint      │ │    │
│                 │  │    (mic VAD gating)        │ │    │
│                 │  │  - generate_and_emit_hint  │ │    │
│                 │  │    (classify→RAG→hint→     │ │    │
│                 │  │     emit, Practice immedi- │ │    │
│                 │  │     ate / Shadow delayed)  │ │    │
│                 │  └───────────────────────────┘ │    │
│                 └─────────────────────────────────┘    │
│  ┌────────────────────────────────────────────────┐    │
│  │  SQLite + sqlite-vec                            │    │
│  │  (sessions · transcript_lines · documents ·     │    │
│  │   chunks · chunks_vec · settings)               │    │
│  └────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────┘
```

**Legend:** All listed code is functional and tested. `candle` implements BERT embeddings (`snowflake-arctic-embed-s`) in the `rag::embeddings` module, and `sqlite-vec` performs KNN vector search. Session control is handled by three separate Tauri commands (`start_session`, `stop_session`, `panic_mode`) instead of a single `toggle_audio_capture`. The STT pipeline integrates Moonshine (FFI + CLI fallback), VAD, classification, DB persistence, and pushes hint jobs to the orchestrator. The classifier module uses heuristic rules (regex + keyword density, bilingual EN/ES) with trap keywords for regret and failure topics. The orchestrator module stitches classifier + RAG → hints (immediate in Practice, delayed in Shadow) via a dedicated worker thread; in Shadow mode, hints are gated by `audio::mic_vad::MicVadState` — if the user starts speaking on Channel A before the 2.5s delay expires, the hint is silently cancelled. A `PanicState` registered in Tauri state silences all hints for 10s when activated via the panic button/mute command. At session end, Channel A (mic) audio is batch-transcribed via Moonshine + SimpleVAD in a dedicated thread (`kue-batch-transcribe`) and persisted with `speaker='user'` (ADR-015). The overlay window (transparent, always-on-top, click-through, 400×100) displays hints via an `Overlay` React component that listens for `new-hint` Tauri events, auto-shows in Shadow mode via `session-started`, auto-hides on `session-stopped`, and displays a panic indicator on `panic-mode`. Hints auto-dismiss after 3s.

## Stack

| Layer | Technology |
|---|---|
| Frontend | React 18 + TypeScript + Tailwind CSS 3 |
| App Core | Rust (Tauri v2), session commands: `start_session`/`stop_session`/`panic_mode` |
| Database | SQLite + sqlite-vec (vectors) |
| Audio | cpal (mic) + screencapturekit-rs (loopback) + hound (WAV) |
| STT + Classifier | Moonshine (FFI + CLI fallback) + heuristics/regex classifier (bilingual EN/ES, trap keywords for regret/failure) + batch transcription (Ch. A) |
| Hint Engine | orchestrator module (HintScheduler + hint worker thread + PanicState) |
| Embeddings | candle (HuggingFace Rust) + `snowflake-arctic-embed-s` |
| Overlay Window | Tauri v2 multi-window (transparent, click-through, always-on-top, auto-show/hide on session events) |
| Mic VAD (Shadow gating) | `audio::mic_vad::MicVadState` wraps `SimpleVAD` for Channel A |
| Panic/Mute | `PanicState` in Tauri state silences hints for 10s via `panic_mode` command + `panic-mode` event |
| Post-call (planned) | BYOK (Anthropic/OpenAI/Ollama/etc.) |

## Related documentation

- [`spec.md`](./spec.md) — Complete functional specification of the product
- [`design.md`](./design.md) — Technical design and current architecture
- [`adr.md`](./adr.md) — Architecture decision records

## Project

```
kue/
├── src/                     # Frontend React + TypeScript
│   ├── App.tsx              # App router: renders MainApp (session control UI) or Overlay by window label
│   ├── Overlay.tsx          # Hint overlay component (listens for new-hint, session-started/stopped, panic-mode events, 3s auto-dismiss)
│   ├── main.tsx             # Entry point
│   └── index.css            # Tailwind directives
├── src-tauri/               # Rust backend (Tauri)
│   ├── src/
│   │   ├── main.rs          # Entry point
│   │   ├── lib.rs           # Tauri builder + setup (registers DB, AudioCapture, RAG,
│   │   │                    #   overlay click-through, PanicState, cleans orphan temp dirs)
│   │   ├── types.rs         # TranscriptLine, Speaker (STT → classifier contract, Copy derive)
│   │   ├── db/
│   │   │   └── mod.rs       # Schema, migrations, sqlite-vec, tests
│   │   ├── audio/
│   │   │   ├── mod.rs       # Re-exports capture and mic_vad modules
│   │   │   ├── capture.rs   # Dual capture (cpal + SCK), WAV writer, start_session/stop_session/panic_mode cmds
│   │   │   └── mic_vad.rs   # MicVadState — VAD for Channel A (Shadow-mode hint cancellation)
│   │   ├── overlay.rs       # show_overlay Tauri command (show/hide overlay window)
│   │   ├── classifier/
│   │   │   └── mod.rs       # Question classification (heuristics + regex, bilingual EN/ES, trap keywords for regret/failure, 48+ tests)
│   │   ├── orchestrator/
│   │   │   ├── mod.rs       # HintJob, HintScheduler, PanicState, should_cancel_hint, generate_and_emit_hint
│   │   │   └── worker.rs    # hint worker thread (poll loop + expired hint emission)
│   │   ├── stt/
│   │   │   ├── mod.rs       # STTEngine trait, STTConfig, persist_transcript_line, create_engine (module-level)
│   │   │   ├── ffi.rs       # MoonshineFFIEngine (libmoonshine.dylib via libloading)
│   │   │   ├── cli.rs       # MoonshineCLIEngine (fallback to moonshine-voice CLI)
│   │   │   ├── vad.rs       # SimpleVAD (energy-based)
│   │   │   ├── pipeline.rs  # STTPipeline (thread loopback→VAD→STT→events→DB)
│   │   │   └── batch.rs     # Channel A batch transcription (VAD+STT, post-session)
│   │   └── rag/
│   │       ├── mod.rs       # Re-export of embeddings and indexer
│   │       ├── embeddings.rs # BERT model (snowflake-arctic-embed-s), embedding generation
│   │       └── indexer.rs   # Ingestion, chunking, indexing, and vector search
│   ├── Cargo.toml           # Rust dependencies
│   ├── tauri.conf.json      # Tauri v2 configuration (main + overlay windows)
│   └── capabilities/
│       ├── default.json     # Permissions for main window (core:default + shell:allow-open)
│       └── overlay.json     # Minimal permissions for overlay window (core:default)
├── package.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
└── tsconfig.json
```
