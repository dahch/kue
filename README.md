# Kue

> Memory copilot for technical interviews — helps you recall your own metrics, projects, and structure without generating answers for you.

Desktop application (macOS, Tauri v2) with real-time transcription, local RAG over your CV/projects, and ultra-short hints to maintain fluency under pressure. Post-call, optional analysis with your own LLM (BYOK). Practice mode includes an AI interviewer with TTS-generated questions from a tailored plan.

**Current status:** All v1 features implemented and tested. Core: dual audio capture (mic via `cpal` + system loopback via ScreenCaptureKit), RAG engine (embeddings with `candle` + vector search with `sqlite-vec`), STT (Moonshine FFI + CLI fallback + VAD + pipeline + batch transcription for Channel A), question classifier (heuristics + regex, bilingual EN/ES), orchestrator (HintScheduler + hint worker + PanicState), overlay window (transparent, always-on-top, click-through, 400×100) with hint display (3s auto-dismiss), mic VAD for Shadow-mode hint cancellation, panic/mute button (10s silence), Channel A batch transcription at session end (ADR-015), post-call BYOK analysis (keychain-stored API keys, provider selection for Anthropic/OpenAI/Gemini/OpenRouter/DeepSeek/Ollama), Moonshine auto-provisioning, onboarding wizard, session metadata persistence, file logging, Tauri auto-updater. **New in latest sprints:** AI Interview mode (`interview_plan.rs` + `interview_runner.rs`) generates a question plan from a job description using BYOK LLM and reads questions aloud via macOS `say` TTS; i18n system (`src/i18n.ts`) with bilingual EN/ES support and language switcher in the header; `llm` module extracting shared LLM response types; `tts` module for text-to-speech; `ReindexPanel` for on-demand folder re-indexing; `get_log_dir_path` command for log access. All implemented and tested.

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

If Moonshine is not yet provisioned on first launch, a `ProvisioningProgress` UI (progress bar, file counter, stage label, retry button) blocks entry until downloads complete. Once provisioned, the app shows an **Onboarding wizard** (`src/Onboarding.tsx`) that guides the user through screen recording permission, embedding model loading, API key configuration, and folder indexing (steps 1–4). After onboarding, the main session control UI unlocks (mode selector, start/stop, company/role inputs, panic button, transcript log with chat-bubble styling, session history, post-call analysis panel, AI Interview plan generator, reindex dialog). The Rust backend connects to SQLite and creates the schema at `~/Library/Application Support/com.kue.app/kue.db`.

**Practice mode** now supports an **AI interviewer**: paste a job description, Kue generates a question plan via your configured LLM, then reads each question aloud using macOS `say` TTS (`tts/mod.rs`). The session transcript captures both your response and the AI's questions. **Shadow mode** works as before (eavesdrops on a real call).

## Tests

```bash
# Frontend unit tests (Vitest + Testing Library, jsdom)
npm run test
npm run test:watch    # watch mode

# Rust tests (database — all logic implemented)
npm run test:rust:db

# Rust tests (all modules, ~530 tests)
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
| `npm run test` | Frontend unit tests (Vitest) |
| `npm run test:watch` | Frontend tests in watch mode |
| `npm run test:rust:db` | Tests for the database module only |
| `npm run test:rust` | Tests for all Rust modules (~530 tests) |
| `npm run coverage:rust` | Runs Rust tests (alias for `test:rust`) |
| `npm run coverage:rust:check` | Checks availability of coverage tools |
| `npm run coverage:rust:db` | Coverage for the database module (tarpaulin) |
| `npm run coverage:rust:full` | Full Rust coverage (tarpaulin) |
| `npm run coverage:rust:text` | Coverage on stdout (tarpaulin) |

## Architecture (high level)

```
┌───────────────────────────────────────────────────────────────┐
│                    Tauri v2 Shell                              │
│  ┌──────────────┐ ┌──────────────────────────────────────┐    │
│  │  Frontend     │ │  Rust Backend                        │    │
│  │  (React + TS) │ │                                      │    │
│  │  Header.tsx   │ │  ▸ db (init_db, get/set_settings,    │    │
│  │   (i18n EN/ES)│ │      get_sessions, get_transcript)   │    │
│  │  MainApp       │ │  ▸ audio (start/stop session,       │    │
│  │   (session    │ │      panic_mode, is_transcript_ready, │    │
│  │    control,   │◄│      get_log_dir_path) | mic_vad     │    │
│  │    transcript,│ │  ▸ classifier (question heuristics)  │    │
│  │    PlanGen,   │ │  ▸ orchestrator (HintScheduler +     │    │
│  │    LiveInter- │ │      PanicState + hint worker)       │    │
│  │    view,      │ │  ▸ stt (FFI/CLI/VAD/pipeline/batch   │    │
│  │    PostCall,  │ │      /provisioning)                  │    │
│  │    Reindex)   │ │  ▸ rag (embeddings + indexer)        │    │
│  │  Overlay.tsx  │ │  ▸ analyze (post-call BYOK)          │    │
│  │   (hint       │ │  ▸ keys (keychain API keys)          │    │
│  │    overlay)   │ │  ▸ onboarding (first-run wizard)     │    │
│  │               │ │  ▸ logging (file logger + rotation)  │    │
│  │               │ │  ▸ llm (shared response types)       │    │
│  │               │ │  ▸ tts (macOS say, Samantha voice)   │    │
│  │               │ │  ▸ interview_plan (plan generation)  │    │
│  │               │ │  ▸ interview_runner (AI Interview    │    │
│  │               │ │      orchestrator)                   │    │
│  │               │ │  ▸ overlay (show/hide overlay win)   │    │
│  └──────────────┘ │                                       │    │
│                   │  ┌──────────────────────────────┐    │    │
│                   │  │  Audio (hardware)             │    │    │
│                   │  │  cpal (mic) + SCK (loopback)  │    │    │
│                   │  │  hound (WAV) + rubato (resamp)│    │    │
│                   │  └──────────────────────────────┘    │    │
│                   │  ┌──────────────────────────────┐    │    │
│                   │  │  Overlay Window               │    │    │
│                   │  │  (transparent, always-on-top, │    │    │
│                   │  │   click-through, 400×100)     │    │    │
│                   │  └──────────────────────────────┘    │    │
│                   │  ┌──────────────────────────────┐    │    │
│                   │  │  TTS (macOS say)              │    │    │
│                   │  │  Used by interview_runner     │    │    │
│                   │  │  to speak AI questions aloud  │    │    │
│                   │  └──────────────────────────────┘    │    │
│                   └──────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  SQLite + sqlite-vec                                  │    │
│  │  (sessions · transcript_lines · documents · chunks ·  │    │
│  │   chunks_vec · settings)                              │    │
│  └──────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  Tauri Events (selected)                              │    │
│  │  new-transcript · question-detected · new-hint ·     │    │
│  │  panic-mode · session-started/stopped ·               │    │
│  │  post-call-transcript-ready · post-call-transcript-│    │
│  │  error · interview-question · interview-status ·     │    │
│  │  interview-finished                                   │    │
│  └──────────────────────────────────────────────────────┘    │
└───────────────────────────────────────────────────────────────┘
```

**Legend:** All listed code is functional and tested. `candle` implements BERT embeddings (`snowflake-arctic-embed-s`) in the `rag::embeddings` module, and `sqlite-vec` performs KNN vector search. Session control is handled by three separate Tauri commands (`start_session`, `stop_session`, `panic_mode`) instead of a single `toggle_audio_capture`. The STT pipeline integrates Moonshine (FFI + CLI fallback), VAD, classification, DB persistence, and pushes hint jobs to the orchestrator. The classifier module uses heuristic rules (regex + keyword density, bilingual EN/ES) with trap keywords for regret and failure topics. The orchestrator module stitches classifier + RAG → hints (immediate in Practice, delayed in Shadow) via a dedicated worker thread; in Shadow mode, hints are gated by `audio::mic_vad::MicVadState` — if the user starts speaking on Channel A before the 2.5s delay expires, the hint is silently cancelled. A `PanicState` registered in Tauri state silences all hints for 10s when activated via the panic button/mute command. At session end, Channel A (mic) audio is batch-transcribed via Moonshine + SimpleVAD in a dedicated thread (`kue-batch-transcribe`) and persisted with `speaker='user'` (ADR-015). After batch transcription completes, the full transcript (both speakers) is available for post-call analysis: the `analyze_session` command sends the transcript + RAG context to a user-configured LLM via BYOK (Anthropic/OpenAI/Ollama/etc.), with API keys stored in the OS keychain (not in the `settings` table). On first launch, `stt::provisioning` auto-downloads Moonshine dylibs + model (~482 MB) from PyPI and `download.moonshine.ai`, reporting progress via `moonshine-download-progress` events and offering retry via the `retry_moonshine_download` Tauri command. The overlay window (transparent, always-on-top, click-through, 400×100) displays hints via an `Overlay` React component that listens for `new-hint` Tauri events, auto-shows in Shadow mode via `session-started`, auto-hides on `session-stopped`, and displays a panic indicator on `panic-mode`. Hints auto-dismiss after 3s.

## Stack

| Layer | Technology |
|---|---|
| Frontend | React 18 + TypeScript + Tailwind CSS 3 + Vitest + Testing Library |
| App Core | Rust (Tauri v2), session commands: `start_session`/`stop_session`/`panic_mode` |
| Database | SQLite + sqlite-vec (vectors) |
| Audio | cpal (mic) + screencapturekit-rs (loopback, vendored) + hound (WAV) |
| STT + Classifier | Moonshine (FFI + CLI fallback) + heuristics/regex classifier (bilingual EN/ES, trap keywords for regret/failure) + batch transcription (Ch. A) + auto-provisioning (`stt/provisioning.rs` downloads dylibs + model on first launch, progress events, `retry_moonshine_download` + `is_moonshine_provisioned` Tauri commands) |
| Hint Engine | orchestrator module (HintScheduler + hint worker thread + PanicState) |
| Embeddings | candle (HuggingFace Rust) + `snowflake-arctic-embed-s`, CPU+Accelerate backend |
| Overlay Window | Tauri v2 multi-window (transparent, click-through, always-on-top, auto-show/hide on session events) |
| Mic VAD (Shadow gating) | `audio::mic_vad::MicVadState` wraps `SimpleVAD` for Channel A |
| Panic/Mute | `PanicState` in Tauri state silences hints for 10s via `panic_mode` command + `panic-mode` event |
| Post-call | BYOK (Anthropic/OpenAI/Gemini/OpenRouter/DeepSeek/Ollama) via `analyze_session` command, API keys stored in OS Keychain via `keyring` crate |
| **AI Interview** | `interview_plan.rs` generates question plans from job descriptions via BYOK LLM; `interview_runner.rs` orchestrates the TTS-driven AI interviewer flow; `tts/mod.rs` wraps macOS `say` for text-to-speech |
| **LLM Client** | `llm/mod.rs` — shared response types (OpenAI, Anthropic, Gemini, etc.) for post-call analysis and interview plan generation |
| **i18n** | `src/i18n.ts` — bilingual EN/ES with `useLanguage()` hook, UI language switcher in `Header.tsx`, language persisted in `settings` table |
| Logging | `logging::Logger` — file logs with rotation (keeps last 5 files) to `{app_data_dir}/logs/` via `log` + `chrono` crates; `get_log_dir_path` command for frontend access |
| Onboarding | First-run wizard (`onboarding.rs` + `Onboarding.tsx`) — 4 steps: screen permission, embedding model loading, API key config, folder indexing |
| Auto-updater | `tauri-plugin-updater` — built-in Tauri v2 updater supporting signed DMG updates |

## Related documentation

- [`spec.md`](./spec.md) — Complete functional specification of the product
- [`design.md`](./design.md) — Technical design and current architecture
- [`adr.md`](./adr.md) — Architecture decision records

## Project

```
kue/
├── src/                     # Frontend React + TypeScript
│   ├── App.tsx              # App router + MainApp (session control, transcript bubbles,
│   │                        #   AI Interview, PlanGenerator, ReindexPanel, LiveInterview,
│   │                        #   SessionList, PostCallPanel) — all in one file
│   ├── main.tsx             # Entry point
│   ├── index.css            # Tailwind directives + custom animations
│   ├── Header.tsx           # Sticky header with Kue logo and language switcher (ES/EN)
│   ├── i18n.ts              # i18n system: translations EN/ES, useLanguage hook, t() function,
│   │                        #   persistence via localStorage + backend settings table
│   ├── Icon.tsx             # Inline SVG icon set (30 icons, 24×24 grid, aria-hidden)
│   ├── ui.tsx               # Shared UI primitives: Spinner, SectionLabel, Equalizer, StyledSelect
│   ├── hooks.ts             # Custom hooks: usePersistedSetting (get+set via backend),
│   │                        #   useTauriEvent (auto-cleanup listener)
│   ├── types.ts             # IndexSummary interface
│   ├── validation.ts        # sanitizeError, isValidFolderPath, formatIndexResult
│   ├── ApiKeyInput.tsx      # API key input component for post-call + plan generation
│   ├── ProvisioningProgress.tsx  # Moonshine download progress UI (progress bar, file counter,
│   │                        #   error display, retry button), gates access to Onboarding
│   ├── Onboarding.tsx       # 4-step first-run wizard: screen permission, model load, API key,
│   │                        #   folder indexing — gates access to MainApp
│   ├── Overlay.tsx          # Hint overlay component (listens for new-hint, session-started/stopped,
│   │                        #   panic-mode events, 3s auto-dismiss)
│   └── __tests__/           # Frontend unit tests (Vitest + Testing Library, jsdom)
│       ├── setup.ts         # Tauri API mocks (invoke, listen, webviewWindow)
│       ├── Onboarding.test.tsx
│       ├── PostCallPanel.test.tsx
│       └── ProvisioningProgress.test.tsx
├── src-tauri/               # Rust backend (Tauri)
│   ├── src/
│   │   ├── main.rs          # Entry point
│   │   ├── lib.rs           # Tauri builder + setup (23 Tauri commands registered)
│   │   ├── types.rs         # TranscriptLine, Speaker (STT → classifier contract)
│   │   ├── db/
│   │   │   └── mod.rs       # Schema, migrations, sqlite-vec, get_setting/set_setting,
│   │   │                    #   get_sessions/get_session_transcript
│   │   ├── audio/
│   │   │   ├── mod.rs       # Re-exports capture and mic_vad modules
│   │   │   ├── capture.rs   # Dual capture (cpal + SCK), WAV writer, start_session/stop_session/
│   │   │   │                #   panic_mode/is_transcript_ready/get_log_dir_path cmds
│   │   │   └── mic_vad.rs   # MicVadState — VAD for Channel A (Shadow-mode hint cancellation)
│   │   ├── overlay.rs       # show_overlay Tauri command
│   │   ├── classifier/
│   │   │   └── mod.rs       # Question classification (heuristics + regex, bilingual EN/ES)
│   │   ├── orchestrator/
│   │   │   ├── mod.rs       # HintJob, HintScheduler, PanicState, hint generation
│   │   │   └── worker.rs    # hint worker thread
│   │   ├── stt/
│   │   │   ├── mod.rs       # STTEngine trait, persist_transcript_line, create_engine
│   │   │   ├── ffi.rs       # MoonshineFFIEngine (libmoonshine.dylib via libloading)
│   │   │   ├── cli.rs       # MoonshineCLIEngine (fallback)
│   │   │   ├── vad.rs       # SimpleVAD (energy-based)
│   │   │   ├── pipeline.rs  # STTPipeline (loopback→VAD→STT→classify→events→DB→HintJobs)
│   │   │   ├── provisioning.rs  # Moonshine auto-download (dylibs + model)
│   │   │   └── batch.rs     # Channel A batch transcription
│   │   ├── rag/
│   │   │   ├── mod.rs       # Re-exports
│   │   │   ├── embeddings.rs # BERT model (snowflake-arctic-embed-s, CPU+Accelerate)
│   │   │   └── indexer.rs   # Ingestion, chunking, indexing, vector search, PDF extraction
│   │   ├── analyze.rs       # Post-call BYOK analysis (Anthropic/OpenAI/Gemini/...)
│   │   ├── keys.rs          # Keychain API key storage (save_key/has_key Tauri commands)
│   │   ├── onboarding.rs    # First-run wizard (screen permission, model load, folder index)
│   │   ├── logging.rs       # File logger with rotation (keeps last 5 files)
│   │   ├── llm/
│   │   │   └── mod.rs       # Shared LLM response types (OpenAI, Anthropic, Gemini, etc.)
│   │   ├── tts/
│   │   │   └── mod.rs       # TTS via macOS `say` command (Samantha voice)
│   │   ├── interview_plan.rs # AI question plan generation via BYOK LLM
│   │   └── interview_runner.rs  # AI Interview orchestrator (TTS + event emission)
│   ├── vendor/
│   │   └── screencapturekit-rs/  # Vendored ScreenCaptureKit bindings
│   ├── Cargo.toml           # Rust dependencies
│   ├── tauri.conf.json      # Tauri v2 configuration (main + overlay windows, updater)
│   └── capabilities/
│       ├── default.json     # Main window permissions (core:default + shell:allow-open)
│       └── overlay.json     # Overlay permissions (core:default)
├── package.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
└── tsconfig.json
```
