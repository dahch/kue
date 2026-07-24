# Kue — spec.md (v1)

> Nombre en clave: **Kue**.

> **Estado de implementación:** Sprint 0 completado (infraestructura Tauri + SQLite + sqlite-vec). El schema de BD está implementado con 20+ tests. Las secciones §3–§9 describen el producto completo planeado; consulta [`design.md`](./design.md) para lo que está realmente construido. 

## 1. Objetivo

Aplicación de escritorio (macOS-only en v1) que funciona como "copiloto de memoria" para entrevistas técnicas y simulacros. No responde por el usuario: extrae información de su propio contexto (CV, proyectos, métricas) y muestra hints ultra-cortos (5-8 palabras) que ayudan a mantener fluidez y estructura bajo presión. Al finalizar, guarda el transcript completo para un análisis post-call bajo demanda.

## 2. Visión general del producto

**Mercado objetivo:** ingenieros de software, científicos de datos y perfiles técnicos preparándose para entrevistas, o que quieren un apoyo de memoria durante procesos de selección reales.

**Propuesta de valor:**
- **Sin trampas:** no genera respuestas; solo recuerda tus propias métricas, proyectos y estructura.
- **Privacidad total:** STT, RAG y clasificación corren 100% local. El único dato que sale de la máquina es el transcript post-call, y solo si el usuario decide enviarlo a un LLM externo (BYOK).
- **Bajo estrés:** reduce la ansiedad escénica recordando puntos clave en el momento justo, no antes ni después.

## 3. Objetivos / no-objetivos (v1)

**Sí:**
- Transcripción en tiempo real con separación de hablante por canal de audio.
- Modo **Practice** (simulacro, hints didácticos e inmediatos) y modo **Shadow** (entrevista real, hints solo si hay bloqueo >2.5s).
- Ingesta de contexto propio (PDF/TXT/MD) indexado localmente vía RAG.
- Transcript completo guardado por sesión.
- Análisis post-call bajo demanda con BYOK.

**No (v1):**
- Windows y Linux — **macOS-only por ahora**, evaluado para v2 según qué tan bien funcione este v1.
- Respuestas completas generadas en vivo — fuera de alcance por diseño, no por plazo.
- Sync multi-dispositivo o backend en la nube.
- Clonación de voz / TTS en vivo.

## 4. Features principales

| Módulo | Descripción | Modo |
|---|---|---|
| **Practice** | Entrevista simulada con feedback generoso; hints más didácticos, el clasificador explica la estructura. | Local |
| **Shadow** | Entrevista real; hints escasos, solo aparecen si el usuario se bloquea (delay > 2.5s tras la pregunta). | Local |
| **Post-Call** | Botón que analiza el transcript completo: resumen, preguntas débiles, proyectos olvidados, estructura STAR mejorable. | BYOK |

## 5. Stack tecnológico

| Capa | Tecnología | Justificación |
|---|---|---|
| Frontend | React + TypeScript | UI dinámica, prototipado rápido. |
| App Core | Rust (Tauri) | Acceso nativo a audio, ventanas transparentes (overlay), binario único. |
| Captura de audio | `cpal` (mic) + `screencapturekit-rs` (loopback sistema) | ScreenCaptureKit (macOS 13+) captura audio de sistema sin drivers virtuales — evita depender de BlackHole. Requiere permiso de usuario en Ajustes → Privacidad → Screen & System Audio Recording (no hay entitlement de firma que lo evite). |
| STT | Moonshine (Medium) | Local, streaming, <260ms de latencia, diarización nativa como respaldo. |
| Embeddings (RAG) | candle (HuggingFace Rust) | Inferencia nativa en Rust. Candidatos: `all-MiniLM-L6-v2` o `snowflake-arctic-embed-s` (pendiente benchmark propio). |
| Vector DB / Storage | SQLite + sqlite-vec | Búsqueda vectorial dentro del mismo archivo `.db` que transcripts/sesiones. |
| Clasificador de preguntas | Rust, heurísticas + regex | Sin LLM externo — ver §7 para el detalle de reglas. |
| Análisis post-call | BYOK (Anthropic/OpenAI/Gemini/OpenRouter/Ollama/OpenAI-compatible) | Sin presión de latencia, usuario controla costo y privacidad. |
| Secretos (API keys) | Keychain nativo del OS vía `tauri-plugin-stronghold` o keyring | Nunca texto plano en la tabla `settings`. |

## 6. Arquitectura de módulos

```text
┌─────────────────────────────────────────────────────────────────┐
│                        TAURI SHELL (Rust)                        │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │   Captura de Audio                                        │   │
│  │   - Canal A: Micrófono (cpal) — tu voz                    │   │
│  │   - Canal B: Loopback de sistema (ScreenCaptureKit)        │   │
│  │             — entrevistador                                │   │
│  └──────────┬────────────────────────────────┬───────────────┘   │
│             ▼                                ▼                   │
│  ┌──────────────────────┐         ┌───────────────────────┐      │
│  │ STT (Moonshine)      │         │ Buffer local (WAV)    │      │
│  │ Solo canal B en      │         │ Canal A+B, para        │      │
│  │ tiempo real           │         │ post-call               │      │
│  └──────┬───────────────┘         └───────────────────────┘      │
│         ▼ texto streaming                                        │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │  Clasificador de Preguntas (heurísticas + regex)          │    │
│  │  - ¿Es pregunta? (interrogación + verbos imperativos      │    │
│  │    tipo "cuéntame", "dime", "descríbeme")                  │    │
│  │  - Tipo: Técnica / STAR / Arquitectura / Trampa            │    │
│  └──────────┬──────────────────────────────────────────────┘     │
│             ▼ si es pregunta                                     │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  RAG (sqlite-vec + candle)                               │     │
│  │  - Embedding de la pregunta                               │     │
│  │  - vec_distance_cosine contra chunks_vec                  │     │
│  │  - Recupera el chunk (proyecto/métrica) más cercano        │     │
│  └──────────┬─────────────────────────────────────────────┘      │
│             ▼ chunk recuperado                                   │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  Generador de Hints (Rust puro)                          │     │
│  │  - Formatea: "💡 {tag}: {metric}" (máx. 8 palabras)       │     │
│  │  - En Shadow: solo dispara si delay > 2.5s                │     │
│  └──────────┬─────────────────────────────────────────────┘      │
│             ▼ evento Tauri                                       │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  Overlay (ventana Tauri)                                  │     │
│  │  - always_on_top, click-through, semi-transparente         │     │
│  │  - posicionable, botón de Pánico (silencia hints)          │     │
│  └─────────────────────────────────────────────────────────┘     │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────┐     │
│  │  SQLite (fuente de verdad única)                          │     │
│  │  transcripts · docs (vectores) · sessions · settings        │     │
│  └─────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────┘
```

## 7. Clasificador de preguntas — detalle

Reglas heurísticas, sin LLM:
- Señal directa: signo de interrogación.
- Señal por verbo imperativo al inicio de frase (sin "?"): "cuéntame", "dime", "descríbeme", "explícame", "camínenme por" — cubre preguntas conductuales típicas que en inglés/español no siempre llevan interrogación.
- Clasificación de tipo (técnica / STAR / arquitectura / trampa) por palabras clave asociadas a cada categoría.
- Falsos positivos esperables en small talk — mitigar con lista de exclusión de frases de cortesía ("¿cómo estás?", "¿me escuchas bien?").

## 8. Modelo de datos

```sql
sessions(id, started_at, ended_at, company, role, mode)  -- mode: practice|shadow
transcript_lines(id, session_id, speaker, text, started_at_ms, ended_at_ms)
documents(id, filename, type, added_at)
chunks(id, document_id, text, chunk_index, tag, metric)
chunks_vec(chunk_id, embedding)  -- vía sqlite-vec
settings(key, value)  -- NO incluye API keys (ver §5, Keychain)
```

## 9. Flujo de usuario

1. **Setup inicial:** el usuario carga CV/proyectos, la app indexa (candle genera embeddings → sqlite-vec). Si quiere post-call, configura su API key (guardada en keychain, no en `settings`).
2. **Durante la entrevista:** elige Practice o Shadow. Captura dual de audio arranca. Moonshine transcribe canal B. El clasificador detecta preguntas y dispara RAG. El overlay muestra el hint ~3s y desaparece (inmediato en Practice, tras 2.5s de bloqueo en Shadow).
3. **Post-call:** se guarda transcript completo. Botón "Analizar" envía transcript + contexto relevante al LLM elegido (BYOK) → resumen, preguntas débiles, proyectos no mencionados, mejoras de estructura STAR.

## 10. Plan de desarrollo (MVP)

| Sprint | Objetivo | Entregables | Estado |
|---|---|---|---|---|
| 0 | Infraestructura base | Proyecto Tauri + React. Dependencias Rust (`cpal`, `screencapturekit-rs`, `tauri`, `rusqlite`+`sqlite-vec`, `candle`). Schema SQLite completo (sessions, transcript_lines, documents, chunks, chunks_vec, settings) con migraciones. sqlite-vec registrado. `get_db_status` command. 20+ tests. | ✅ **Completado** |
| 1 | Captura de audio & STT | Loopback funcional en macOS vía ScreenCaptureKit (permiso de usuario gestionado en onboarding). Integración de Moonshine en canal B. Buffer en disco. | ⬜ No iniciado |
| 2 | Motor de RAG | Indexación de documentos locales. sqlite-vec + candle generando y buscando embeddings. Objetivo: query <20ms. | ⬜ No iniciado |
| 3 | Clasificador & hints | Reglas de §7 en Rust. Generación de hint de 5-8 palabras. Eventos Tauri al frontend. | ⬜ No iniciado |
| 4 | Overlay & UI | Ventana transparente, always-on-top, click-through. Practice vs Shadow. Botón de Pánico. | ⬜ No iniciado |
| 5 | Post-call & BYOK | Export/consulta de SQLite. Llamada a API externa. Guardado seguro de key en keychain. Guardado del análisis. | ⬜ No iniciado |

## 11. Open Questions / riesgos

- **Legalidad de grabar sin consentimiento explícito de la otra parte** — revisar para España como mínimo antes de que esto sea un hábito regular.
- Benchmark propio entre `all-MiniLM-L6-v2` y `snowflake-arctic-embed-s` para elegir el modelo de embeddings definitivo.
- Tamaño de chunk / overlap para el RAG de contexto — aún sin definir.
- Estabilidad de ScreenCaptureKit en macOS <14 — hay reportes de segfaults intermitentes en versiones antiguas; validar el mínimo soportado antes de comprometerlo en el onboarding.
- Umbral de 2.5s en Shadow — validar empíricamente que no se sienta ni muy ansioso ni muy tarde.