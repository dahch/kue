import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Overlay from "./Overlay";
import ProvisioningProgress from "./ProvisioningProgress";

interface TranscriptEvent {
  session_id: string;
  text: string;
  speaker: string;
}

interface HintPayload {
  text: string;
  type: string;
  session_id: string;
}

interface PanicPayload {
  until_secs: number;
}

export interface SessionRow {
  id: string;
  started_at: string;
  ended_at: string | null;
  company: string;
  role: string;
  mode: string;
  line_count: number;
}

export interface AnalyzeResult {
  summary: string;
  weak_questions: string[];
  forgotten_projects: string[];
  star_improvements: string[];
}

type SessionMode = "practice" | "shadow";

const PROVIDERS = [
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
  { value: "gemini", label: "Gemini" },
  { value: "openrouter", label: "OpenRouter" },
  { value: "ollama", label: "Ollama (local)" },
];

function ApiKeyInput({
  provider,
  hasSavedKey,
  onKeySaved,
}: {
  provider: string;
  hasSavedKey: boolean;
  onKeySaved: () => void;
}) {
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSave = useCallback(async () => {
    if (!apiKey.trim()) return;
    try {
      await invoke("save_key", { provider, key: apiKey.trim() });
      setApiKey("");
      setError(null);
      onKeySaved();
    } catch (e) {
      setError(`Error al guardar: ${e}`);
    }
  }, [provider, apiKey, onKeySaved]);

  return (
    <div className="mb-4">
      <label className="mb-1 block text-sm font-medium text-zinc-400">
        API Key ({provider})
      </label>
      <div className="flex gap-2">
        <div className="relative flex-1">
          <input
            type={showKey ? "text" : "password"}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={hasSavedKey ? "Key guardada (sobrescribir)" : "Pega tu API key"}
            className="w-full rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 pr-10 text-sm text-white placeholder-zinc-500"
          />
          <button
            className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-zinc-500 hover:text-zinc-300"
            onClick={() => setShowKey(!showKey)}
          >
            {showKey ? "ocultar" : "mostrar"}
          </button>
        </div>
        <button
          className="rounded-lg bg-zinc-700 px-3 py-2 text-sm font-medium text-white hover:bg-zinc-600 disabled:opacity-50"
          onClick={handleSave}
          disabled={!apiKey.trim()}
        >
          Guardar
        </button>
      </div>
      {hasSavedKey && (
        <p className="mt-1 text-xs text-emerald-500">Key guardada en Keychain</p>
      )}
      {error && <p className="mt-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}

export function PostCallPanel({ session }: { session: SessionRow }) {
  const [transcriptReady, setTranscriptReady] = useState(false);
  const [checking, setChecking] = useState(true);
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("");
  const [analyzing, setAnalyzing] = useState(false);
  const [result, setResult] = useState<AnalyzeResult | null>(null);
  const [error, setError] = useState("");
  const [hasSavedKey, setHasSavedKey] = useState(false);
  const unlistenRef = useRef<Promise<() => void> | null>(null);

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const ready: boolean = await invoke("is_transcript_ready", {
          sessionId: session.id,
        });
        if (!cancelled) {
          setTranscriptReady(ready);
          setChecking(false);
        }
      } catch {
        if (!cancelled) setChecking(false);
      }
    };
    check();
    return () => { cancelled = true; };
  }, [session.id]);

  useEffect(() => {
    const unlistenPromise = listen<{ session_id: string }>(
      "post-call-transcript-ready",
      (event) => {
        if (event.payload.session_id === session.id) {
          setTranscriptReady(true);
          setChecking(false);
        }
      }
    );
    unlistenRef.current = unlistenPromise;
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [session.id]);

  useEffect(() => {
    invoke<boolean>("has_key", { provider })
      .then(setHasSavedKey)
      .catch((e) => console.error("Failed to check key:", e));
  }, [provider]);

  const handleAnalyze = useCallback(async () => {
    setAnalyzing(true);
    setError("");
    setResult(null);
    try {
      const res: AnalyzeResult = await invoke("analyze_session", {
        sessionId: session.id,
        provider,
        model: model.trim() || null,
      });
      setResult(res);
    } catch (e) {
      setError(`${e}`);
    } finally {
      setAnalyzing(false);
    }
  }, [session.id, provider, model]);

  const modeLabel = session.mode === "practice" ? "Practice" : "Shadow";
  const dateStr = new Date(session.started_at).toLocaleString();

  return (
    <div className="w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6">
      <h2 className="mb-3 text-lg font-semibold text-emerald-400">
        Post-call: {dateStr}
      </h2>
      <p className="mb-4 text-sm text-zinc-400">
        {modeLabel} &middot; {session.line_count} líneas
      </p>

      <div className="mb-4">
        {checking ? (
          <p className="text-sm text-zinc-500">Verificando transcripción...</p>
        ) : transcriptReady ? (
          <p className="text-sm text-emerald-400">Transcripción completa ✓</p>
        ) : (
          <div className="flex items-center gap-2 text-sm text-amber-400">
            <span className="inline-block h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
            Procesando transcripción...
          </div>
        )}
      </div>

      <ApiKeyInput
        provider={provider}
        hasSavedKey={hasSavedKey}
        onKeySaved={() => setHasSavedKey(true)}
      />

      <div className="mb-4 flex gap-3">
        <div className="flex-1">
          <label className="mb-1 block text-sm font-medium text-zinc-400">Proveedor</label>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value)}
            className="w-full rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 text-sm text-white"
          >
            {PROVIDERS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex-1">
          <label className="mb-1 block text-sm font-medium text-zinc-400">
            Modelo <span className="text-zinc-600">(opcional)</span>
          </label>
          <input
            type="text"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder="default"
            className="w-full rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 text-sm text-white placeholder-zinc-500"
          />
        </div>
      </div>

      <button
        className={`w-full rounded-lg py-3 font-medium transition-colors ${
          !transcriptReady || analyzing
            ? "cursor-not-allowed bg-zinc-700 text-zinc-500"
            : "bg-emerald-600 text-white hover:bg-emerald-500"
        }`}
        onClick={handleAnalyze}
        disabled={!transcriptReady || analyzing}
      >
        {analyzing ? (
          <span className="flex items-center justify-center gap-2">
            <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-zinc-400 border-t-white" />
            Analizando...
          </span>
        ) : transcriptReady ? (
          "Analizar"
        ) : (
          "Procesando transcripción..."
        )}
      </button>

      {error && (
        <div className="mt-4 rounded-lg border border-red-800 bg-red-900/30 p-3">
          <p className="text-sm text-red-400">{error}</p>
        </div>
      )}

      {result && (
        <div className="mt-4 space-y-4">
          <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
            <h3 className="mb-2 text-sm font-semibold text-zinc-400 uppercase tracking-wide">
              Resumen
            </h3>
            <p className="text-sm text-zinc-200">{result.summary}</p>
          </div>

          {result.weak_questions.length > 0 && (
            <div className="rounded-lg border border-amber-800/40 bg-amber-900/20 p-4">
              <h3 className="mb-2 text-sm font-semibold text-amber-400 uppercase tracking-wide">
                Preguntas débiles
              </h3>
              <ul className="list-disc space-y-1 pl-4 text-sm text-zinc-300">
                {result.weak_questions.map((q) => (
                  <li key={`weak-${q.slice(0, 20)}`}>{q}</li>
                ))}
              </ul>
            </div>
          )}

          {result.forgotten_projects.length > 0 && (
            <div className="rounded-lg border border-blue-800/40 bg-blue-900/20 p-4">
              <h3 className="mb-2 text-sm font-semibold text-blue-400 uppercase tracking-wide">
                Proyectos no mencionados
              </h3>
              <ul className="list-disc space-y-1 pl-4 text-sm text-zinc-300">
                {result.forgotten_projects.map((p) => (
                  <li key={`fp-${p.slice(0, 20)}`}>{p}</li>
                ))}
              </ul>
            </div>
          )}

          {result.star_improvements.length > 0 && (
            <div className="rounded-lg border border-purple-800/40 bg-purple-900/20 p-4">
              <h3 className="mb-2 text-sm font-semibold text-purple-400 uppercase tracking-wide">
                Mejoras STAR
              </h3>
              <ul className="list-disc space-y-1 pl-4 text-sm text-zinc-300">
                {result.star_improvements.map((s) => (
                  <li key={`star-${s.slice(0, 20)}`}>{s}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function MainApp() {
  const [mode, setMode] = useState<SessionMode>("practice");
  const [running, setRunning] = useState(false);
  const [lastTranscript, setLastTranscript] = useState("");
  const [lastHint, setLastHint] = useState("");
  const [panicking, setPanicking] = useState(false);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [selectedSession, setSelectedSession] = useState<SessionRow | null>(null);
  const panicTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const unlistenTranscript = listen<TranscriptEvent>("new-transcript", (event) => {
      setLastTranscript(event.payload.text);
    });

    const unlistenHint = listen<HintPayload>("new-hint", (event) => {
      setLastHint(event.payload.text);
    });

    const unlistenPanic = listen<PanicPayload>("panic-mode", (event) => {
      setPanicking(true);
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
      const ms = event.payload.until_secs * 1000;
      panicTimerRef.current = setTimeout(() => setPanicking(false), ms);
    });

    return () => {
      unlistenTranscript.then((fn) => fn());
      unlistenHint.then((fn) => fn());
      unlistenPanic.then((fn) => fn());
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
    };
  }, []);

  const refreshSessions = useCallback(async () => {
    try {
      const list: SessionRow[] = await invoke("get_sessions");
      setSessions(list);
    } catch (e) {
      console.error("Failed to load sessions:", e);
    }
  }, []);

  useEffect(() => {
    refreshSessions();
  }, [refreshSessions]);

  const handleStart = useCallback(async () => {
    try {
      await invoke("start_session", { mode });
      setRunning(true);
    } catch (e) {
      console.error("Failed to start session:", e);
    }
  }, [mode]);

  const handleStop = useCallback(async () => {
    try {
      await invoke("stop_session");
      setRunning(false);
    } catch (e) {
      console.error("Failed to stop session:", e);
    }
  }, []);

  // After stop completes, refresh sessions and select the latest
  useEffect(() => {
    if (!running) {
      const timer = setTimeout(() => {
        refreshSessions().then(() => {
          setSelectedSession((prev) => prev ?? sessions[0] ?? null);
        });
      }, 500);
      return () => clearTimeout(timer);
    }
  }, [running, refreshSessions, sessions]);

  const handlePanic = useCallback(async () => {
    try {
      await invoke("panic_mode");
    } catch (e) {
      console.error("Failed to activate panic mode:", e);
    }
  }, []);

  return (
    <div className="flex min-h-screen flex-col items-center bg-zinc-950 p-8 text-white">
      <h1 className="mb-8 mt-4 text-4xl font-bold">Kue</h1>

      {/* Mode selector */}
      <div className="mb-6 w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6">
        <h2 className="mb-3 text-lg font-semibold text-blue-400">Modo</h2>
        <div className="flex gap-3">
          <button
            className={`flex-1 rounded-lg py-2 font-medium transition-colors ${
              mode === "practice"
                ? "bg-blue-600 text-white"
                : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
            }`}
            onClick={() => setMode("practice")}
            disabled={running}
          >
            Practice
          </button>
          <button
            className={`flex-1 rounded-lg py-2 font-medium transition-colors ${
              mode === "shadow"
                ? "bg-blue-600 text-white"
                : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
            }`}
            onClick={() => setMode("shadow")}
            disabled={running}
          >
            Shadow
          </button>
        </div>
      </div>

      {/* Session controls */}
      <div className="mb-6 flex w-full max-w-lg gap-3">
        {!running ? (
          <button
            className="flex-1 rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
            onClick={handleStart}
          >
            Iniciar Sesión
          </button>
        ) : (
          <button
            className="flex-1 rounded-lg bg-red-600 py-3 font-medium transition-colors hover:bg-red-500"
            onClick={handleStop}
          >
            Detener Sesión
          </button>
        )}

        <button
          className={`rounded-lg px-6 py-3 font-medium transition-colors ${
            panicking
              ? "bg-orange-500 text-white"
              : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
          }`}
          onClick={handlePanic}
          disabled={!running}
        >
          {panicking ? "\u{1F507} 10s" : "P\u00e1nico"}
        </button>
      </div>

      {/* Log area */}
      <div className="mb-6 w-full max-w-lg space-y-3">
        <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-4">
          <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-zinc-500">
            Último transcript
          </h3>
          <p className="min-h-[1.25rem] text-sm text-zinc-300">
            {lastTranscript || "\u00a0"}
          </p>
        </div>

        <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-4">
          <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-zinc-500">
            Último hint
          </h3>
          <p className="min-h-[1.25rem] text-sm text-zinc-300">
            {lastHint || "\u00a0"}
          </p>
        </div>
      </div>

      {/* Post-call panel */}
      {selectedSession && (
        <div className="mb-6 w-full max-w-lg">
          <PostCallPanel session={selectedSession} />
        </div>
      )}

      {/* Session history */}
      {!running && sessions.length > 0 && (
        <div className="w-full max-w-lg">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">
            {selectedSession ? "Sesiones anteriores" : "Sesiones"}
          </h2>
          <div className="space-y-2">
            {sessions.map((s) => (
              <button
                key={s.id}
                className={`w-full rounded-lg border px-4 py-3 text-left text-sm transition-colors ${
                  selectedSession?.id === s.id
                    ? "border-emerald-600 bg-zinc-800 text-white"
                    : "border-zinc-700 bg-zinc-900 text-zinc-400 hover:border-zinc-500"
                }`}
                onClick={() => setSelectedSession(s)}
              >
                {new Date(s.started_at).toLocaleString()} &middot;{" "}
                {s.mode === "practice" ? "Practice" : "Shadow"} &middot;{" "}
                {s.line_count} líneas
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function App() {
  const [isOverlay, setIsOverlay] = useState(false);
  const [showMain, setShowMain] = useState(false);
  const handleProvisioned = useCallback(() => setShowMain(true), []);

  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label === "overlay") {
      setIsOverlay(true);
    }
  }, []);

  if (isOverlay) {
    return <Overlay />;
  }

  if (!showMain) {
    return <ProvisioningProgress onProvisioned={handleProvisioned} />;
  }

  return <MainApp />;
}

export default App;
