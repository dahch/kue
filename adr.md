# Kue — Architecture Decision Records

---

### ADR-001: Native desktop architecture via Tauri (Rust + React)

**Context:** We need to capture system audio (loopback) and microphone, and display an overlay that doesn't interfere with video call screen-sharing. Both require OS-level access that a browser doesn't expose.

**Decision:** Tauri, Rust backend, React/TypeScript frontend. Reuses knowledge already acquired in [[musicsync]].

**Consequences:**
- *(Positive)* Direct access to low-level audio APIs. Native windows with `always_on_top` and transparency. ~15-20MB binary, minimal RAM. Audio and ML (Moonshine, candle) run in native threads without blocking the UI.
- *(Negative)* Learning curve in Rust and packaging.

**Alternatives considered:** Electron (larger footprint, worse access to low-level audio). Pure web (impossible to capture loopback).

---

### ADR-002: macOS-only for v1

**Context:** Windows (WASAPI) and macOS (Core Audio/ScreenCaptureKit) require completely different audio capture implementations. Covering both in v1 multiplies the effort before validating whether the product works.

**Decision:** v1 is built and validated on macOS only. Windows/Linux remain evaluated for v2 based on v1 results.

**Consequences:** The audio capture layer must be designed with a clear interface (trait in Rust) that allows adding Windows/Linux backends later without rewriting the rest of the pipeline.

**Alternatives considered:** Multi-OS support from v1 (disproportionate effort before validating the concept).

---

### ADR-003: Loopback capture via ScreenCaptureKit, without virtual drivers

**Context:** We need to capture system output audio (interviewer's voice) on macOS. The traditional solution (BlackHole or other virtual audio driver) requires the user to install and configure a "Multi-Output" device before using the app, which can also disable volume keys and fail when switching to Bluetooth headphones.

**Decision:** Use ScreenCaptureKit (macOS 13+) via the `screencapturekit-rs` crate for system loopback, without third-party drivers. The microphone is captured separately with `cpal`.

**Consequences:**
- *(Positive)* Zero installation friction for the user; audio is routed at the system level without touching the user's audio configuration.
- *(Negative)* Requires explicit user permission in Settings → Privacy → Screen & System Audio Recording (no signing entitlement can bypass it). There are reports of intermittent segfaults on macOS versions prior to 14 — pending validation of the minimum supported version.

**Alternatives considered:** BlackHole/Loopback (third-party virtual driver, high installation friction, risk of breaking user's audio configuration).

---

### ADR-004: Diarization via separate audio channels

**Context:** We need to distinguish what the interviewer says from what the candidate says, without relying on expensive or unreliable real-time acoustic diarization models.

**Decision:** Capture two native streams: user's microphone (Channel A, via `cpal`) and system loopback (Channel B, via ScreenCaptureKit). STT is only applied to Channel B.

**Consequences:** Free and reliable speaker labeling. Computation savings by transcribing only the relevant channel. Moonshine's `identify_speakers` remains as a fallback if the mic picks up both voices (e.g., speakers without headphones).

**Alternatives considered:** Pure acoustic diarization (less reliable as a primary mechanism), mixed audio capture + AI separation (unreliable in real time).

---

### ADR-005: Moonshine for STT, instead of Whisper or cloud STT

**Context:** We need real-time transcription, low latency, and offline operation — interview audio must not leave the machine.

**Decision:** Moonshine (Medium), running locally via its C++ core with a C interface bindable from Rust.

**Consequences:**
- *(Positive)* Latency <260ms. No API cost. Total privacy of interviewer's audio. ~5x faster than equivalent Whisper.
- *(Negative)* CPU usage (~1-2 cores). ~300MB model to package or download during onboarding.

**Alternatives considered:** Deepgram/AssemblyAI (paid, internet-dependent). Whisper.cpp (slower and heavier for real time). Parakeet (no native diarization, runs as a separate HTTP server).

---

### ADR-006: sqlite-vec + candle for context RAG, instead of zvec

**Context:** Own context (CV, projects, stories) is small in volume (thousands of chunks, not millions). We need to index and search it quickly (<20ms) within a single binary, without external processes or network dependencies.

**Decision:** sqlite-vec (SQLite extension for vectors) + candle (HuggingFace ML framework in Rust) for local embeddings. Everything lives in the same SQLite file as transcripts/sessions/settings.

**Consequences:**
- *(Positive)* A single `.db` file for the entire app — trivial backup and portability. Zero background processes. Hybrid queries (text + vector) in a single SQL query.
- *(Negative)* sqlite-vec is still in alpha (possible breaking changes between minor releases), accepted as a minor risk since at this scale brute force search is sufficient. candle requires initial download of the embedding model (~90MB for snowflake-arctic-embed-s).

**Alternatives considered:** zvec (more mature dedicated ANN engine, but requires a separate storage engine). LEANN (very complete ingestion pipeline, but 100% Python ecosystem, would require a sidecar). LanceDB/Qdrant/Milvus (require a server or complicate unification with the transcripts DB).

---

### ADR-007: 5-8 word hints, never full answers

**Context:** Tools like Cluely generate full answers in real time, which is noticeable (robotic cadence, delays) and shifts the problem to the first day on the job if the user is hired relying on it. We want to help without breaking the natural conversation flow or crossing into "cheating."

**Decision:** The classifier/hint engine never generates an answer. It only extracts a metric, project name, or structure tag from the user's own context, displayed in a brief format ("💡 {tag}: {metric}", max 8 words).

**Consequences:** The user constructs the sentence mentally, maintaining authenticity and their own voice. The interface doesn't require reading paragraphs, just a glance. The boundary between "recalling your own data" and "letting the model argue for you" stands as a design principle for the entire hint engine.

**Alternatives considered:** Full answer generation with LLM in real time (high latency, risk of detection, and the ethical problem that motivated this design from the start).

---

### ADR-008: Practice and Shadow modes

**Context:** The same hint mechanism doesn't work equally for practice and real interviews: in a real interview, showing hints all the time would indeed be a form of cheating; in practice, it helps for them to be generous and instructive.

**Decision:** Two explicit modes. **Practice**: immediate hints with structure explanation. **Shadow**: hints only if the user has more than 2.5s of silence after the question (genuine block), and without additional explanation.

**Consequences:** The product design itself reinforces the ethical boundary — in real mode, the system by default doesn't intervene unless there's evidence the user needs it. The 2.5s threshold remains as a parameter to validate empirically (see spec.md, Open Questions).

**Alternatives considered:** A single mode with the same behavior in both contexts (doesn't distinguish the acceptable level of intervention between practicing and a real interview).

---

### ADR-009: Unified storage — SQLite as source of truth

**Context:** We need to store transcripts with timestamps, session metadata, and embedding vectors in a single portable location.

**Decision:** SQLite as the single engine, with tables for `sessions`, `transcript_lines`, `documents`, `chunks` and `chunks_vec` (via sqlite-vec).

**Consequences:** Database as a single file, easy to copy/backup/delete. Familiar SQL queries. Hybrid text+vector operations in a single query.

**Alternatives considered:** Separate storage engines for vectors vs. relational data (see ADR-006).

---

### ADR-010: Post-call analysis with BYOK (bring your own key)

**Context:** Post-call analysis has no latency constraints, so it can use any large model. Forcing a proprietary provider implies infrastructure cost for the developer and less privacy control for the user.

**Decision:** The user provides their own API key (Anthropic, OpenAI, Gemini, OpenRouter) or points to a local/OpenAI-compatible endpoint (Ollama, vLLM, MLX). Same "pluggable AI providers" pattern already used in [[jobmatch-ai]]. The key is stored in the OS native keychain, never in plain text in the `settings` table.

**Consequences:**
- *(Positive)* Zero AI infrastructure costs for the developer. The user chooses their balance between cost and privacy (can go 100% local with Ollama).
- *(Negative)* Initial friction — the user must obtain and configure their own key.

**Alternatives considered:** Providing a proprietary model managed by the app (infrastructure cost, less privacy for the user).

---

### ADR-011: Raw audio doesn't persist by default — only the text transcript

**Context:** Dual capture writes WAVs of both channels for STT to consume in streaming. Persisting those WAVs indefinitely on disk changes the product's risk profile: it goes from "I save a text transcript" to "I permanently save the interviewer's actual voice," which directly ties into the open legal question about recording consent (spec.md §11) — persisting by default unnecessarily worsens it before resolving it.

**Decision:** The WAV is written to a session temp directory (`std::env::temp_dir()/kue-session-{timestamp}/`), consumed in streaming by Moonshine. At session end (`stop_session`), the WAV is automatically deleted unless the user has explicitly enabled `settings.retain_audio` (opt-in, default `false`). The text transcript (`transcript_lines`) always persists — it's the real basis for post-call.

**Consequences:**
- *(Positive)* Reduces legal/privacy risk by default without waiting to resolve the consent question. The default behavior is the most conservative.
- *(Negative)* If the user wants to debug STT quality on real audio, they must explicitly enable retention — minor friction, accepted in exchange for risk reduction.

**Alternatives considered:** Default WAV persistence (what the code had before this correction) — unnecessary risk before resolving recording legality.

---

### ADR-012: `snowflake-arctic-embed-s` for embeddings, instead of `all-MiniLM-L6-v2`

**Context:** We need an embedding model for RAG that is small, fast on CPU/Metal, generates 384-dimensional vectors (compatible with sqlite-vec), and has good retrieval performance on technical documents (CVs, project descriptions).

**Decision:** `snowflake-arctic-embed-s`, same 384-d scheme as `all-MiniLM-L6-v2` (doesn't require schema changes to `chunks_vec`), but with better reported performance on public MTEB/BEIR benchmarks. Loaded via `candle` + `hf-hub`.

**Consequences:**
- *(Positive)* No schema changes (384-d). Better retrieval on benchmarks. ~90MB model, downloadable once during onboarding.
- *(Negative)* No own benchmark was run on the user's documents — the decision is based on public benchmarks. If the CV/project domain turns out to be very different, there could be a performance gap.

**Alternatives considered:** `all-MiniLM-L6-v2` (same vector size, lower MTEB/BEIR score). Models >768-d like `e5-large` (incompatible with current schema without migration). Multilingual models (not needed for v1 in technical Spanish/English).

---

### ADR-013: Word-based chunking with CHUNK_SIZE=150 and CHUNK_OVERLAP=20

**Context:** Documents to index (CV, projects in Markdown/PDF/TXT) need to be split into fragments for RAG. Chunk size directly impacts retrieval quality and the ability to stay within the 512-token limit of BERT WordPiece (tokenizer of `snowflake-arctic-embed-s`).

**Decision:** Word-count chunking (not token-based) with `CHUNK_SIZE=150` words and `CHUNK_OVERLAP=20` words between adjacent fragments. Empirically validated through an integration test that verifies no chunk exceeds 512 tokens when tokenized (see `test_chunk_size_fits_in_model_context` in `rag/indexer.rs`).

**Consequences:**
- *(Positive)* Simple and predictable chunking, no dependency on the tokenizer at indexing time (only in validation). 512 BERT WordPiece tokens ~ 380 English words — 150 words per chunk with overlap provides ample margin.
- *(Negative)* Very long words (e.g., German compounds, code with long identifiers) could approach the limit. Chunking doesn't respect paragraph/section boundaries — a chunk may cut an idea in half.

**Alternatives considered:** Token-based chunking (more precise but requires the tokenizer in the indexing hot path, adding latency and complexity). Paragraph-based chunking (variable size, hard to control). No overlap (loses context between fragments).
