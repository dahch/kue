# Kue — Architecture Decision Records

---

### ADR-001: Arquitectura de escritorio nativa vía Tauri (Rust + React)

**Contexto:** Necesitamos capturar audio del sistema (loopback) y micrófono, y mostrar un overlay que no interfiera con el screen-sharing de videollamadas. Ambas cosas requieren acceso a nivel de OS que un navegador no expone.

**Decisión:** Tauri, backend en Rust, frontend en React/TypeScript. Reutiliza el conocimiento ya adquirido en [[musicsync]].

**Consecuencias:**
- *(Positivas)* Acceso directo a APIs de audio de bajo nivel. Ventanas nativas con `always_on_top` y transparencia. Binario ~15-20MB, RAM mínima. Audio y ML (Moonshine, candle) corren en hilos nativos sin bloquear la UI.
- *(Negativas)* Curva de aprendizaje en Rust y empaquetado.

**Alternativas descartadas:** Electron (footprint mayor, peor acceso a audio de bajo nivel). Web puro (imposible capturar loopback).

---

### ADR-002: macOS-only para v1

**Contexto:** Windows (WASAPI) y macOS (Core Audio/ScreenCaptureKit) requieren implementaciones de captura de audio completamente distintas. Cubrir ambos en v1 multiplica el esfuerzo antes de validar si el producto funciona.

**Decisión:** v1 se construye y valida solo en macOS. Windows/Linux quedan evaluados para v2 según resultados del v1.

**Consecuencias:** La capa de audio capture debe diseñarse con una interfaz clara (trait en Rust) que permita añadir backends de Windows/Linux después sin reescribir el resto del pipeline.

**Alternativas descartadas:** Soporte multi-OS desde v1 (esfuerzo desproporcionado antes de validar el concepto).

---

### ADR-003: Captura de loopback vía ScreenCaptureKit, sin drivers virtuales

**Contexto:** Se necesita capturar el audio de salida del sistema (voz del entrevistador) en macOS. La solución tradicional (BlackHole u otro driver de audio virtual) exige que el usuario instale y configure un dispositivo "Multi-Output" antes de poder usar la app, lo cual además puede desactivar las teclas de volumen y fallar al cambiar a audífonos Bluetooth.

**Decisión:** Usar ScreenCaptureKit (macOS 13+) vía el crate `screencapturekit-rs` para loopback de sistema, sin drivers de terceros. El micrófono se captura por separado con `cpal`.

**Consecuencias:**
- *(Positivas)* Cero fricción de instalación para el usuario; el audio se enruta a nivel de sistema sin tocar la configuración de audio del usuario.
- *(Negativas)* Requiere permiso explícito del usuario en Ajustes → Privacidad → Screen & System Audio Recording (no existe entitlement de firma que lo evite). Hay reportes de segfaults intermitentes en versiones de macOS anteriores a la 14 — pendiente validar el mínimo soportado.

**Alternativas descartadas:** BlackHole/Loopback (driver virtual de terceros, fricción de instalación alta, riesgos de romper la configuración de audio del usuario).

---

### ADR-004: Diarización mediante canales de audio separados

**Contexto:** Necesitamos distinguir qué dice el entrevistador y qué dice el candidato, sin depender de modelos de diarización acústica costosos o poco fiables en tiempo real.

**Decisión:** Capturar dos flujos nativos: micrófono del usuario (Canal A, vía `cpal`) y loopback del sistema (Canal B, vía ScreenCaptureKit). El STT solo se aplica al Canal B.

**Consecuencias:** Etiquetado de hablante gratuito y confiable. Ahorro de cómputo al transcribir solo el canal relevante. Moonshine's `identify_speakers` queda como respaldo si el mic capta ambas voces (ej. altavoces sin audífonos).

**Alternativas descartadas:** Diarización acústica pura (menos confiable como mecanismo primario), captura de audio mixto + separación por IA (poco fiable en tiempo real).

---

### ADR-005: Moonshine para STT, en vez de Whisper o STT en la nube

**Contexto:** Necesitamos transcripción en tiempo real, baja latencia, y funcionamiento sin conexión a internet — el audio de las entrevistas no debe salir de la máquina.

**Decisión:** Moonshine (Medium), corriendo localmente vía su core en C++ con interfaz C bindeable desde Rust.

**Consecuencias:**
- *(Positivas)* Latencia <260ms. Sin costo por API. Privacidad total del audio del entrevistador. ~5x más rápido que Whisper equivalente.
- *(Negativas)* Uso de CPU (~1-2 cores). Modelo de ~300MB a empaquetar o descargar en el onboarding.

**Alternativas descartadas:** Deepgram/AssemblyAI (de pago, dependientes de internet). Whisper.cpp (más lento y pesado para tiempo real). Parakeet (sin diarización nativa, corre como servidor HTTP separado).

---

### ADR-006: sqlite-vec + candle para el RAG del contexto, en vez de zvec

**Contexto:** El contexto propio (CV, proyectos, historias) es de volumen pequeño (miles de chunks, no millones). Necesitamos indexarlo y buscarlo rápido (<20ms) dentro de un único binario, sin procesos externos ni dependencias de red.

**Decisión:** sqlite-vec (extensión de SQLite para vectores) + candle (framework ML de HuggingFace en Rust) para embeddings locales. Todo vive en el mismo archivo SQLite que transcripts/sesiones/settings.

**Consecuencias:**
- *(Positivas)* Un solo archivo `.db` para toda la app — backup y portabilidad triviales. Cero procesos en segundo plano. Consultas híbridas (texto + vector) en una sola query SQL.
- *(Negativas)* sqlite-vec sigue en fase alpha (breaking changes posibles entre releases menores), aceptado como riesgo menor dado que a esta escala la búsqueda por fuerza bruta es suficiente. candle requiere la descarga inicial del modelo de embeddings (~90MB para all-MiniLM).

**Alternativas descartadas:** zvec (motor dedicado más maduro en ANN, pero implica un segundo motor de almacenamiento separado). LEANN (pipeline de ingesta muy completo, pero ecosistema 100% Python, requeriría sidecar). LanceDB/Qdrant/Milvus (requieren servidor o complican unificar con la DB de transcripts).

---

### ADR-007: Hints de 5-8 palabras, nunca respuestas completas

**Contexto:** Herramientas como Cluely generan respuestas completas en tiempo real, lo cual se nota (cadencia robótica, delays) y traslada el problema al primer día del puesto si el usuario es contratado apoyándose en eso. Queremos ayudar sin romper el flujo natural de la conversación ni cruzar a "cheating".

**Decisión:** El clasificador/motor de hints nunca genera una respuesta. Solo extrae del contexto propio del usuario una métrica, un nombre de proyecto o una etiqueta de estructura, mostrada en formato breve ("💡 {tag}: {metric}", máx. 8 palabras).

**Consecuencias:** El usuario construye la oración mentalmente, manteniendo autenticidad y su propia voz. La interfaz no requiere leer párrafos, solo un vistazo. El límite entre "recordar tu propio dato" y "que el modelo argumente por ti" queda como principio de diseño para todo el motor de hints.

**Alternativas descartadas:** Generación de respuestas completas con LLM en tiempo real (latencia alta, riesgo de detección, y el problema ético que motivó este diseño desde el inicio).

---

### ADR-008: Modos Practice y Shadow

**Contexto:** Un mismo mecanismo de hints no sirve igual para practicar que para una entrevista real: en la real, mostrar hints todo el tiempo sí sería una forma de cheating; en la práctica, ayuda que sean generosos y didácticos.

**Decisión:** Dos modos explícitos. **Practice**: hints inmediatos y con explicación de estructura. **Shadow**: hints solo si el usuario lleva más de 2.5s de silencio tras la pregunta (bloqueo real), y sin explicación adicional.

**Consecuencias:** El propio diseño del producto refuerza el límite ético — en modo real, el sistema por defecto no interviene salvo que haya evidencia de que el usuario lo necesita. El umbral de 2.5s queda como parámetro a validar empíricamente (ver spec.md, Open Questions).

**Alternativas descartadas:** Un solo modo con el mismo comportamiento en ambos contextos (no distingue el nivel de intervención aceptable entre practicar y una entrevista real).

---

### ADR-009: Almacenamiento unificado — SQLite como fuente de verdad

**Contexto:** Necesitamos almacenar transcripts con timestamps, metadatos de sesión, y vectores de embeddings en un solo lugar portable.

**Decisión:** SQLite como motor único, con tablas para `sessions`, `transcript_lines`, `documents`, `chunks` y `chunks_vec` (vía sqlite-vec).

**Consecuencias:** Base de datos como archivo único, fácil de copiar/respaldar/borrar. Queries SQL familiares. Operaciones híbridas texto+vector en una sola query.

**Alternativas descartadas:** Motores de almacenamiento separados para vectores vs. datos relacionales (ver ADR-006).

---

### ADR-010: Análisis post-call con BYOK (bring your own key)

**Contexto:** El análisis post-call no tiene restricción de latencia, así que puede usar cualquier modelo grande. Forzar un proveedor propio implica costo de infraestructura para el desarrollador y menos control de privacidad para el usuario.

**Decisión:** El usuario aporta su propia API key (Anthropic, OpenAI, Gemini, OpenRouter) o apunta a un endpoint local/OpenAI-compatible (Ollama, vLLM, MLX). Mismo patrón "pluggable AI providers" ya usado en [[jobmatch-ai]]. La key se guarda en el keychain nativo del OS, nunca en texto plano en la tabla `settings`.

**Consecuencias:**
- *(Positivas)* Cero gastos de infraestructura de IA para el desarrollador. El usuario elige su balance entre costo y privacidad (puede ir 100% local con Ollama).
- *(Negativas)* Fricción inicial — el usuario debe obtener y configurar su propia key.

**Alternativas descartadas:** Proveer un modelo propio gestionado por la app (costo de infraestructura, menor privacidad para el usuario).