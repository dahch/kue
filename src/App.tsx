import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Onboarding from "./Onboarding";
import Overlay from "./Overlay";
import ProvisioningProgress from "./ProvisioningProgress";
import Header from "./Header";
import {
  formatLines,
  initLanguage,
  loadLanguageFromBackend,
  saveLanguage,
  setLanguage,
  speakerLabel,
  t,
  useLanguage,
  type Language,
} from "./i18n";
import type { IndexSummary } from "./types";
import { formatIndexResult, isValidFolderPath } from "./validation";

interface TranscriptLine {
  text: string;
  speaker: string;
  started_at_ms: number;
  ended_at_ms: number;
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
      setError(`${t("error")}: ${e}`);
    }
  }, [provider, apiKey, onKeySaved]);

  return (
    <div className="mb-4">
      <label className="mb-1 block text-sm font-medium text-zinc-400">
        {t("apiKey")} ({provider})
      </label>
      <div className="flex gap-2">
        <div className="relative flex-1">
          <input
            type={showKey ? "text" : "password"}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={hasSavedKey ? t("apiKey") : t("apiKey")}
            className="w-full rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 pr-10 text-sm text-white placeholder-zinc-500"
          />
          <button
            className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-zinc-500 hover:text-zinc-300"
            onClick={() => setShowKey(!showKey)}
          >
            {showKey ? t("hide") : t("show")}
          </button>
        </div>
        <button
          className="rounded-lg bg-zinc-700 px-3 py-2 text-sm font-medium text-white hover:bg-zinc-600 disabled:opacity-50"
          onClick={handleSave}
          disabled={!apiKey.trim()}
        >
          {t("save")}
        </button>
      </div>
      {hasSavedKey && (
        <p className="mt-1 text-xs text-emerald-500">{t("apiKey")} {t("apiKeySaved")}</p>
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

  const modeLabel = session.mode === "practice" ? t("practice") : t("shadow");
  const dateStr = new Date(session.started_at).toLocaleString();

  return (
    <div className="w-full rounded-xl border border-zinc-700 bg-zinc-900 p-6">
      <h2 className="mb-3 text-lg font-semibold text-emerald-400">
        Post-call: {dateStr}
      </h2>
      <p className="mb-4 text-sm text-zinc-400">
        {modeLabel} &middot; {formatLines(session.line_count)}
        {session.company && (
          <span> &middot; {session.company}{session.role ? ` (${session.role})` : ""}</span>
        )}
      </p>

      <div className="mb-4">
        {checking ? (
          <p className="text-sm text-zinc-500">{t("processingTranscript")}</p>
        ) : transcriptReady ? (
          <p className="text-sm text-emerald-400">{t("transcriptComplete")} ✓</p>
        ) : (
          <div className="flex items-center gap-2 text-sm text-amber-400">
            <span className="inline-block h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
            {t("processingTranscript")}
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
          <label className="mb-1 block text-sm font-medium text-zinc-400">{t("provider")}</label>
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
            {t("model")} <span className="text-zinc-600">({t("modelOptional")})</span>
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
            {t("analyzing")}
          </span>
        ) : transcriptReady ? (
          t("analyze")
        ) : (
          t("processingTranscript")
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
              {t("summary")}
            </h3>
            <p className="text-sm text-zinc-200">{result.summary}</p>
          </div>

          {result.weak_questions.length > 0 && (
            <div className="rounded-lg border border-amber-800/40 bg-amber-900/20 p-4">
              <h3 className="mb-2 text-sm font-semibold text-amber-400 uppercase tracking-wide">
                {t("weakQuestions")}
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
                {t("forgottenProjects")}
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
                {t("starImprovements")}
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

function ReindexPanel({ onClose }: { onClose: () => void }) {
  const [folderPath, setFolderPath] = useState("");
  const [indexing, setIndexing] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pathError = folderPath.trim() ? isValidFolderPath(folderPath.trim()) : null;

  const handleIndex = useCallback(async () => {
    const trimmed = folderPath.trim();
    const validationError = isValidFolderPath(trimmed);
    if (validationError) {
      setError(validationError);
      return;
    }
    setIndexing(true);
    setError(null);
    setResult(null);
    try {
      const summary: IndexSummary = await invoke("index_folder_cmd", { path: trimmed });
      setResult(formatIndexResult(summary));
    } catch (e) {
      setError(`${e}`);
    } finally {
      setIndexing(false);
    }
  }, [folderPath]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6 shadow-2xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold text-white">{t("reindexFolder")}</h2>
          <button onClick={onClose} className="text-zinc-400 hover:text-white">✕</button>
        </div>
        <p className="mb-3 text-sm text-zinc-400">
          {t("indexProjectsDescription")}
        </p>
        <input
          type="text"
          value={folderPath}
          onChange={(e) => setFolderPath(e.target.value)}
          placeholder={t("folderPathPlaceholder")}
          className="w-full rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 text-sm text-white placeholder-zinc-500"
        />
        {pathError && folderPath.trim() && (
          <p className="mt-1 text-xs text-amber-400">{pathError}</p>
        )}
        <button
          className="mt-3 w-full rounded-lg bg-emerald-600 py-2 font-medium text-white transition-colors hover:bg-emerald-500 disabled:opacity-50"
          onClick={handleIndex}
          disabled={indexing || !!pathError}
        >
          {indexing ? t("indexing") : t("indexFolder")}
        </button>
        {result && <p className="mt-3 text-sm text-emerald-400">{result}</p>}
        {error && <p className="mt-3 text-sm text-red-400">{error}</p>}
      </div>
    </div>
  );
}

function MainApp({ onLanguageChange }: { onLanguageChange: (lang: Language) => void }) {
  useLanguage();

  const [mode, setMode] = useState<SessionMode>("practice");
  const [company, setCompany] = useState("");
  const [role, setRole] = useState("");
  const [running, setRunning] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [transcript, setTranscript] = useState<TranscriptLine[]>([]);
  const [lastHint, setLastHint] = useState("");
  const [panicking, setPanicking] = useState(false);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [selectedSession, setSelectedSession] = useState<SessionRow | null>(null);
  const [showReindex, setShowReindex] = useState(false);
  const panicTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const transcriptEndRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const unlistenTranscript = listen<TranscriptLine>("new-transcript", (event) => {
      setTranscript((prev) => [...prev, event.payload]);
    });

    const unlistenHint = listen<HintPayload>("new-hint", (event) => {
      setLastHint(event.payload.text);
    });

    const unlistenPanic = listen<PanicPayload>("panic-mode", (event) => {
      setPanicking(true);
      setLastHint("");
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

  useEffect(() => {
    transcriptEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [transcript]);

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
      await invoke("start_session", {
        mode,
        company: company || null,
        role: role || null,
      });
      setTranscript([]);
      setLastHint("");
      setRunning(true);
    } catch (e) {
      console.error("Failed to start session:", e);
    }
  }, [mode, company, role]);

  const handleStop = useCallback(async () => {
    setStopping(true);
    try {
      await invoke("stop_session");
      setRunning(false);
    } catch (e) {
      console.error("Failed to stop session:", e);
    } finally {
      setStopping(false);
    }
  }, []);

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
    <div className="flex min-h-screen flex-col bg-zinc-950 text-white">
      <Header onLanguageChange={onLanguageChange} />

      {panicking && (
        <div className="flex items-center justify-center gap-2 bg-orange-600 px-4 py-2 text-sm font-medium text-white shadow-lg">
          <span className="text-lg">🔇</span>
          {t("panicBanner")}
        </div>
      )}

      <main className="mx-auto w-full max-w-6xl flex-1 p-6">
        <div className="grid gap-6 lg:grid-cols-[1fr_380px]">
          <div className="space-y-6">
            <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-5">
              <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">
                {t("mode")}
              </h2>
              <div className="flex gap-3">
                <button
                  className={`flex-1 rounded-lg py-2.5 font-medium transition-colors ${
                    mode === "practice"
                      ? "bg-blue-600 text-white"
                      : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
                  }`}
                  onClick={() => setMode("practice")}
                  disabled={running}
                >
                  {t("practice")}
                </button>
                <button
                  className={`flex-1 rounded-lg py-2.5 font-medium transition-colors ${
                    mode === "shadow"
                      ? "bg-blue-600 text-white"
                      : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
                  }`}
                  onClick={() => setMode("shadow")}
                  disabled={running}
                >
                  {t("shadow")}
                </button>
              </div>
            </div>

            {!running && (
              <div className="flex gap-3">
                <div className="flex-1">
                  <label className="mb-1 block text-xs font-medium text-zinc-500">
                    {t("company")} <span className="text-zinc-600">({t("optional")})</span>
                  </label>
                  <input
                    type="text"
                    value={company}
                    onChange={(e) => setCompany(e.target.value)}
                    placeholder={t("companyPlaceholder")}
                    className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white placeholder-zinc-600"
                  />
                </div>
                <div className="flex-1">
                  <label className="mb-1 block text-xs font-medium text-zinc-500">
                    {t("role")} <span className="text-zinc-600">({t("optional")})</span>
                  </label>
                  <input
                    type="text"
                    value={role}
                    onChange={(e) => setRole(e.target.value)}
                    placeholder={t("rolePlaceholder")}
                    className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white placeholder-zinc-600"
                  />
                </div>
              </div>
            )}

            <div className="flex gap-3">
              {!running ? (
                <button
                  className="flex-1 rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
                  onClick={handleStart}
                >
                  {t("startSession")}
                </button>
              ) : (
                <button
                  className="flex-1 rounded-lg bg-red-600 py-3 font-medium transition-colors hover:bg-red-500 disabled:opacity-70"
                  onClick={handleStop}
                  disabled={stopping}
                >
                  {stopping ? t("stopping") : t("stopSession")}
                </button>
              )}

              <button
                className={`rounded-lg px-6 py-3 font-medium transition-colors ${
                  panicking
                    ? "bg-orange-500 text-white"
                    : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
                }`}
                onClick={handlePanic}
                disabled={!running || stopping}
              >
                {panicking ? `🔇 10s` : t("panic")}
              </button>

              {!running && (
                <button
                  className="rounded-lg border border-zinc-600 px-4 py-3 text-sm font-medium text-zinc-300 transition-colors hover:border-zinc-500 hover:text-white"
                  onClick={() => setShowReindex(true)}
                >
                  {t("reindexFolder")}
                </button>
              )}
            </div>

            <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-5">
              <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">
                {t("transcript")}
              </h3>
              <div className="h-64 space-y-2 overflow-y-auto pr-2">
                {transcript.length === 0 ? (
                  <p className="text-sm text-zinc-600">{running ? t("listening") : "—"}</p>
                ) : (
                  transcript.map((line, idx) => (
                    <div key={idx} className="rounded-lg bg-zinc-800/50 p-3">
                      <p className="mb-1 text-xs font-medium text-blue-400">
                        {speakerLabel(line.speaker)}
                      </p>
                      <p className="text-sm text-zinc-200">{line.text}</p>
                    </div>
                  ))
                )}
                <div ref={transcriptEndRef} />
              </div>
            </div>

            <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-5">
              <h3 className="mb-2 text-sm font-semibold uppercase tracking-wide text-zinc-500">
                {t("hint")}
              </h3>
              <p className="min-h-[1.25rem] text-sm text-zinc-300">
                {panicking ? t("panicBanner") : lastHint || t("noHint")}
              </p>
            </div>
          </div>

          <div className="space-y-6">
            {selectedSession && (
              <PostCallPanel session={selectedSession} />
            )}

            {!running && sessions.length > 0 && (
              <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-5">
                <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-zinc-500">
                  {selectedSession ? t("previousSessions") : t("sessions")}
                </h2>
                <div className="space-y-2">
                  {sessions.map((s) => (
                    <button
                      key={s.id}
                      className={`w-full rounded-lg border px-4 py-3 text-left text-sm transition-colors ${
                        selectedSession?.id === s.id
                          ? "border-emerald-600 bg-zinc-800 text-white"
                          : "border-zinc-700 bg-zinc-900/50 text-zinc-400 hover:border-zinc-500"
                      }`}
                      onClick={() => setSelectedSession(s)}
                    >
                      {new Date(s.started_at).toLocaleString()} &middot;{" "}
                      {s.mode === "practice" ? t("practice") : t("shadow")} &middot;{" "}
                      {formatLines(s.line_count)}
                      {s.company && (
                        <span className="ml-2 text-zinc-500">
                          &middot; {s.company}{s.role ? ` (${s.role})` : ""}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        </div>
      </main>

      {showReindex && <ReindexPanel onClose={() => setShowReindex(false)} />}
    </div>
  );
}

function App() {
  const [isOverlay, setIsOverlay] = useState(false);
  const [showProvisioning, setShowProvisioning] = useState(true);
  const [showOnboarding, setShowOnboarding] = useState(false);

  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label === "overlay") {
      setIsOverlay(true);
    }
  }, []);

  useEffect(() => {
    initLanguage();
    loadLanguageFromBackend().catch((e) =>
      console.warn("Failed to load language from backend:", e)
    );
  }, []);

  const handleProvisioned = useCallback(async () => {
    try {
      const firstRun: boolean = await invoke("is_first_run");
      if (firstRun) {
        setShowOnboarding(true);
      }
    } catch (e) {
      console.error("is_first_run check failed, skipping onboarding:", e);
    }
    setShowProvisioning(false);
  }, []);

  const handleOnboardingComplete = useCallback(() => {
    setShowOnboarding(false);
  }, []);

  const handleLanguageChange = useCallback((lang: Language) => {
    setLanguage(lang);
    saveLanguage(lang).catch((e) => console.error("Failed to save language:", e));
  }, []);

  if (isOverlay) {
    return <Overlay />;
  }

  if (showProvisioning) {
    return <ProvisioningProgress onProvisioned={handleProvisioned} />;
  }

  if (showOnboarding) {
    return <Onboarding onComplete={handleOnboardingComplete} />;
  }

  return (
    <MainApp onLanguageChange={handleLanguageChange} />
  );
}

export default App;
