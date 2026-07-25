# Kue

> Copiloto de memoria para entrevistas técnicas — te ayuda a recordar tus propias métricas, proyectos y estructura sin generar respuestas por ti.

Aplicación de escritorio (macOS, Tauri v2) con transcripción en tiempo real, RAG local sobre tu CV/proyectos, y hints ultra-cortos para mantener fluidez bajo presión. Post-call, análisis opcional con tu propio LLM (BYOK).

**Estado actual:** Sprint 0 completa — infraestructura base (Tauri + SQLite/sqlite-vec) y captura de audio dual (micrófono vía `cpal` + loopback de sistema vía ScreenCaptureKit) implementadas y con tests. El resto del pipeline (STT → RAG → hints → overlay) está en planificación (ver `spec.md`).

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

La app abrirá una ventana con un contador de prueba. El backend Rust se conecta a SQLite y crea el schema en `~/Library/Application Support/com.kue.app/kue.db`.

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
| `npm run tauri:dev` | Tauri + Vite en modo desarrollo |
| `npm run tauri:build` | Build de producción (genera .dmg) |
| `npm run test:rust:db` | Tests del módulo de base de datos |
| `npm run coverage:rust:db` | Cobertura del módulo de base de datos |

## Arquitectura (alto nivel)

```
┌───────────────────────────────────────────────┐
│              Tauri v2 Shell                     │
│  ┌──────────┐   ┌────────────────────────┐    │
│  │ Frontend │   │  Rust Backend           │    │
│  │ (React + │◄──│  - init_db             │    │
│  │ Tailwind)│   │  - sqlite-vec (RAG)    │    │
│  │          │   │  - get_db_status cmd   │    │
│  └──────────┘   │  - toggle_audio_capture │    │
│                 │  - cpal (mic capture)   │    │
│                 │  - SCK (loopback)       │    │
│                 │  - hound (WAV writer)   │    │
│                 │  - candle (embeddings,  │    │
│                 │    stub)                │    │
│                 └────────────────────────┘    │
└───────────────────────────────────────────────┘
```

**Leyenda:** `cpal`, `screencapturekit-rs` y `hound` tienen código funcional. `candle` sigue siendo stub (dependencia declarada en `Cargo.toml` pero sin código de inferencia aún).

## Stack

| Capa | Tecnología |
|---|---|
| Frontend | React 18 + TypeScript + Tailwind CSS 3 |
| App Core | Rust (Tauri v2) |
| Base de datos | SQLite + sqlite-vec (vectores) |
| Audio | cpal (mic) + screencapturekit-rs (loopback) + hound (WAV) |
| STT (planeado) | Moonshine |
| Embeddings (planeado) | candle (HuggingFace Rust) |
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
│   │   ├── lib.rs        # Tauri builder + setup (registra DB + AudioCapture)
│   │   ├── db/
│   │   │   └── mod.rs    # Schema, migraciones, sqlite-vec, tests
│   │   └── audio/
│   │       ├── mod.rs    # Re-export del módulo capture
│   │       └── capture.rs # Captura dual (cpal + SCK), WAV writer, toggle_audio_capture cmd
│   ├── Cargo.toml        # Dependencias Rust
│   ├── tauri.conf.json   # Configuración Tauri v2
│   └── capabilities/     # Permisos Tauri (core, shell)
├── package.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
└── tsconfig.json
```
