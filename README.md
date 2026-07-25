# Kue

> Copiloto de memoria para entrevistas técnicas — te ayuda a recordar tus propias métricas, proyectos y estructura sin generar respuestas por ti.

Aplicación de escritorio (macOS, Tauri v2) con transcripción en tiempo real, RAG local sobre tu CV/proyectos, y hints ultra-cortos para mantener fluidez bajo presión. Post-call, análisis opcional con tu propio LLM (BYOK).

**Estado actual:** Sprint 0 completa — infraestructura base (Tauri + SQLite/sqlite-vec), captura de audio dual (micrófono vía `cpal` + loopback de sistema vía ScreenCaptureKit) y motor RAG (embeddings con `candle` + búsqueda vectorial con `sqlite-vec`) implementados y con tests (+100 tests en Rust). El resto del pipeline (STT → hints → overlay) está en planificación (ver `spec.md`).

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

La app abrirá una ventana con controles debug para indexar carpetas (RAG) y buscar contexto. El backend Rust se conecta a SQLite y crea el schema en `~/Library/Application Support/com.kue.app/kue.db`.

## Tests

```bash
# Tests de Rust (base de datos — toda la lógica implementada)
npm run test:rust:db

# Tests de Rust (todos los módulos, incluidos futuros)
npm run test:rust

# Cobertura de Rust (requiere cargo-tarpaulin)
npm run coverage:rust:db
npm run coverage:rust:full
```

## Scripts disponibles

| Comando | Descripción |
|---|---|
| `npm run dev` | Servidor Vite standalone (sin Tauri) |
| `npm run build` | Build del frontend (TypeScript + Vite) |
| `npm run preview` | Preview del build de frontend |
| `npm run tauri` | CLI de Tauri directamente |
| `npm run tauri:dev` | Tauri + Vite en modo desarrollo |
| `npm run tauri:build` | Build de producción (genera .dmg) |
| `npm run test:rust:db` | Tests solo del módulo de base de datos |
| `npm run test:rust` | Tests de todos los módulos Rust (>100 tests) |
| `npm run coverage:rust:db` | Cobertura del módulo de base de datos |
| `npm run coverage:rust:full` | Cobertura completa de Rust |
| `npm run coverage:rust:check` | Verifica disponibilidad de herramientas de cobertura |
| `npm run coverage:rust:text` | Cobertura en stdout |

## Arquitectura (alto nivel)

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
│                 └───────────────────────────────┘    │
│  ┌──────────────────────────────────────────────┐    │
│  │  SQLite + sqlite-vec                          │    │
│  │  (sessions · transcript_lines · documents ·   │    │
│  │   chunks · chunks_vec · settings)              │    │
│  └──────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────┘
```

**Leyenda:** Todo el código listado es funcional. `candle` implementa embeddings BERT (`snowflake-arctic-embed-s`) en el módulo `rag::embeddings`, y `sqlite-vec` hace la búsqueda vectorial KNN.

## Stack

| Capa | Tecnología |
|---|---|
| Frontend | React 18 + TypeScript + Tailwind CSS 3 |
| App Core | Rust (Tauri v2) |
| Base de datos | SQLite + sqlite-vec (vectores) |
| Audio | cpal (mic) + screencapturekit-rs (loopback) + hound (WAV) |
| STT (planeado) | Moonshine |
| Embeddings | candle (HuggingFace Rust) + `snowflake-arctic-embed-s` |
| Post-call (planeado) | BYOK (Anthropic/OpenAI/Ollama/etc.) |

## Documentación relacionada

- [`spec.md`](./spec.md) — Especificación funcional completa del producto
- [`design.md`](./design.md) — Diseño técnico y arquitectura actual
- [`adr.md`](./adr.md) — Registro de decisiones arquitectónicas

## Proyecto

```
kue/
├── src/                  # Frontend React + TypeScript
│   ├── App.tsx           # Componente principal (placeholder)
│   ├── main.tsx          # Entry point
│   └── index.css         # Tailwind directives
├── src-tauri/            # Backend Rust (Tauri)
│   ├── src/
│   │   ├── main.rs       # Entry point
│   │   ├── lib.rs        # Tauri builder + setup (registra DB, AudioCapture, RAG, limpia orphan temp dirs)
│   │   ├── types.rs      # TranscriptLine, Speaker (contrato STT → clasificador)
│   │   ├── db/
│   │   │   └── mod.rs    # Schema, migraciones, sqlite-vec, tests
│   │   ├── audio/
│   │   │   ├── mod.rs    # Re-export del módulo capture
│   │   │   └── capture.rs # Captura dual (cpal + SCK), WAV writer, toggle_audio_capture cmd
│   │   └── rag/
│   │       ├── mod.rs    # Re-export de embeddings e indexer
│   │       ├── embeddings.rs # Modelo BERT (snowflake-arctic-embed-s), generación de embeddings
│   │       └── indexer.rs    # Ingesta, chunking, indexación y búsqueda vectorial
│   ├── Cargo.toml        # Dependencias Rust
│   ├── tauri.conf.json   # Configuración Tauri v2
│   └── capabilities/     # Permisos Tauri (core, shell)
├── package.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
└── tsconfig.json
```
