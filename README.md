# Kue

> Memory copilot for technical interviews — helps you recall your own metrics, projects, and structure without generating answers for you.

Desktop application (macOS, Tauri v2) with real-time transcription, local RAG over your CV/projects, and ultra-short hints to maintain fluency under pressure. Post-call, optional analysis with your own LLM (BYOK).

**Current status:** Sprint 0–2 completed — base infrastructure (Tauri + SQLite/sqlite-vec), dual audio capture (microphone via `cpal` + system loopback via ScreenCaptureKit), RAG engine (embeddings with `candle` + vector search with `sqlite-vec`), and STT module (Moonshine FFI + CLI fallback + VAD + pipeline) implemented and tested (+200 tests in Rust). The STT module is complete and tested but not yet integrated into the app lifecycle. The rest of the pipeline (classifier → hints → overlay) is in planning (see `spec.md`).

---

## Prerrequisitos

- **Rust** 1.79+ (`rustup install 1.79`)
- **Node.js** 18+ (recomendado 20 LTS)
- **macOS 13+** (ScreenCaptureKit requerido para captura de audio de sistema)

## Getting Started

```bash
# Clonar el repositorio
git clone <repo-url> && cd kue

# Instalar dependencias del frontend
npm install

# Iniciar en modo desarrollo (Tauri + Vite)
npm run tauri:dev
```

The app will open a window with debug controls to index folders (RAG) and search context. The Rust backend connects to SQLite and creates the schema at `~/Library/Application Support/com.kue.app/kue.db`.

## Tests

```bash
# Rust tests (database — all logic implemented)
npm run test:rust:db

# Rust tests (all modules, >200 tests)
npm run test:rust

# Rust coverage (requires cargo-tarpaulin)
npm run coverage:rust:db
npm run coverage:rust:full
```

## Scripts disponibles

| Command | Description |
|---|---|
| `npm run dev` | Standalone Vite server (without Tauri) |
| `npm run build` | Frontend build (TypeScript + Vite) |
| `npm run preview` | Preview of frontend build |
| `npm run tauri` | Tauri CLI directly |
| `npm run tauri:dev` | Tauri + Vite in development mode |
| `npm run tauri:build` | Production build (generates .dmg) |
| `npm run test:rust:db` | Tests for the database module only |
| `npm run test:rust` | Tests for all Rust modules (>200 tests) |
| `npm run coverage:rust` | Runs Rust tests (alias for `test:rust`) |
| `npm run coverage:rust:check` | Checks availability of coverage tools |
| `npm run coverage:rust:db` | Coverage for the database module (tarpaulin) |
| `npm run coverage:rust:full` | Full Rust coverage (tarpaulin) |
| `npm run coverage:rust:text` | Coverage on stdout (tarpaulin) |

## Architecture (high level)

```
┌──────────────────────────────────────────────────────┐
│                 Tauri v2 Shell                        │
│  ┌──────────┐   ┌───────────────────────────────┐    │
│  │ Frontend │   │  Rust Backend                  │    │
│  │ (React + │◄──│  - db::init_db                │    │
│  │ Tailwind)│   │  - db::get_db_status cmd      │    │
│  │          │   │  - audio::toggle_audio_capture│    │
│  │          │   │  - rag::index_folder_cmd     │    │
│  │          │   │  - rag::search_context cmd   │    │
│  │          │   │  - stt module (tested, not   │    │
│  │          │   │    wired in lifecycle)       │    │
│  └──────────┘   │  - types (TranscriptLine,     │    │
│                 │    Speaker)                    │    │
│                 │  - cpal (mic capture)          │    │
│                 │  - SCK (loopback)              │    │
│                 │  - hound (WAV writer)          │    │
│                 │  ┌─────────────────────────┐   │    │
│                 │  │  RAG Engine              │   │    │
│                 │  │  - rag::embeddings       │   │    │
│                 │  │    (candle BERT, 384-d)  │   │    │
│                 │  │  - rag::indexer          │   │    │
│                 │  │    (ingest / search /    │   │    │
│                 │  │     chunk / folder)      │   │    │
│                 │  └─────────────────────────┘   │    │
│                 │  ┌─────────────────────────┐   │    │
│                 │  │  STT Module              │   │    │
│                 │  │  - stt::ffi             │   │    │
│                 │  │    (Moonshine FFI)      │   │    │
│                 │  │  - stt::cli (fallback)  │   │    │
│                 │  │  - stt::vad (energy)    │   │    │
│                 │  │  - stt::pipeline        │   │    │
│                 │  │    (VAD→STT→events→DB)  │   │    │
│                 │  └─────────────────────────┘   │    │
│                 └───────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │  SQLite + sqlite-vec                          │    │
│  │  (sessions · transcript_lines · documents ·   │    │
│  │   chunks · chunks_vec · settings)              │    │
│  └──────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────┘
```

**Legend:** All listed code is functional and tested. `candle` implements BERT embeddings (`snowflake-arctic-embed-s`) in the `rag::embeddings` module, and `sqlite-vec` performs KNN vector search. The STT module is implemented (FFI + CLI + VAD + pipeline thread) but not yet integrated into the app lifecycle — see `design.md` for details.

## Stack

| Layer | Technology |
|---|---|
| Frontend | React 18 + TypeScript + Tailwind CSS 3 |
| App Core | Rust (Tauri v2) |
| Database | SQLite + sqlite-vec (vectors) |
| Audio | cpal (mic) + screencapturekit-rs (loopback) + hound (WAV) |
| STT | Moonshine (FFI + CLI fallback, not integrated in lifecycle) |
| Embeddings | candle (HuggingFace Rust) + `snowflake-arctic-embed-s` |
| Post-call (planned) | BYOK (Anthropic/OpenAI/Ollama/etc.) |

## Related documentation

- [`spec.md`](./spec.md) — Complete functional specification of the product
- [`design.md`](./design.md) — Technical design and current architecture
- [`adr.md`](./adr.md) — Architecture decision records

## Project

```
kue/
├── src/                  # Frontend React + TypeScript
│   ├── App.tsx           # Debug UI: RAG indexing and vector search
│   ├── main.tsx          # Entry point
│   └── index.css         # Tailwind directives
├── src-tauri/            # Rust backend (Tauri)
│   ├── src/
│   │   ├── main.rs       # Entry point
│   │   ├── lib.rs        # Tauri builder + setup (registers DB, AudioCapture, RAG, cleans orphan temp dirs)
│   │   ├── types.rs      # TranscriptLine, Speaker (STT → classifier contract)
│   │   ├── db/
│   │   │   └── mod.rs    # Schema, migrations, sqlite-vec, tests
│   │   ├── audio/
│   │   │   ├── mod.rs    # Re-export of the capture module
│   │   │   └── capture.rs # Dual capture (cpal + SCK), WAV writer, toggle_audio_capture cmd
│   │   ├── stt/
│   │   │   ├── mod.rs    # STTEngine trait, STTConfig, re-exports
│   │   │   ├── ffi.rs    # MoonshineFFIEngine (libmoonshine.dylib via libloading)
│   │   │   ├── cli.rs    # MoonshineCLIEngine (fallback to moonshine-voice CLI)
│   │   │   ├── vad.rs    # SimpleVAD (energy-based)
│   │   │   └── pipeline.rs # STTPipeline (thread loopback→VAD→STT→events→DB)
│   │   └── rag/
│   │       ├── mod.rs    # Re-export of embeddings and indexer
│   │       ├── embeddings.rs # BERT model (snowflake-arctic-embed-s), embedding generation
│   │       └── indexer.rs    # Ingestion, chunking, indexing, and vector search
│   ├── Cargo.toml        # Rust dependencies
│   ├── tauri.conf.json   # Tauri v2 configuration
│   └── capabilities/     # Tauri permissions (core, shell)
├── package.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
└── tsconfig.json
```
