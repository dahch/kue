# Kue — spec.md (v1)

> Codename: **Kue**.

> **Implementation status:** Sprints 0–3 completed, Sprint 4 (overlay) next. The DB schema is implemented (27 tests), the audio capture module (mic + loopback, ~1230 lines, 50 tests), the RAG engine (embeddings + indexer, 14+43=57 tests), the STT module (5 files, ~815 non-test lines, 84 tests), the classifier module (1 file, 48 tests), and the orchestrator module (2 files, ~400 non-test lines, 39 tests). The STT module integrates Moonshine via FFI (libmoonshine.dylib) with CLI fallback (`moonshine-voice`), simple VAD, pipeline in its own thread, `new-transcript` events to the frontend and persistence in `transcript_lines`. The STT pipeline is integrated into the app lifecycle via `toggle_audio_capture` command, which creates DB sessions, spawns the pipeline thread, and persists transcripts. The classifier receives each transcribed line (called from `STTPipeline::flush_segment`) and emits a `question-detected` event when a question is recognized. When a question is detected, the pipeline pushes a `HintJob` to the orchestrator module, which runs classifier + RAG search → hint text → immediate emit (Practice) or delayed emit (Shadow) via a dedicated worker thread (`kue-hint-worker`). Sections §3–§9 describe the complete planned product; see [`design.md`](./design.md) for what is actually built.

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

- Real-time transcription with speaker separation by audio channel.
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
| Secrets (API keys)  | Native OS keychain via `tauri-plugin-stronghold` or keyring        | Never plain text in the `settings` table.                                                                                                                                                                                            |

## 6. Module architecture

```text
┌──────────────────────────────────────────────────────────────────────┐
│                        TAURI SHELL (Rust)                             │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │   Audio Capture                                              │    │
│  │   - Channel A: Microphone (cpal) — your voice               │    │
│  │   - Channel B: System Loopback (ScreenCaptureKit)            │    │
│  │             — interviewer                                    │    │
│  └──────────┬──────────────────────────────────┬──────────────┘    │
│             ▼                                  ▼                    │
│  ┌──────────────────────┐         ┌───────────────────────┐        │
│  │ STT (Moonshine)      │         │ Local buffer (WAV)    │        │
│  │ Only channel B in    │         │ Channel A+B, for       │        │
│  │ real time            │         │ post-call              │        │
│  └──────┬───────────────┘         └───────────────────────┘        │
│         ▼ streaming text                                             │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Question Classifier (heuristics + regex)                     │   │
│  │  - Is it a question? (question mark + imperative verbs        │   │
│  │    like "cuéntame", "dime", "descríbeme")                     │   │
│  │  - Type: Technical / STAR / Architecture / Trick              │   │
│  └──────────┬─────────────────────────────────────────────────┘   │
│             ▼ if it's a question  (HintJob via mpsc channel)       │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │  Orchestrator (HintScheduler + hint worker thread)          │   │
│  │  ┌──────────────────────────────────────────────────────┐  │   │
│  │  │  1. Classifies question type (Technical/STAR/etc.)   │  │   │
│  │  │  2. Queries RAG (top_k=1, tag/metric if available)   │  │   │
│  │  │  3. Builds hint: "💡 {tag}: {metric}" (≤8 words)    │  │   │
│  │  │     or generic: "💡 Usa STAR: Situación, Tarea..."    │  │   │
│  │  │  4a. Practice → emit "new-hint" event immediately    │  │   │
│  │  │  4b. Shadow → schedule via HintScheduler, emit        │  │   │
│  │  │     after 2.5s delay if not cancelled                │  │   │
│  │  └──────────────────────────────────────────────────────┘  │   │
│  └──────────┬─────────────────────────────────────────────────┘   │
│             ▼ Tauri event ("new-hint")                              │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │  Overlay (Tauri window)                                       │ │
│  │  - always_on_top, click-through, semi-transparent             │ │
│  │  - positionable, Panic button (silences hints)                │ │
│  └──────────────────────────────────────────────────────────────┘ │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐ │
│  │  SQLite (single source of truth)                              │ │
│  │  transcripts · docs (vectors) · sessions · settings            │ │
│  └──────────────────────────────────────────────────────────────┘ │
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
  - **Trap:** weakness, failure, worst, mistake, "what would you do differently", "why should we hire you", etc.
- **Experience question heuristic:** If the text matches "cuéntame", "tell me about", "walk me through" (without "code"), "dime", etc., it defaults to STAR.
- **Fallback:** Questions without any keyword match default to Technical.
- **Tie-breaking:** Trap > Architecture > STAR > Technical when multiple categories have the same score.
- **Bilingual:** All keyword lists and imperative triggers cover both Spanish and English.

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

1. **Initial setup:** the user uploads CV/projects, the app indexes (candle generates embeddings → sqlite-vec). If they want post-call, they configure their API key (stored in keychain, not in `settings`).
2. **During the interview:** chooses Practice or Shadow. Dual audio capture starts. Moonshine transcribes channel B. The classifier detects questions and triggers RAG. The overlay shows the hint for ~3s and disappears (immediate in Practice, after 2.5s of stalling in Shadow).
3. **Post-call:** full transcript is saved. "Analyze" button sends transcript + relevant context to the chosen LLM (BYOK) → summary, weak questions, unmentioned projects, STAR structure improvements.

## 10. Development plan (MVP)

| Sprint | Objective           | Deliverables                                                                                                                                                                                                                                                                                                                                                                              | Status           |
| ------ | ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| 0      | Base infrastructure | Tauri + React project. Rust dependencies (`cpal`, `screencapturekit-rs`, `tauri`, `rusqlite`+`sqlite-vec`, `candle`). Complete SQLite schema (sessions, transcript_lines, documents, chunks, chunks_vec, settings) with migrations. sqlite-vec registered. Dual audio capture (mic cpal + loopback SCK) with WAV writing. `get_db_status` and `toggle_audio_capture` commands. 48+ tests. | ✅ **Completed** |
| 1      | STT (Moonshine)     | Moonshine integration on channel B. STTEngine trait + FFI engine (`libmoonshine.dylib`) + CLI fallback (`moonshine-voice transcribe`). Simple VAD (energy-based). Pipeline thread that receives audio from loopback, segments by VAD, transcribes, emits `new-transcript` event and persists in `transcript_lines`.                                                                       | ✅ **Completed** |
| 2      | RAG Engine          | Local document indexing. sqlite-vec + candle generating and searching embeddings. Goal: query <20ms.                                                                                                                                                                                                                                                                                      | ✅ **Completed** |
| 3 | Classifier & hints | Rules from §7 in Rust. `QuestionType` enum (Technical/STAR/Architecture/Trap/None) with heuristic + keyword-density scoring. `classify_text` Tauri command registered in invoke_handler. Classifier wired into STT pipeline — each transcribed line is classified and emits `question-detected` event (see `stt/pipeline.rs`). Orchestrator module (`orchestrator/mod.rs` + `orchestrator/worker.rs`) integrates classifier + RAG to produce hints: RAG search (top_k=1), `💡 {tag}: {metric}` format (max 8 words), generic fallback per type. Dedicated hint worker thread (`kue-hint-worker`) processes HintJobs via mpsc channel, uses `HintScheduler` for Shadow mode's 2.5s delay, and emits `new-hint` events via Tauri. | ✅ **Completed** (48 classifier tests + 39 orchestrator tests) |
| 4      | Overlay & UI        | Transparent window, always-on-top, click-through. Practice vs Shadow. Panic button.                                                                                                                                                                                                                                                                                                       | ⬜ Not started   |
| 5      | Post-call & BYOK    | SQLite export/query. External API call. Secure key storage in keychain. Analysis saving.                                                                                                                                                                                                                                                                                                  | ⬜ Not started   |

## 11. Open Questions / risks

- **Legality of recording without explicit consent from the other party** — review for Spain at minimum before this becomes a regular habit. Partially mitigated by ADR-011 (audio doesn't persist by default), but the underlying legal question remains open for when the user enables retention or for the text transcript itself.
- ~~Own benchmark between `all-MiniLM-L6-v2` and `snowflake-arctic-embed-s`~~ — Resolved (ADR-012): chose snowflake-arctic-embed-s based on public retrieval benchmarks (MTEB/BEIR published by Snowflake), not an own test on the user's documents. Same 384 dims as MiniLM, no impact on the schema.
- ~~Chunk size / overlap for context RAG~~ — Resolved (ADR-013): `CHUNK_SIZE=150` words, `CHUNK_OVERLAP=20` words, empirically validated against the 512-token limit of `snowflake-arctic-embed-s` (BERT WordPiece) — see `test_chunk_size_fits_in_model_context` in `rag/indexer.rs`.
- ScreenCaptureKit stability on macOS <14 — there are reports of intermittent segfaults on older versions; validate the minimum supported version before committing to it in onboarding.
- 2.5s threshold in Shadow — empirically validate that it doesn't feel too anxious nor too late.
