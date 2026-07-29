import { useCallback, useEffect, useId, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PROVIDERS } from "./constants";
import { useTauriEvent, useLLMSettings } from "./hooks";
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
import { formatIndexResult, isValidFolderPath, sanitizeError } from "./validation";
import ApiKeyInput from "./ApiKeyInput";
import SettingsDialog from "./SettingsDialog";
import { Icon, type IconName } from "./Icon";
import { Equalizer, SectionLabel, Spinner, StyledSelect } from "./ui";

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

interface PlannedQuestion {
  text: string;
  qtype: string;
  budget_seconds: number;
}

interface InterviewPlanPayload {
  questions: PlannedQuestion[];
}

interface StartSessionResult {
  mic_active: boolean;
  loopback_active: boolean;
  session_id: string | null;
}

type SessionMode = "practice" | "shadow";

/* ---------- Helpers ---------- */

function formatMsCompact(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}m${String(total % 60).padStart(2, "0")}s`;
}

function formatElapsedColon(totalSeconds: number): string {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function TranscriptBubble({ line }: { line: TranscriptLine }) {
  const isUser = line.speaker === "user";
  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"} animate-fade-up`}>
      <div
        className={`max-w-[85%] rounded-2xl px-3.5 py-2.5 ${
          isUser
            ? "rounded-br-md border border-volt-400/20 bg-volt-400/[0.08]"
            : "rounded-bl-md border border-white/5 bg-ink-750"
        }`}
      >
        <p
          className={`mb-0.5 flex items-baseline gap-2 font-mono text-[10px] font-medium uppercase tracking-[0.14em] ${
            isUser ? "text-volt-400" : "text-signal-blue"
          }`}
        >
          {speakerLabel(line.speaker)}
          <span className="font-normal normal-case tracking-normal text-zinc-500">
            {formatMsCompact(line.started_at_ms)}
          </span>
        </p>
        <p className="text-sm leading-relaxed text-zinc-100">{line.text}</p>
      </div>
    </div>
  );
}

function GhostButton({
  onClick,
  disabled,
  icon,
  children,
  title,
  danger,
  className,
}: {
  onClick: () => void | Promise<void>;
  disabled?: boolean;
  icon?: IconName;
  children: React.ReactNode;
  title?: string;
  danger?: boolean;
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`inline-flex items-center justify-center gap-2 rounded-lg border px-4 py-2.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
        danger
          ? "border-signal-red/40 text-signal-red hover:bg-signal-red/10"
          : "border-white/10 text-zinc-300 hover:border-white/25 hover:text-white"
      } ${className ?? ""}`}
    >
      {icon && <Icon name={icon} className="h-4 w-4" />}
      {children}
    </button>
  );
}

/* ---------- Post-call panel ---------- */

export function PostCallPanel({ session, onOpenSettings }: { session: SessionRow; onOpenSettings?: (tab?: "api-keys" | "llm-defaults" | "general") => void }) {
  const [transcriptReady, setTranscriptReady] = useState(false);
  const [checking, setChecking] = useState(true);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const { provider, setProvider, model, setModel } = useLLMSettings("analyze");
  const [analyzing, setAnalyzing] = useState(false);
  const [result, setResult] = useState<AnalyzeResult | null>(null);
  const [error, setError] = useState("");
  const [hasSavedKey, setHasSavedKey] = useState(false);
  const [showTranscript, setShowTranscript] = useState(false);
  const [transcriptLines, setTranscriptLines] = useState<TranscriptLine[]>([]);
  const [loadingTranscript, setLoadingTranscript] = useState(false);
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
    if (transcriptReady || processingError) return;
    const interval = setInterval(async () => {
      try {
        const ready: boolean = await invoke("is_transcript_ready", {
          sessionId: session.id,
        });
        if (ready) {
          setTranscriptReady(true);
          setChecking(false);
        }
      } catch {
        // keep polling
      }
    }, 5000);
    return () => clearInterval(interval);
  }, [session.id, transcriptReady, processingError]);

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
    const unlistenErr = listen<{ session_id: string; message: string }>(
      "post-call-transcript-error",
      (event) => {
        if (event.payload.session_id === session.id) {
          setProcessingError(event.payload.message);
          setChecking(false);
        }
      }
    );
    return () => {
      unlistenErr.then((fn) => fn());
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
      setError(sanitizeError(e));
    } finally {
      setAnalyzing(false);
    }
  }, [session.id, provider, model]);

  const handleToggleTranscript = useCallback(async () => {
    if (showTranscript) {
      setShowTranscript(false);
      return;
    }
    if (transcriptLines.length === 0) {
      setLoadingTranscript(true);
      try {
        const lines: TranscriptLine[] = await invoke("get_session_transcript", {
          sessionId: session.id,
        });
        setTranscriptLines(lines);
      } catch (e) {
        console.error("Failed to load transcript:", e);
      } finally {
        setLoadingTranscript(false);
      }
    }
    setShowTranscript(true);
  }, [showTranscript, transcriptLines.length, session.id]);

  const modeLabel = session.mode === "practice" ? t("practice") : t("shadow");
  const dateStr = new Date(session.started_at).toLocaleString();

  return (
    <section className="relative z-10 w-full rounded-2xl border border-white/5 bg-ink-900 p-6 shadow-card animate-fade-up">
      <SectionLabel>{t("postCall")}</SectionLabel>

      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <h3 className="text-base font-semibold text-white">{dateStr}</h3>
        <span className="rounded-full border border-white/10 bg-ink-750 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-zinc-400">
          {modeLabel}
        </span>
      </div>
      <p className="mt-1 text-xs text-zinc-500">
        {formatLines(session.line_count)}
        {session.company && (
          <span> &middot; {session.company}{session.role ? ` (${session.role})` : ""}</span>
        )}
      </p>

      <div className="mt-4">
        {processingError ? (
          <div className="rounded-xl border border-signal-red/30 bg-signal-red/[0.07] p-3.5" role="alert">
            <p className="mb-2.5 flex items-start gap-2 text-sm text-signal-red">
              <Icon name="alert" className="mt-0.5 h-4 w-4" />
              {processingError}
            </p>
            <div className="flex gap-2">
              <GhostButton
                icon="refresh"
                className="px-3 py-1.5 text-xs"
                onClick={async () => {
                  setProcessingError(null);
                  setChecking(true);
                  try {
                    const ready: boolean = await invoke("is_transcript_ready", {
                      sessionId: session.id,
                    });
                    setTranscriptReady(ready);
                  } catch {
                    // stay in checking/error
                  } finally {
                    setChecking(false);
                    if (transcriptReady) setProcessingError(null);
                  }
                }}
              >
                {t("retry")}
              </GhostButton>
              <GhostButton
                icon="file-text"
                className="px-3 py-1.5 text-xs"
                onClick={async () => {
                  try {
                    const logDir: string = await invoke("get_log_dir_path");
                    try {
                      const { open } = await import("@tauri-apps/plugin-shell");
                      await open(`file://${logDir}`);
                    } catch {
                      alert(`Log folder: ${logDir}`);
                    }
                  } catch {
                    // silent
                  }
                }}
              >
                {t("viewLogs")}
              </GhostButton>
            </div>
          </div>
        ) : checking ? (
          <p className="flex items-center gap-2 text-sm text-zinc-500">
            <Spinner className="h-3.5 w-3.5" />
            {t("processingTranscript")}
          </p>
        ) : transcriptReady ? (
          <p className="flex items-center gap-2 text-sm text-volt-400">
            <Icon name="check" className="h-4 w-4" />
            {t("transcriptComplete")}
          </p>
        ) : (
          <p className="flex items-center gap-2 text-sm text-signal-amber">
            <span aria-hidden="true" className="inline-block h-2 w-2 animate-pulse rounded-full bg-signal-amber" />
            {t("processingTranscript")}
          </p>
        )}
      </div>

      {transcriptReady && (
        <div className="mt-4">
          <GhostButton
            icon={showTranscript ? "eye-off" : "file-text"}
            className="px-3 py-1.5 text-xs"
            onClick={handleToggleTranscript}
          >
            {showTranscript ? t("hideTranscript") : t("viewTranscript")}
          </GhostButton>

          {showTranscript && (
            <div className="mt-3 max-h-72 space-y-2 overflow-y-auto rounded-xl border border-white/5 bg-ink-850 p-3">
              {loadingTranscript ? (
                <p className="flex items-center justify-center gap-2 py-6 text-sm text-zinc-500">
                  <Spinner className="h-3.5 w-3.5" />
                  {t("loading") + "..."}
                </p>
              ) : transcriptLines.length === 0 ? (
                <p className="py-6 text-center text-sm text-zinc-500">{t("noTranscriptLines")}</p>
              ) : (
                transcriptLines.map((line, i) => (
                  <TranscriptBubble key={`tl-${i}-${line.started_at_ms}`} line={line} />
                ))
              )}
            </div>
          )}
        </div>
      )}

      <div className="mt-5">
        <ApiKeyInput
          provider={provider}
          hasSavedKey={hasSavedKey}
          onKeySaved={() => setHasSavedKey(true)}
        />
      </div>

      <div className="mb-4 grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
            {t("provider")}
          </label>
          <StyledSelect
            value={provider}
            onChange={setProvider}
            options={PROVIDERS}
            ariaLabel={t("provider")}
          />
        </div>
        <div>
          <label htmlFor="analyze-model" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
            {t("model")} <span className="normal-case text-zinc-600">({t("modelOptional")})</span>
          </label>
          <input
            id="analyze-model"
            type="text"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder="default"
            className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
          />
        </div>
      </div>

      <button
        type="button"
        onClick={() => onOpenSettings?.("llm-defaults")}
        className="mb-4 flex items-center gap-1.5 font-mono text-[10px] font-medium uppercase tracking-wider text-zinc-500 transition-colors hover:text-volt-400"
      >
        <Icon name="sliders" className="h-3 w-3" />
        {t("configureAllInSettings")}
      </button>

      <button
        className={`flex w-full items-center justify-center gap-2 rounded-xl py-3 font-semibold transition-all ${
          !transcriptReady || analyzing
            ? "cursor-not-allowed bg-ink-700 text-zinc-500"
            : "bg-volt-400 text-ink-950 shadow-volt hover:bg-volt-300 active:scale-[0.99]"
        }`}
        onClick={handleAnalyze}
        disabled={!transcriptReady || analyzing}
      >
        {analyzing ? (
          <>
            <Spinner className="h-4 w-4 border-zinc-500 border-t-ink-950" />
            {t("analyzing")}
          </>
        ) : transcriptReady ? (
          <>
            <Icon name="sparkle" className="h-4 w-4" />
            {t("analyze")}
          </>
        ) : (
          t("processingTranscript")
        )}
      </button>

      {error && (
        <div className="mt-4 rounded-xl border border-signal-red/30 bg-signal-red/[0.07] p-3.5" role="alert">
          <p className="flex items-start gap-2 text-sm text-signal-red">
            <Icon name="alert" className="mt-0.5 h-4 w-4" />
            {error}
          </p>
        </div>
      )}

      {result && (
        <div className="mt-5 space-y-3.5 animate-fade-up">
          <div className="rounded-xl border border-white/5 bg-ink-850 p-4">
            <h4 className="mb-2 flex items-center gap-2 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-400">
              <Icon name="file-text" className="h-3.5 w-3.5" />
              {t("summary")}
            </h4>
            <p className="text-sm leading-relaxed text-zinc-200">{result.summary}</p>
          </div>

          {result.weak_questions.length > 0 && (
            <div className="rounded-xl border border-signal-amber/25 bg-signal-amber/[0.06] p-4">
              <h4 className="mb-2 flex items-center gap-2 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-signal-amber">
                <Icon name="alert" className="h-3.5 w-3.5" />
                {t("weakQuestions")}
              </h4>
              <ul className="list-disc space-y-1.5 pl-4 text-sm leading-relaxed text-zinc-300 marker:text-signal-amber/60">
                {result.weak_questions.map((q) => (
                  <li key={`weak-${q.slice(0, 20)}`}>{q}</li>
                ))}
              </ul>
            </div>
          )}

          {result.forgotten_projects.length > 0 && (
            <div className="rounded-xl border border-signal-blue/25 bg-signal-blue/[0.06] p-4">
              <h4 className="mb-2 flex items-center gap-2 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-signal-blue">
                <Icon name="folder" className="h-3.5 w-3.5" />
                {t("forgottenProjects")}
              </h4>
              <ul className="list-disc space-y-1.5 pl-4 text-sm leading-relaxed text-zinc-300 marker:text-signal-blue/60">
                {result.forgotten_projects.map((p) => (
                  <li key={`fp-${p.slice(0, 20)}`}>{p}</li>
                ))}
              </ul>
            </div>
          )}

          {result.star_improvements.length > 0 && (
            <div className="rounded-xl border border-signal-violet/25 bg-signal-violet/[0.06] p-4">
              <h4 className="mb-2 flex items-center gap-2 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-signal-violet">
                <Icon name="sparkle" className="h-3.5 w-3.5" />
                {t("starImprovements")}
              </h4>
              <ul className="list-disc space-y-1.5 pl-4 text-sm leading-relaxed text-zinc-300 marker:text-signal-violet/60">
                {result.star_improvements.map((s) => (
                  <li key={`star-${s.slice(0, 20)}`}>{s}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

/* ---------- Re-index dialog ---------- */

function ReindexPanel({ onClose }: { onClose: () => void }) {
  const [folderPath, setFolderPath] = useState("");
  const [indexing, setIndexing] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const pathError = folderPath.trim() ? isValidFolderPath(folderPath.trim()) : null;

  useEffect(() => {
    inputRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

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
      setError(sanitizeError(e));
    } finally {
      setIndexing(false);
    }
  }, [folderPath]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-ink-950/80 p-4 backdrop-blur-sm animate-fade-in"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="w-full max-w-lg rounded-2xl border border-white/10 bg-ink-900 p-6 shadow-pop animate-scale-in"
      >
        <div className="mb-1 flex items-center justify-between">
          <h2 id={titleId} className="flex items-center gap-2.5 text-lg font-semibold text-white">
            <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-volt-400/10 text-volt-400">
              <Icon name="folder" className="h-4 w-4" />
            </span>
            {t("reindexFolder")}
          </h2>
          <button
            onClick={onClose}
            aria-label={t("close")}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-zinc-500 transition-colors hover:bg-white/5 hover:text-white"
          >
            <Icon name="x" className="h-4 w-4" />
          </button>
        </div>
        <p className="mb-4 text-sm leading-relaxed text-zinc-400">
          {t("indexProjectsDescription")}
        </p>
        <label htmlFor="reindex-path" className="visually-hidden">
          {t("folderPathLabel")}
        </label>
        <input
          id="reindex-path"
          ref={inputRef}
          type="text"
          value={folderPath}
          onChange={(e) => setFolderPath(e.target.value)}
          placeholder={t("folderPathPlaceholder")}
          aria-invalid={!!pathError && !!folderPath.trim()}
          className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2.5 font-mono text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
        />
        {pathError && folderPath.trim() && (
          <p className="mt-1.5 flex items-center gap-1.5 text-xs text-signal-amber" role="alert">
            <Icon name="alert" className="h-3.5 w-3.5" />
            {pathError}
          </p>
        )}
        <button
          className="mt-4 flex w-full items-center justify-center gap-2 rounded-xl bg-volt-400 py-2.5 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99] disabled:cursor-not-allowed disabled:bg-ink-700 disabled:text-zinc-500 disabled:shadow-none"
          onClick={handleIndex}
          disabled={indexing || !!pathError}
        >
          {indexing ? (
            <>
              <Spinner className="h-4 w-4 border-zinc-500 border-t-ink-950" />
              {t("indexing")}
            </>
          ) : (
            t("indexFolder")
          )}
        </button>
        {result && (
          <p className="mt-3 flex items-center gap-2 text-sm text-volt-400">
            <Icon name="check" className="h-4 w-4" />
            {result}
          </p>
        )}
        {error && (
          <p className="mt-3 flex items-start gap-2 text-sm text-signal-red" role="alert">
            <Icon name="alert" className="mt-0.5 h-4 w-4" />
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

/* ---------- Plan generator (AI Interview) ---------- */

function PlanGenerator({
  jobDescription,
  setJobDescription,
  interviewPlan,
  setInterviewPlan,
  onOpenSettings,
}: {
  jobDescription: string;
  setJobDescription: (v: string) => void;
  interviewPlan: PlannedQuestion[] | null;
  setInterviewPlan: (v: PlannedQuestion[] | null) => void;
  onOpenSettings?: (tab?: "api-keys" | "llm-defaults" | "general") => void;
}) {
  const [durationMinutes, setDurationMinutes] = useState(15);
  const { provider: planProvider, setProvider: setPlanProvider, model: planModel, setModel: setPlanModel } = useLLMSettings("plan");
  const [generatingPlan, setGeneratingPlan] = useState(false);
  const [planError, setPlanError] = useState<string | null>(null);
  const [hasPlanKey, setHasPlanKey] = useState(false);

  useEffect(() => {
    invoke<boolean>("has_key", { provider: planProvider })
      .then(setHasPlanKey)
      .catch(() => setHasPlanKey(false));
  }, [planProvider]);

  const handleGeneratePlan = useCallback(async () => {
    if (!jobDescription.trim()) return;
    if (generatingPlan) return;
    setGeneratingPlan(true);
    setPlanError(null);
    setInterviewPlan(null);
    try {
      const plan: InterviewPlanPayload = await invoke("generate_interview_plan", {
        jobDescription: jobDescription.trim(),
        durationMinutes,
        provider: planProvider,
        model: planModel.trim() || null,
      });
      setInterviewPlan(plan.questions);
    } catch (e) {
      setPlanError(sanitizeError(e));
    } finally {
      setGeneratingPlan(false);
    }
  }, [jobDescription, durationMinutes, planProvider, planModel, generatingPlan, setInterviewPlan]);

  return (
    <section className="relative z-10 rounded-2xl border border-white/5 bg-ink-900 p-6 shadow-card animate-fade-up">
      <div className="mb-1 flex items-center gap-2.5">
        <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-volt-400/10 text-volt-400">
          <Icon name="sparkle" className="h-4 w-4" />
        </span>
        <h2 className="text-base font-semibold text-white">{t("aiInterview")}</h2>
      </div>
      <p className="mb-4 text-xs leading-relaxed text-zinc-500">{t("planDesc")}</p>

      <div className="mb-3">
        <label htmlFor="job-description" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
          {t("jobDescription")}
        </label>
        <textarea
          id="job-description"
          value={jobDescription}
          onChange={(e) => setJobDescription(e.target.value)}
          placeholder={t("jobDescriptionPlaceholder")}
          rows={4}
          className="w-full resize-none rounded-lg border border-white/10 bg-ink-750 px-3 py-2.5 text-sm leading-relaxed text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
        />
      </div>

      <div className="mb-4 grid grid-cols-3 gap-3">
        <div>
          <label htmlFor="plan-duration" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
            {t("durationMinutes")}
          </label>
          <input
            id="plan-duration"
            type="number"
            min={5}
            max={120}
            value={durationMinutes}
            onChange={(e) => setDurationMinutes(Math.max(5, parseInt(e.target.value) || 15))}
            className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white transition-colors focus:border-volt-400/50"
          />
        </div>
        <div>
          <label className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
            {t("provider")}
          </label>
          <StyledSelect
            value={planProvider}
            onChange={setPlanProvider}
            options={PROVIDERS}
            ariaLabel={t("provider")}
          />
        </div>
        <div>
          <label htmlFor="plan-model" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
            {t("model")} <span className="normal-case text-zinc-600">({t("modelOptional")})</span>
          </label>
          <input
            id="plan-model"
            type="text"
            value={planModel}
            onChange={(e) => setPlanModel(e.target.value)}
            placeholder={planProvider === "ollama" ? "llama3.1" : "default"}
            className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
          />
        </div>
      </div>

      {!hasPlanKey && planProvider !== "ollama" && (
        <div className="mb-4 rounded-lg border border-signal-amber/25 bg-signal-amber/[0.06] p-3">
          <p className="flex items-start gap-2 text-xs text-signal-amber">
            <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            {t("noKeyForProviderMsg", { provider: planProvider })}
          </p>
          <button
            type="button"
            onClick={() => onOpenSettings?.("api-keys")}
            className="mt-1.5 flex items-center gap-1.5 font-mono text-[10px] font-medium uppercase tracking-wider text-signal-amber transition-colors hover:text-signal-amber/70"
          >
            <Icon name="sliders" className="h-3 w-3" />
            {t("openSettings")}
          </button>
        </div>
      )}

      <button
        type="button"
        onClick={() => onOpenSettings?.("llm-defaults")}
        className="mb-3 flex items-center gap-1.5 font-mono text-[10px] font-medium uppercase tracking-wider text-zinc-500 transition-colors hover:text-volt-400"
      >
        <Icon name="sliders" className="h-3 w-3" />
        {t("configureAllInSettings")}
      </button>

      <button
        className="flex w-full items-center justify-center gap-2 rounded-xl bg-volt-400 py-2.5 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99] disabled:cursor-not-allowed disabled:bg-ink-700 disabled:text-zinc-500 disabled:shadow-none"
        onClick={handleGeneratePlan}
        disabled={generatingPlan || !jobDescription.trim()}
      >
        {generatingPlan ? (
          <>
            <Spinner className="h-4 w-4 border-zinc-500 border-t-ink-950" />
            {t("generating") + "..."}
          </>
        ) : (
          <>
            <Icon name="sparkle" className="h-4 w-4" />
            {t("generateQuestions")}
          </>
        )}
      </button>

      {planError && (
        <p className="mt-2.5 flex items-start gap-2 text-xs text-signal-red" role="alert">
          <Icon name="alert" className="mt-0.5 h-3.5 w-3.5" />
          {planError}
        </p>
      )}

      {interviewPlan && (
        <div className="mt-4 animate-fade-up">
          <p className="mb-2.5 flex items-center gap-2 text-xs font-medium text-volt-400">
            <Icon name="check" className="h-3.5 w-3.5" />
            {interviewPlan.length} {t("questionsGenerated")}
          </p>
          <ol className="space-y-2">
            {interviewPlan.map((q, i) => (
              <li
                key={`q-${i}-${q.text.slice(0, 40)}`}
                className="flex gap-3 rounded-xl border border-white/5 bg-ink-850 p-3.5"
              >
                <span className="font-mono text-xs font-semibold tabular-nums text-volt-400/80">
                  {String(i + 1).padStart(2, "0")}
                </span>
                <div className="min-w-0">
                  <p className="text-sm leading-relaxed text-zinc-200">{q.text}</p>
                  <p className="mt-1.5 flex flex-wrap gap-1.5">
                    <span className="rounded-full border border-white/10 bg-ink-750 px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider text-zinc-400">
                      {q.qtype}
                    </span>
                    <span className="flex items-center gap-1 rounded-full border border-white/10 bg-ink-750 px-2 py-0.5 font-mono text-[10px] tabular-nums text-zinc-400">
                      <Icon name="clock" className="h-3 w-3" />
                      {q.budget_seconds}s
                    </span>
                  </p>
                </div>
              </li>
            ))}
          </ol>
        </div>
      )}
    </section>
  );
}

/* ---------- Live AI interview ---------- */

function LiveInterview() {
  const [aiQuestion, setAiQuestion] = useState<{ index: number; total: number; text: string } | null>(null);
  const [aiStatus, setAiStatus] = useState<"speaking" | "listening" | "finished" | null>(null);

  useTauriEvent<{ question_index: number; total_questions: number; text: string }>(
    "interview-question",
    (payload) => {
      setAiQuestion({
        index: payload.question_index,
        total: payload.total_questions,
        text: payload.text,
      });
    },
  );

  useTauriEvent<{ status: string }>("interview-status", (payload) => {
    setAiStatus(payload.status as "speaking" | "listening" | "finished");
  });

  useTauriEvent<{ session_id: string }>("interview-finished", () => {
    setAiStatus("finished");
  });

  if (!aiQuestion) return null;

  return (
    <section className="rounded-2xl border border-volt-400/30 bg-volt-400/[0.04] p-6 shadow-volt animate-fade-up">
      <div className="mb-3 flex items-center justify-between">
        <h2 className="flex items-center gap-2 font-mono text-[11px] font-medium uppercase tracking-[0.18em] text-volt-400">
          <Icon name="sparkle" className="h-3.5 w-3.5" />
          {t("aiInterview")}
        </h2>
        <span className="font-mono text-[11px] tabular-nums text-zinc-400">
          {t("question")} {aiQuestion.index + 1}/{aiQuestion.total}
        </span>
      </div>

      <div aria-hidden="true" className="mb-4 h-1 w-full overflow-hidden rounded-full bg-ink-700">
        <div
          className="h-full rounded-full bg-volt-400 transition-all duration-500"
          style={{ width: `${((aiQuestion.index + 1) / aiQuestion.total) * 100}%` }}
        />
      </div>

      <p className="text-lg font-medium leading-relaxed text-white">{aiQuestion.text}</p>

      <p className="mt-3 flex items-center gap-2 text-xs text-zinc-400">
        {aiStatus === "listening" && <Equalizer />}
        {aiStatus ? t(`interviewStatus_${aiStatus}`) : "—"}
      </p>

      {aiStatus !== "finished" && (
        <div className="mt-4 flex gap-2">
          <GhostButton
            icon="skip"
            className="px-3 py-1.5 text-xs"
            onClick={async () => {
              try { await invoke("skip_ai_question"); } catch { console.warn("Failed to skip question"); }
            }}
          >
            {t("skipQuestion")}
          </GhostButton>
          <GhostButton
            icon="stop"
            danger
            className="px-3 py-1.5 text-xs"
            onClick={async () => {
              try { await invoke("stop_ai_interview"); } catch { console.warn("Failed to stop interview"); }
            }}
          >
            {t("endInterview")}
          </GhostButton>
        </div>
      )}
    </section>
  );
}

/* ---------- Session list (right rail) ---------- */

function SessionList({
  sessions,
  selectedSession,
  onSelect,
}: {
  sessions: SessionRow[];
  selectedSession: SessionRow | null;
  onSelect: (s: SessionRow) => void;
}) {
  if (sessions.length === 0) return null;

  return (
    <section className="rounded-2xl border border-white/5 bg-ink-900 p-5 shadow-card animate-fade-up">
      <div className="mb-3.5 flex items-center gap-2.5 px-1">
        <Icon name="history" className="h-4 w-4 text-zinc-500" />
        <h2 className="font-mono text-[11px] font-medium uppercase tracking-[0.18em] text-zinc-400">
          {selectedSession ? t("previousSessions") : t("sessions")}
        </h2>
      </div>
      <div className="space-y-2">
        {sessions.map((s) => {
          const selected = selectedSession?.id === s.id;
          return (
            <button
              key={s.id}
              aria-pressed={selected}
              className={`w-full rounded-xl border px-4 py-3 text-left transition-all ${
                selected
                  ? "border-volt-400/40 bg-volt-400/[0.05]"
                  : "border-white/5 bg-ink-850 hover:border-white/15"
              }`}
              onClick={() => onSelect(s)}
            >
              <span className="flex items-center justify-between gap-2">
                <span className="font-mono text-[11px] tabular-nums text-zinc-400">
                  {new Date(s.started_at).toLocaleString()}
                </span>
                <span
                  className={`rounded-full px-2 py-0.5 font-mono text-[10px] uppercase tracking-wider ${
                    s.mode === "practice"
                      ? "bg-volt-400/10 text-volt-400"
                      : "bg-signal-blue/10 text-signal-blue"
                  }`}
                >
                  {s.mode === "practice" ? t("practice") : t("shadow")}
                </span>
              </span>
              <span className="mt-1 block truncate text-sm font-medium text-zinc-200">
                {s.company || (s.mode === "practice" ? t("practice") : t("shadow"))}
                {s.role ? <span className="text-zinc-500"> · {s.role}</span> : null}
              </span>
              <span className="mt-0.5 block text-xs text-zinc-500">
                {formatLines(s.line_count)}
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}

/* ---------- Main app ---------- */

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
  const [panicUntil, setPanicUntil] = useState(0);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [selectedSession, setSelectedSession] = useState<SessionRow | null>(null);
  const [showReindex, setShowReindex] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [settingsTab, setSettingsTab] = useState<"api-keys" | "llm-defaults" | "general">("api-keys");
  // Interview plan state
  const [jobDescription, setJobDescription] = useState("");
  const [interviewPlan, setInterviewPlan] = useState<PlannedQuestion[] | null>(null);
  // Session runtime
  const [startingSession, setStartingSession] = useState(false);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const panicTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const transcriptEndRef = useRef<HTMLDivElement | null>(null);

  useTauriEvent<TranscriptLine>("new-transcript", (payload) => {
    setTranscript((prev) => [...prev, payload]);
  });

  useTauriEvent<HintPayload>("new-hint", (payload) => {
    setLastHint(payload.text);
  });

  useTauriEvent<PanicPayload>("panic-mode", (payload) => {
    setPanicking(true);
    setLastHint("");
    setPanicUntil(Date.now() + payload.until_secs * 1000);
    if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
    const ms = payload.until_secs * 1000;
    panicTimerRef.current = setTimeout(() => setPanicking(false), ms);
  });

  // Compute remaining panic seconds for the button display
  const panicRemaining = panicking
    ? Math.max(0, Math.ceil((panicUntil - Date.now()) / 1000))
    : 0;

  // Tick the remaining panic counter every second
  useEffect(() => {
    if (!panicking) return;
    const id = setInterval(() => {
      if (Date.now() >= panicUntil) {
        setPanicking(false);
      }
    }, 1000);
    return () => clearInterval(id);
  }, [panicking, panicUntil]);

  useEffect(() => {
    transcriptEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [transcript]);

  // Live session clock
  useEffect(() => {
    if (!running || startedAt === null) return;
    const id = setInterval(() => {
      setElapsed(Math.max(0, Math.floor((Date.now() - startedAt) / 1000)));
    }, 1000);
    return () => clearInterval(id);
  }, [running, startedAt]);

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
    if (startingSession || stopping) return;
    setStartingSession(true);
    try {
      const result: StartSessionResult = await invoke("start_session", {
        mode,
        company: company || null,
        role: role || null,
      });
      setTranscript([]);
      setLastHint("");
      setRunning(true);
      setStartedAt(Date.now());
      setElapsed(0);

      if (mode === "practice" && interviewPlan && result.session_id) {
        try {
          await invoke("start_ai_interview", {
            sessionId: result.session_id,
            jobDescription: jobDescription.trim(),
            interviewPlanJson: JSON.stringify({ questions: interviewPlan }),
          });
        } catch (e) {
          console.error("Failed to start AI interview:", e);
        }
      }
    } catch (e) {
      console.error("Failed to start session:", e);
    } finally {
      setStartingSession(false);
    }
  }, [mode, company, role, interviewPlan, jobDescription, startingSession, stopping]);

  const handleStop = useCallback(async () => {
    setStopping(true);
    try {
      await invoke("stop_session");
      setRunning(false);
      setStartedAt(null);
    } catch (e) {
      console.error("Failed to stop session:", e);
    } finally {
      setStopping(false);
    }
  }, []);

  useEffect(() => {
    if (!running) {
      const timer = setTimeout(async () => {
        try {
          const list: SessionRow[] = await invoke("get_sessions");
          setSessions(list);
          if (list.length > 0) {
            setSelectedSession(list[0]);
          }
        } catch (e) {
          console.error("Failed to load sessions:", e);
        }
      }, 500);
      return () => clearTimeout(timer);
    }
  }, [running]);

  const handlePanic = useCallback(async () => {
    try {
      await invoke("panic_mode");
    } catch (e) {
      console.error("Failed to activate panic mode:", e);
    }
  }, []);

  const modeOptions: { value: SessionMode; icon: IconName; descKey: "practiceDesc" | "shadowDesc" }[] = [
    { value: "practice", icon: "mic", descKey: "practiceDesc" },
    { value: "shadow", icon: "eye", descKey: "shadowDesc" },
  ];

  const handleOpenSettings = useCallback((tab?: "api-keys" | "llm-defaults" | "general") => {
    if (tab) setSettingsTab(tab);
    setShowSettings(true);
  }, []);

  return (
    <div className="flex min-h-screen flex-col text-white">
      <Header onLanguageChange={onLanguageChange} onOpenSettings={() => handleOpenSettings()} />

      {panicking && (
        <div className="flex items-center justify-center gap-2 border-b border-signal-amber/30 bg-signal-amber/10 px-4 py-2 text-sm font-medium text-signal-amber animate-fade-in" role="alert">
          <Icon name="mute" className="h-4 w-4" />
          {t("panicBanner")}
        </div>
      )}

      <main className="mx-auto w-full max-w-6xl flex-1 px-6 py-7">
        <div className="grid gap-6 lg:grid-cols-[1fr_380px]">
          <div className="space-y-6">

            {/* ============ Session console ============ */}
            <section className="rounded-2xl border border-white/5 bg-ink-900 p-6 shadow-card animate-fade-up">
              <div className="mb-5 flex items-center justify-between">
                <SectionLabel>{t("sessionSetup")}</SectionLabel>
                {running && (
                  <span className="flex items-center gap-2 rounded-full border border-signal-red/30 bg-signal-red/10 px-3 py-1 font-mono text-[11px] font-medium uppercase tracking-widest text-signal-red">
                    <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-signal-red animate-pulse-dot" />
                    {t("live")}
                    <span aria-hidden="true" className="h-3 w-px bg-signal-red/30" />
                    <span className="tabular-nums">{formatElapsedColon(elapsed)}</span>
                  </span>
                )}
              </div>

              <div role="radiogroup" aria-label={t("mode")} className="grid gap-3 sm:grid-cols-2">
                {modeOptions.map(({ value, icon, descKey }) => {
                  const selected = mode === value;
                  return (
                    <button
                      key={value}
                      role="radio"
                      aria-checked={selected}
                      disabled={running}
                      onClick={() => setMode(value)}
                      className={`group relative rounded-xl border p-4 text-left transition-all disabled:cursor-not-allowed disabled:opacity-60 ${
                        selected
                          ? "border-volt-400/50 bg-volt-400/[0.06] shadow-volt"
                          : "border-white/10 bg-ink-800 hover:border-white/25"
                      }`}
                    >
                      <span className="mb-2.5 flex items-center justify-between">
                        <span
                          className={`flex h-9 w-9 items-center justify-center rounded-lg transition-colors ${
                            selected ? "bg-volt-400 text-ink-950" : "bg-ink-700 text-zinc-400 group-hover:text-zinc-200"
                          }`}
                        >
                          <Icon name={icon} className="h-5 w-5" />
                        </span>
                        {selected && (
                          <span className="flex h-5 w-5 items-center justify-center rounded-full bg-volt-400 text-ink-950">
                            <Icon name="check" className="h-3 w-3" strokeWidth={2.5} />
                          </span>
                        )}
                      </span>
                      <span className={`block text-sm font-semibold ${selected ? "text-white" : "text-zinc-300"}`}>
                        {t(value)}
                      </span>
                      <span className="mt-0.5 block text-xs leading-relaxed text-zinc-500">
                        {t(descKey)}
                      </span>
                    </button>
                  );
                })}
              </div>

              {!running && (
                <div className="mt-4 grid gap-3 sm:grid-cols-2">
                  <div>
                    <label htmlFor="session-company" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
                      {t("company")} <span className="normal-case text-zinc-600">({t("optional")})</span>
                    </label>
                    <input
                      id="session-company"
                      type="text"
                      value={company}
                      onChange={(e) => setCompany(e.target.value)}
                      placeholder={t("companyPlaceholder")}
                      className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2.5 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
                    />
                  </div>
                  <div>
                    <label htmlFor="session-role" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
                      {t("role")} <span className="normal-case text-zinc-600">({t("optional")})</span>
                    </label>
                    <input
                      id="session-role"
                      type="text"
                      value={role}
                      onChange={(e) => setRole(e.target.value)}
                      placeholder={t("rolePlaceholder")}
                      className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2.5 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
                    />
                  </div>
                </div>
              )}

              <div className="mt-5 flex flex-wrap gap-3">
                {!running ? (
                  <button
                    className="flex flex-1 items-center justify-center gap-2 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 shadow-volt transition-all hover:bg-volt-300 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-40 disabled:shadow-none"
                    onClick={handleStart}
                    disabled={startingSession || stopping}
                  >
                    {startingSession ? (
                      <>
                        <Spinner className="h-4 w-4 border-ink-950/30 border-t-ink-950" />
                        {t("starting") + "..."}
                      </>
                    ) : (
                      <>
                        <Icon name="play" className="h-4 w-4" />
                        {t("startSession")}
                      </>
                    )}
                  </button>
                ) : (
                  <button
                    className="flex flex-1 items-center justify-center gap-2 rounded-xl bg-signal-red py-3 font-semibold text-ink-950 transition-all hover:brightness-110 active:scale-[0.99] disabled:cursor-not-allowed disabled:opacity-60"
                    onClick={handleStop}
                    disabled={stopping}
                  >
                    {stopping ? (
                      <>
                        <Spinner className="h-4 w-4 border-ink-950/30 border-t-ink-950" />
                        {t("stopping")}
                      </>
                    ) : (
                      <>
                        <Icon name="stop" className="h-4 w-4" />
                        {t("stopSession")}
                      </>
                    )}
                  </button>
                )}

                <button
                  type="button"
                  title={t("panicTooltip")}
                  aria-pressed={panicking}
                  className={`inline-flex items-center justify-center gap-2 rounded-xl border px-5 py-3 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                    panicking
                      ? "border-signal-amber/50 bg-signal-amber/15 text-signal-amber"
                      : "border-white/10 text-zinc-300 hover:border-white/25 hover:text-white"
                  }`}
                  onClick={handlePanic}
                  disabled={!running || stopping}
                >
                  <Icon name="mute" className="h-4 w-4" />
                  {panicking ? <span className="font-mono tabular-nums">{panicRemaining}s</span> : t("panic")}
                </button>

                {!running && (
                  <GhostButton icon="folder" onClick={() => setShowReindex(true)} className="px-5 py-3">
                    {t("reindexFolder")}
                  </GhostButton>
                )}
              </div>
            </section>

            {/* ============ Live AI question ============ */}
            {running && <LiveInterview />}

            {/* ============ Plan generator ============ */}
            {!running && mode === "practice" && (
              <PlanGenerator
                jobDescription={jobDescription}
                setJobDescription={setJobDescription}
                interviewPlan={interviewPlan}
                setInterviewPlan={setInterviewPlan}
                onOpenSettings={handleOpenSettings}
              />
            )}

            {/* ============ Transcript ============ */}
            <section className="rounded-2xl border border-white/5 bg-ink-900 p-6 shadow-card animate-fade-up">
              <div className="mb-4 flex items-center justify-between">
                <SectionLabel>{t("transcript")}</SectionLabel>
                {running && <Equalizer className="h-3.5 w-4" />}
              </div>
              <div
                role="log"
                aria-label={t("transcript")}
                className="h-64 space-y-2.5 overflow-y-auto pr-2"
              >
                {transcript.length === 0 ? (
                  <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
                    {running ? (
                      <>
                        <Equalizer className="h-6 w-10" />
                        <div>
                          <p className="text-sm font-medium text-zinc-300">{t("listening")}</p>
                          <p className="mt-0.5 text-xs text-zinc-500">{t("listeningCaption")}</p>
                        </div>
                      </>
                    ) : (
                      <>
                        <span className="flex h-11 w-11 items-center justify-center rounded-xl border border-white/5 bg-ink-800 text-zinc-600">
                          <Icon name="mic" className="h-5 w-5" />
                        </span>
                        <p className="max-w-[26ch] text-sm leading-relaxed text-zinc-500">
                          {t("emptyTranscript")}
                        </p>
                      </>
                    )}
                  </div>
                ) : (
                  transcript.map((line, idx) => (
                    <TranscriptBubble key={`tb-${idx}-${line.started_at_ms}`} line={line} />
                  ))
                )}
                <div ref={transcriptEndRef} />
              </div>
            </section>

            {/* ============ Hint ============ */}
            <section
              className={`rounded-2xl border p-6 shadow-card transition-colors duration-500 animate-fade-up ${
                panicking
                  ? "border-signal-amber/25 bg-ink-900"
                  : lastHint
                    ? "border-volt-400/30 bg-gradient-to-br from-volt-400/[0.07] to-ink-900"
                    : "border-white/5 bg-ink-900"
              }`}
            >
              <div className="mb-3 flex items-center gap-2.5">
                <span
                  className={`flex h-8 w-8 items-center justify-center rounded-lg transition-colors ${
                    panicking
                      ? "bg-signal-amber/15 text-signal-amber"
                      : "bg-volt-400/10 text-volt-400"
                  }`}
                >
                  <Icon name={panicking ? "mute" : "bolt"} className="h-4 w-4" />
                </span>
                <h3 className="font-mono text-[11px] font-medium uppercase tracking-[0.18em] text-zinc-400">
                  {t("hint")}
                </h3>
              </div>
              {panicking ? (
                <p className="text-sm text-signal-amber">{t("panicBanner")}</p>
              ) : lastHint ? (
                <p key={lastHint} className="text-base font-medium leading-relaxed text-white animate-fade-up">
                  {lastHint}
                </p>
              ) : (
                <p className="text-sm text-zinc-500">{t("noHint")}</p>
              )}
            </section>
          </div>

          {/* ============ Right rail ============ */}
          <div className="space-y-6">
            {selectedSession && (
              <PostCallPanel session={selectedSession} onOpenSettings={handleOpenSettings} />
            )}

            {!running && (
              <SessionList
                sessions={sessions}
                selectedSession={selectedSession}
                onSelect={setSelectedSession}
              />
            )}
          </div>
        </div>
      </main>

      {showReindex && <ReindexPanel onClose={() => setShowReindex(false)} />}
      {showSettings && (
        <SettingsDialog
          onClose={() => setShowSettings(false)}
          initialTab={settingsTab}
        />
      )}
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
