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

---

### ADR-014: Dedicated hint worker thread with poll-based scheduler for delayed hints

**Date:** 2026-07-26

**Status:** Accepted

**Context:** The hint engine needs to:
- Accept hint jobs from the real-time STT pipeline without blocking it (the pipeline runs in a tight audio loop — any delay in hint generation would cause audio drops).
- Support two emission modes: **Practice** (emit the hint immediately after classification + RAG) and **Shadow** (emit only after a 2.5s delay, simulating "the user is stuck").
- Cancel pending hints if the session ends before the delay expires (otherwise stale hints would appear in a subsequent session).
- Query the RAG index (SQLite + candle embedding) for each hint, which can take 5–20ms.

**Decision:** A three-part architecture:

1. **Worker thread + mpsc channel:** The STT pipeline sends `HintCommand::Process(HintJob)` messages over an `std::sync::mpsc::Sender` (wrapped in `Arc` as `HintJobSender`). A dedicated `kue-hint-worker` thread receives them. This decouples the audio-hot path from the hint generation path (RAG query).

2. **Poll-based scheduler (`HintScheduler`):** Shadow mode hints are stored in a `Vec<PendingHint>` with a `fire_at: Instant` deadline. Rather than spawning one timer per hint, the worker thread polls the scheduler every 500ms via `tick(now)` which returns all hints whose deadline has passed. This avoids the complexity of per-hint timers and makes cancellation trivial (just remove from the vector).

3. **Cancel-on-session-end:** When the STT pipeline thread exits (session stops), it sends `HintCommand::CancelSession(session_id)`. The worker calls `scheduler.cancel_all(session_id)`, which removes all pending hints for that session. This prevents stale hint emission.

**Consequences:**
- *(Positive)* The audio pipeline is never blocked by RAG queries or hint formatting.
- *(Positive)* Shadow mode hint delay is simple and testable — it's just a time comparison in a vector, not a timer interface.
- *(Positive)* Cancellation is O(n) in the number of pending hints — acceptable since at most ~20 questions are expected per session.
- *(Positive)* 39 unit tests cover scheduler timing, cancellation, multi-session isolation, hint formatting variants, and generic fallbacks — all without a real Tauri runtime.
- *(Negative)* Polling every 500ms means a hint can fire up to 500ms late in Shadow mode (2.5s → ~3.0s). This was deemed acceptable — it's within human perception tolerance for a "stuck" delay.
- *(Negative)* The worker thread adds ~200KB of fixed overhead, negligible on a modern system.

**Alternatives considered:**
- **Inline generation in the pipeline thread** (simpler but blocks the audio loop for 5–20ms per RAG query — risk of audio buffer underruns at high question frequency).
- **Per-hint timers (`std::thread::sleep` + `Instant`)** in the pipeline or separate threads (scales poorly with question count; cancellation requires complex `Arc<AtomicBool>` flags; testing requires real time).
- **`tokio::spawn` + `tokio::time::delay`** (would introduce async runtime dependency just for this feature; increases compilation time and binary size).

---

### ADR-015: Channel A is batch-transcribed at session end, not in streaming

**Date:** 2026-07-26

**Status:** Accepted

**Context:** ADR-004 correctly decided that real-time STT only applies to Channel B, because no component of the hint pipeline (classifier, orchestrator, overlay) needs to transcribe the user's voice. But that decision left `Speaker::User` as dead code — the final transcript never captures the user's answers, which prevents post-call analysis (Sprint 5) from evaluating actual response quality, since it only records the interviewer's questions.

**Decision:** At session end (`stop_session`), before applying the audio retention policy (ADR-011), a batch transcription of the Channel A WAV runs using the same Moonshine engine and `SimpleVAD` already used for Channel B, but without any connection to the classifier or orchestrator — only VAD segmentation + transcription + persistence in `transcript_lines` with `speaker='user'`. There is no latency constraint because the session is already over.

The transcription runs in a separate thread (`kue-batch-transcribe`) so as not to block the Tauri `stop_session` command. The batch thread takes ownership of the temp `session_dir` and applies the retention policy (ADR-011) on completion — ensuring the WAV is not deleted before it is transcribed. On completion, it emits a `post-call-transcript-ready` event with the `session_id` so the frontend can reflect the state if needed.

**Consequences:**
- *(Positive)* The complete transcript (both speakers) is available for post-call, fulfilling what spec §2 promises. No additional CPU cost during the live call — the highest-load moment (STT + RAG + overlay running live) is not affected.
- *(Negative)* The user response transcript is not available until batch processing finishes (a few seconds after session end, depending on duration) — it's not instant like Channel B.
- *(Negative)* Reading the entire WAV into memory for VAD segmentation can consume several MB for long sessions — acceptable because it happens in a separate post-session thread, not during live capture.

**Alternatives considered:**
- **Live streaming of Channel A** in parallel with Channel B (doubles CPU/Moonshine usage during the live call, with no functional benefit since nothing consumes that transcription in real time).
- **Keep the current scope** without transcribing Channel A (rejected because it would prevent post-call from fulfilling its purpose — without user answers the LLM cannot evaluate STAR structure or response quality).

---

### ADR-016: Post-call BYOK analysis with keychain-stored API keys

**Date:** 2026-07-26

**Status:** Accepted

**Context:** Sprint 5 needed to implement post-call analysis of the full transcript (both speakers). The requirement was:
- Allow the user to send the transcript + relevant RAG context to an LLM of their choice.
- Support multiple providers (Anthropic, OpenAI, Gemini, OpenRouter, Ollama) without vendor lock-in.
- Keep the user's API key secure — never stored in the app's SQLite database or written to disk in plain text.
- The analysis should be structured (summary, weak questions, missed projects, STAR improvements) and returned in a parseable JSON format.
- The batch transcription of Channel A (ADR-015) completes asynchronously, so the analysis must wait for it to finish.

**Decision:**

1. **Standalone Tauri command (`analyze_session`):** A single `#[tauri::command]` that receives a `session_id`, `provider` string, and optional `model` string. It reads the transcript from SQLite, queries RAG for relevant context, builds a prompt, makes an HTTP request (`reqwest`), parses the JSON response, and returns an `AnalyzeResult`. This runs in the Tauri command thread (not the audio/orchestrator thread) since there is no real-time constraint.

2. **OS Keychain storage via `keyring`:** API keys are stored in the macOS Keychain using the `keyring` crate, identified by service name `"kue"` and the provider name as the user identifier. The `save_key` and `has_key` Tauri commands expose this to the frontend. Keys are never stored in the `settings` table, and a test (`key_never_in_settings_table`) explicitly verifies this.

3. **Structured prompt with JSON schema:** The `build_analysis_prompt` function constructs a Spanish-language prompt that demands a specific JSON schema (`summary`, `weak_questions`, `forgotten_projects`, `star_improvements`). The response parser handles both raw JSON and JSON embedded in markdown code blocks.

4. **Provider-specific HTTP adapters:** Each provider (Anthropic, OpenAI, Gemini, OpenRouter, Ollama) has its own URL builder and header format. The `reqwest` client makes POST requests with the appropriate `Authorization` header (or `x-goog-api-key` for Gemini).

5. **Batch transcription completion gating:** The `analyze_session` command checks `BatchTracker` (a `HashSet` of completed session IDs registered as Tauri state) before proceeding. If the session's Channel A transcription hasn't finished, the command returns an error. The frontend checks `is_transcript_ready` and listens for `post-call-transcript-ready` before enabling the "Analyze" button.

**Consequences:**
- *(Positive)* Zero infrastructure cost for the developer — the user brings their own key and pays their own API usage.
- *(Positive)* Maximum privacy flexibility — the user can use a local Ollama instance for fully offline analysis.
- *(Positive)* No impact on real-time performance — analysis runs in its own command handler, not in the audio/orchestrator threads.
- *(Positive)* Keys are stored in the OS keychain, not in the app's SQLite database, matching best practices for credential storage.
- *(Negative)* Initial friction — the user must obtain and configure their own API key. Mitigated by the `ApiKeyInput` component which shows a clear UI for each provider.
- *(Negative)* Provider-specific HTTP adapters require maintenance if APIs change. Mitigated by keeping the adapter logic in a single file (`analyze.rs`) with per-provider functions.
- *(Negative)* The LLM response must be valid JSON — providers that struggle with structured output may return unparseable responses. Mitigated by the markdown-aware JSON parser and clear error messages.

**Alternatives considered:**
- **Built-in LLM provider managed by the app** (rejected — would require the developer to pay for API costs and manage a billing system).
- **Store keys in the `settings` table** (rejected — keys in plain text in SQLite is a security risk).
- **Post-call analysis via Tauri plugin/shell** (rejected — `reqwest` is simpler and more reliable than subprocess invocation).
- **Analyze in the batch transcription thread** (rejected — mixing concerns; the batch thread's job is to produce transcripts, not consume them).

---

### ADR-017: Moonshine auto-provisioning on first launch

**Date:** 2026-07-26

**Status:** Accepted

**Context:** Moonshine STT requires two external resources that are not bundled with the binary:
1. **dylibs:** `libmoonshine.dylib` (~27 MB) + `libonnxruntime.1.23.2.dylib` (~26 MB), shipped inside a PyPI wheel (`moonshine-voice` v0.0.73).
2. **Model files:** 8 ONNX/config files for the Medium Streaming English model (~429 MB total), hosted on `download.moonshine.ai`.

Previously (Sprints 1–5), the user had to manually download and place these files at `~/.local/share/moonshine/` or `~/moonshine-models/`. This created a poor onboarding experience: first use required following external instructions, the user saw Moonshine-related errors on first launch, and troubleshooting required knowing the filesystem layout.

Sprint 6 needed to eliminate this friction while ensuring:
- The download runs in a background thread — it must not block the Tauri setup or UI startup.
- Progress is reported so the frontend can display a download indicator.
- Integrity verification is performed (SHA-256) to detect corrupted downloads or CDN compromise.
- Retry is possible if the download fails (network issues, temporary server errors).
- Idempotency — re-launching the app after a partial download should not re-download intact files.
- Offline resilience — if there's no internet, the app should not crash; it should degrade gracefully (falling back to CLI engine if available, or showing a clear error).

**Decision:** A dedicated provisioning module (`stt/provisioning.rs`) that:

1. **Runs in a background thread** (`kue-moonshine-provision`) spawned from `lib.rs::setup()` via `ensure_moonshine_installed()`. The thread downloads and verifies all resources while the UI is already responsive.

2. **Downloads dylibs from PyPI:** Fetches the `moonshine_voice` v0.0.73 wheel (SHA-256 pinned at build time), verifies the hash, then extracts `libmoonshine.dylib` and `libonnxruntime.1.23.2.dylib` via the `zip` crate into `{app_data_dir}/moonshine/lib/`.

3. **Downloads the model from `download.moonshine.ai`:** Fetches 8 files (adapter.ort, cross_kv.ort, decoder_kv.ort, decoder_kv_with_attention.ort, encoder.ort, frontend.ort, streaming_config.json, tokenizer.bin) into `{app_data_dir}/moonshine/models/en/medium-streaming/`. Each file's size is checked against expected values (±10%), and its SHA-256 hash is verified.

4. **Progress reporting:** Emits `moonshine-download-progress` Tauri events with stage, file index/count, and downloaded/total bytes, throttled to at most one every 250ms.

5. **Completion signalling:** Emits `moonshine-provisioned` on success or `moonshine-provision-error` on failure.

 6. **Status check command:** `is_moonshine_provisioned` Tauri command lets the frontend check whether provisioning is already complete (used by `ProvisioningProgress.tsx` on mount to skip the download UI if already provisioned).
 7. **Retry command:** `retry_moonshine_download` Tauri command removes partial files and re-launches provisioning.

 8. **Path resolution:** The global `MOONSHINE_BASE` static is set after successful provisioning. The FFI engine's `is_available()` and the `STTConfig::default_model_path()` both check this managed path first, then fall back to dev paths (`~/.local/share/moonshine/`, `~/moonshine-models/`, relative path), ensuring backward compatibility.

 9. **DYLD_LIBRARY_PATH:** The managed lib dir is prepended to `DYLD_LIBRARY_PATH` during `setup()` so that `@rpath/libonnxruntime.*.dylib` is resolvable at load time. This is safe because no other threads have been spawned yet.

**Consequences:**
- *(Positive)* First-launch friction is eliminated — the user doesn't need to manually download or configure anything.
- *(Positive)* Integrity verification (SHA-256 + size checks) protects against corrupted downloads and CDN compromise (with the caveat that model hashes were pinned at build time from a one-time download — see the code comments).
- *(Positive)* Idempotent — partial downloads are detected by size/hash and only the missing/corrupt files are re-downloaded.
- *(Positive)* The background thread doesn't block Tauri setup or UI startup.
- *(Positive)* The frontend `ProvisioningProgress` component (`src/ProvisioningProgress.tsx`) provides a polished download UX with a real progress bar, stage label, file counter, and retry button — the download no longer runs silently.
- *(Negative)* The download is large (~482 MB total) and requires internet on first launch.
- *(Negative)* PyPI wheel SHA-256 must be updated when the moonshine-voice version is bumped. The hash is a compile-time constant in `provisioning.rs`.
- *(Negative)* The model hash pins are not vendor-published — they were computed at build time and protect against *future* CDN compromise but not against already-tampered files at the time of pinning. Documented as a trust caveat in the code.
- *(Negative)* Functions requiring a concrete `tauri::AppHandle` (the download, progress emission, and retry logic) cannot be unit tested — only the pure helper functions (SHA-256, size validation, ZIP extraction dylib detection) have unit coverage (~30 tests). This is a known limitation of `tauri::test::mock_app` returning `AppHandle<MockRuntime>` which cannot substitute for the concrete runtime. Mitigated by having all testable helpers fully covered.

**Alternatives considered:**
- **Bundle dylibs + model in the binary** (rejected — would make the binary ~530 MB and violate package manager norms).
- **Manual download** (the pre-Sprint 6 approach — rejected because it created unacceptable onboarding friction).
- **Single monolithic download** instead of per-file (rejected — per-file allows resumption/retry of individual files and avoids re-downloading 429 MB on a single hash failure).
- **Use `reqwest` async instead of blocking** (rejected — would require introducing a Tokio runtime just for provisioning; the blocking thread is simpler and the download is not latency-critical).

---

### ADR-018: AI Interview — macOS `say` TTS + BYOK question plan generation

**Date:** 2026-07-28

**Status:** Accepted

**Context:** Practice mode was extended with an AI-powered mock interview feature. The requirements were:
- The interviewer must read questions aloud — the user should hear the question, not just read it on screen, simulating a real interview.
- Questions must be tailored to the user's job description and documents, so they exercise relevant skills.
- The user should be able to skip questions or end the interview early.
- The flow must not depend on cloud services for TTS — it must work fully offline.

Two separate sub-decisions:

**Sub-decision A — TTS engine:**
The app needed a text-to-speech engine that is:
- Zero additional installation (bundled with macOS)
- Fast (<1s startup)
- Works without network

**Sub-decision B — Question plan generation:**
The plan must be:
- Generated from the user's job description (paste the JD → get relevant questions)
- Context-aware (uses the user's own documents/RAG to tailor questions)
- Structured (question text, type classification, time budget)
- Generated via the same BYOK LLM provider infrastructure already used for post-call analysis (reuse existing `reqwest` + provider adapters)

**Decision:**

1. **TTS via macOS `say` command** (`tts/mod.rs`): Uses `std::process::Command::new("say")` with the Samantha voice (a high-quality American English female voice available on all macOS versions). The subprocess runs with a 30-second timeout via a monitor thread and `mpsc::recv_timeout`. The `is_available()` check verifies `say` is on PATH (guaranteed on macOS).

2. **Question plan generation via BYOK LLM** (`interview_plan.rs`): A new `generate_interview_plan` Tauri command that:
   - Takes a `job_description` string, `duration_minutes`, `provider`, and optional `model`
   - Queries RAG for relevant documents (`search()` with top_k=5) and injects them as context
   - Builds a Spanish-language prompt requesting a JSON array of `{text, qtype, budget_seconds}`
   - Calls the same provider-specific HTTP adapters as `analyze.rs` (OpenAI/Anthropic/Gemini/OpenRouter/DeepSeek/Ollama)
   - Returns a structured `InterviewPlan` with ordered questions

3. **Interview orchestration** (`interview_runner.rs`):
   - A state machine registered as Tauri state (`Mutex<InterviewRunner>`) that holds a command receiver
   - On `start_ai_interview`, the runner enters a thread loop that iterates planned questions
   - For each question: emit `interview-question` event → call `tts::speak()` → emit `interview-status: "listening"` → wait for next command (skip or timeout)
   - The `skip_ai_question` command advances to the next question; `stop_ai_interview` terminates the loop and emits `interview-finished`

**Consequences:**
- *(Positive)* Zero TTS dependency — `say` is built into macOS since System 7.
- *(Positive)* The question plan uses the same BYOK infrastructure as post-call analysis, so the user's existing API key works for both features.
- *(Positive)* The runner thread is fully controlled by the frontend via Tauri commands — no polling needed.
- *(Positive)* The frontend `LiveInterview` component renders a clean UI with progress, status indicator, and skip/stop buttons.
- *(Negative)* macOS `say` Samantha voice sounds noticeably synthetic — not appropriate for production-quality mock interviews. Could be migrated to Piper or Kokoro TTS in a future sprint.
- *(Negative)* Question plan quality depends entirely on the BYOK LLM chosen. Providers that return unstructured text (not valid JSON) will fail to parse.
- *(Negative)* The runner thread uses `tokio::time::timeout` (the only Tokio dependency in the project) — minimal, but adds ~50ms to compile time.

**Alternatives considered:**
- **Piper TTS** (local, high-quality, but requires bundling a ~50MB model and a Rust FFI or sidecar).
- **ElevenLabs / cloud TTS** (superior voice quality, but breaks the offline requirement and adds cost).
- **Hardcoded question templates** (no personalization — defeats the purpose of tailoring to the user's job and documents).
- **Frontend-only TTS via Web Speech API** (available in the Tauri webview, but unreliable for long-form questions and inconsistent across webview versions).

---

### ADR-019: Lightweight custom i18n system over library dependency

**Date:** 2026-07-28

**Status:** Accepted

**Context:** The app needed bilingual support (English/Spanish) for all user-facing strings. Options were:

- **`react-intl` / `react-i18next`:** Full-featured ICU message format, context providers, async loading, plural rules. Adds ~30–60 KB to the bundle.
- **Custom lightweight system:** A translations object, a `t()` lookup function, and a React hook for reactivity. Zero dependencies beyond React 18.
- **CSS-only approach** (separate HTML files per language): Impossibly brittle for a dynamic app.

Key constraints:
- Language must switch instantly without a full page reload.
- The React tree must re-render on language change without requiring a `<Provider>` wrapper at the root.
- The chosen language must persist across restarts (localStorage + backend `settings.language`).
- The system must work in the overlay window too (separate webview, no shared context).
- Must support template interpolation (`{{count}}` → "5 documents").

**Decision:** Build a custom i18n system (`src/i18n.ts`) with:

1. **Translations as a plain object:** `{ en: { appTitle: "Kue", ... }, es: { appTitle: "Kue", ... } }` — 141 keys per language (282 total entries), with `as const` for type safety.
2. **`t(key, vars?)` function:** Synchronous lookup from the current language object. Template variables replaced via simple `String.replace()`.
3. **`useLanguage()` hook:** Uses React 18's `useSyncExternalStore` to subscribe to language changes. No context provider needed — any component calling `useLanguage()` or `t()` re-renders on language change.
4. **`initLanguage()` for synchronous restore:** Reads from `localStorage` before the first React render, avoiding a flash of the wrong language.
5. **`loadLanguageFromBackend()` for async persistence:** Reads `settings.language` from the Rust backend and updates both localStorage and the in-memory state.
6. **`saveLanguage()` for persistence:** Writes to localStorage (sync) and backend `set_setting` (async fire-and-forget).
7. **Language switcher in `Header.tsx`:** Two toggle buttons (ES/EN) with `aria-pressed` and keyboard accessibility. Calls `onLanguageChange` which triggers `setLanguage()` + `saveLanguage()`.
8. **141 keys per language:** The translations object holds 141 keys per language (282 total entries), covering all UI text, onboarding strings, interview statuses, settings labels, and validation messages.

**Consequences:**
- *(Positive)* Zero bundle size impact from i18n libraries — the translations object is ~25 KB gzipped.
- *(Positive)* Instant language switching — just a state update, no async loading or context re-render.
- *(Positive)* Works in both windows (main and overlay) without a shared context provider.
- *(Positive)* Type-safe — `t()` only accepts valid keys from the `Translations` type.
- *(Negative)* No ICU plural rules — the `{{count}}` interpolation is manual and language-agnostic (works for EN/ES but would not work for languages with complex pluralisation like Arabic or Russian).
- *(Negative)* No lazy loading — all 141 keys per language are loaded at startup. Acceptable for v1 with only two languages.
- *(Negative)* No right-to-left (RTL) support — would need additional work for Arabic/Hebrew if added later.

**Alternatives considered:**
- **`react-intl`** (full ICU support, but heavier and requires a `<IntlProvider>` wrapper — awkward for the overlay window which has a separate React tree).
- **`react-i18next`** (similar complexity, requires async setup for a simple synchronous use case).
- **CSS language classes** (`.lang-en` / `.lang-es` with different text in the markup — completely unmaintainable).
- **Separate builds per language** (doubles build time, requires CI changes — disproportionate for two languages).

---

### ADR-020: Settings dialog with per-feature LLM defaults

**Date:** 2026-07-30

**Status:** Accepted

**Context:** The app has three distinct features that use LLMs: hints (real-time during interview), post-call analysis (after session), and interview plan generation (before session). Each feature may need a different provider or model:
- Hints need low latency → small local model (Ollama) preferred.
- Post-call analysis needs quality → large cloud model (Anthropic/OpenAI) preferred.
- Interview plan generation needs structured JSON output → any provider works.

Previously, each feature hard-coded its own provider or relied on a single global setting. The user had to configure the API key separately per feature but had no way to set per-feature provider/model overrides. The `PostCallPanel` had inline provider/model selectors, but the `PlanGenerator` also had its own, and hints used a hardcoded default.

**Decision:** A three-tier LLM configuration system:

1. **Global defaults** stored in `settings` table as `default_provider` (default: `"openai"`) and `default_model` (optional). These apply to any feature without its own override.
2. **Per-feature overrides** stored as `{feature}_provider` and `{feature}_model` in the same `settings` table (e.g., `analyze_provider`, `plan_provider`, `hint_provider`). Empty values fall back to global defaults.
3. **Settings dialog with 3 tabs** (`SettingsDialog.tsx`): (a) **API Keys** — shows all 6 providers with key status and inline key management; (b) **LLM Defaults** — global provider/model picker + per-feature rows with global/custom radio toggles; (c) **General** — language switcher (moved from header-only to also available here).

The `useLLMSettings(featureKey, providerHardDefault)` custom hook (`hooks.ts`) encapsulates the read/write logic: reads `{featureKey}_provider`, `{featureKey}_model`, `default_provider`, and `default_model` from the backend on mount, resolves the effective provider/model (feature override wins, then global, then hard-coded default), and provides setter functions for the feature-specific overrides.

**Consequences:**
- *(Positive)* Users can route each feature to the most appropriate LLM provider without reconfiguring every time.
- *(Positive)* The `SettingsDialog` provides a single, discoverable UI for all LLM preferences — no more scattered provider dropdowns.
- *(Positive)* The header settings button (`Header.tsx`) gives quick access, and the `PostCallPanel` includes a "Configure all in Settings" link pointing to the LLM Defaults tab.
- *(Positive)* The hook pattern keeps component code clean — each feature just calls `useLLMSettings("analyze")` and gets the resolved provider/model.
- *(Negative)* Eight new settings keys per user (`default_provider`, `default_model`, `hint_provider`, `hint_model`, `analyze_provider`, `analyze_model`, `plan_provider`, `plan_model`) — manageable at this scale.
- *(Negative)* The global default "openai" may not be ideal for users who prefer Anthropic — accepted as a reasonable starting default.
- *(Negative)* The `useLLMSettings` hook fires independent backend reads for each setting key, adding ~4 IPC round-trips per feature on mount. Acceptable because it happens once at component mount, not in a hot path.

**Alternatives considered:**
- **Single hardcoded provider per feature** (the pre-ADR-020 state — rejected because users had no way to change provider from the UI without code changes).
- **Single global provider only** (rejected because it forces the same model on latency-sensitive hints and quality-sensitive analysis).
- **Store all config in localStorage** (rejected — would diverge from the existing pattern where all persistent settings go in the backend `settings` table).
- **Redux/Zustand store for LLM config** (disproportionate complexity — the `useLLMSettings` hook with per-key persistence is simpler and doesn't add dependencies).

---

### ADR-021: Key management commands — delete_key and list_saved_keys

**Date:** 2026-07-30

**Status:** Accepted

**Context:** ADR-016 established the pattern of storing API keys in the OS keychain via `keyring`, with `save_key` and `has_key` Tauri commands. As the app gained:
- More LLM features (hints, analysis, plan generation) — up to 6 providers
- The `SettingsDialog` (ADR-020) with an API Keys tab showing all providers' key status
- The need to delete or change a saved key without overwriting via re-entry

Two gaps emerged:
1. No way to **delete** a key — once saved, the only way to clear it was via System Keychain Access.
2. No way to **list** which providers have saved keys — the frontend had to call `has_key` individually for each provider, which required 6 sequential IPC calls.

**Decision:**

1. **`delete_key` command:** Calls `keyring::Entry::delete_password()`, with `NoEntry` errors swallowed for idempotency — deleting a non-existent key succeeds silently. The frontend `ApiKeyInput` component exposes a delete button (`showDelete` prop) that calls `delete_key` and triggers `onKeyDeleted`.

2. **`list_saved_keys` command:** Takes a `Vec<String>` of provider names (e.g., `["openai", "anthropic", "gemini", "openrouter", "deepseek", "ollama"]`), calls `get_api_key()` for each, and returns the subset that have stored keys. This reduces 6 IPC calls to 1 and is used by `SettingsDialog`'s API Keys tab to show per-provider key status on mount.

**Consequences:**
- *(Positive)* Users can delete keys from within the app — no need to open System Keychain Access.
- *(Positive)* `list_saved_keys` reduces frontend complexity: one call replaces 6 sequential `has_key` calls with a single batch operation.
- *(Positive)* Both commands follow the existing error conventions: `Result<(), String>` for `delete_key` and `Result<Vec<String>, String>` for `list_saved_keys`.
- *(Negative)* `list_saved_keys` iterates over providers sequentially in Rust — if a provider's keychain access hangs, it blocks the thread. Acceptable because keychain access is typically sub-millisecond.
- *(Negative)* The provider list is hardcoded in the frontend (`constants.ts`) and passed in — if a new provider is added without updating the frontend caller, it won't appear in the list. Acceptable — the frontend code is the source of truth for the provider list anyway.

**Alternatives considered:**
- **Keep the status quo** — `has_key` per provider (6 IPC calls on mount). Rejected because it makes the Settings Dialog's API Keys tab load slowly and requires complex loading state management.
- **A single `get_all_keys` command returning a map of provider→status** (rejected — would couple the command to the specific provider list, making it harder to extend with new providers without changing both frontend and backend).
- **Delete via overwrite with empty string** (rejected — the keyring crate treats empty passwords as valid credentials; deleting properly removes the entry).
