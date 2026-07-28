import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, useLanguage } from "./i18n";
import { PROVIDERS } from "./constants";
import ApiKeyInput from "./ApiKeyInput";
import type { IndexSummary } from "./types";
import { formatIndexResult, isValidFolderPath, sanitizeError } from "./validation";
import { Icon, type IconName } from "./Icon";
import { Spinner, StyledSelect } from "./ui";

type OnboardingStep =
  | "checking"
  | "screen_permission"
  | "embedding_model"
  | "api_key"
  | "folder_selection"
  | "done";

const STEP_FLOW: Exclude<OnboardingStep, "checking" | "done">[] = [
  "screen_permission",
  "embedding_model",
  "api_key",
  "folder_selection",
];

const STEP_META: Record<
  (typeof STEP_FLOW)[number],
  { icon: IconName; labelKey: "stepLabelPermission" | "stepLabelModel" | "stepLabelKey" | "stepLabelIndex" }
> = {
  screen_permission: { icon: "shield", labelKey: "stepLabelPermission" },
  embedding_model: { icon: "cpu", labelKey: "stepLabelModel" },
  api_key: { icon: "key", labelKey: "stepLabelKey" },
  folder_selection: { icon: "folder", labelKey: "stepLabelIndex" },
};

function Stepper({ step }: { step: OnboardingStep }) {
  const currentIndex = STEP_FLOW.indexOf(step as (typeof STEP_FLOW)[number]);
  if (currentIndex < 0) return null;

  return (
    <div className="mb-7">
      <ol aria-label={t("onboardingTitle")} className="flex items-center">
        {STEP_FLOW.map((s, i) => {
          const meta = STEP_META[s];
          const done = currentIndex > i;
          const current = currentIndex === i;
          return (
            <li key={s} aria-current={current ? "step" : undefined} className="flex flex-1 items-center last:flex-none">
              <div className="flex flex-col items-center gap-1.5">
                <span
                  className={`flex h-8 w-8 items-center justify-center rounded-full border transition-all ${
                    done
                      ? "border-volt-400 bg-volt-400 text-ink-950"
                      : current
                        ? "border-volt-400/60 bg-volt-400/10 text-volt-400 animate-pulse-volt"
                        : "border-white/10 bg-ink-800 text-zinc-600"
                  }`}
                >
                  {done ? (
                    <Icon name="check" className="h-3.5 w-3.5" strokeWidth={2.5} />
                  ) : (
                    <Icon name={meta.icon} className="h-3.5 w-3.5" />
                  )}
                </span>
                <span
                  className={`font-mono text-[9px] uppercase tracking-wider ${
                    current ? "text-volt-400" : done ? "text-zinc-400" : "text-zinc-600"
                  }`}
                >
                  {t(meta.labelKey)}
                </span>
              </div>
              {i < STEP_FLOW.length - 1 && (
                <span
                  aria-hidden="true"
                  className={`mx-2 mb-5 h-px flex-1 transition-colors ${done ? "bg-volt-400/60" : "bg-white/10"}`}
                />
              )}
            </li>
          );
        })}
      </ol>
      {currentIndex >= 0 && (
        <p className="mt-3 text-center font-mono text-[10px] uppercase tracking-[0.2em] text-zinc-500">
          {t("onboardingStepOf", { current: currentIndex + 1, total: STEP_FLOW.length })}
        </p>
      )}
    </div>
  );
}

function StepHeading({ icon, children }: { icon: IconName; children: React.ReactNode }) {
  return (
    <h2 className="mb-3 flex items-center gap-2.5 text-base font-semibold text-white">
      <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-volt-400/10 text-volt-400">
        <Icon name={icon} className="h-4 w-4" />
      </span>
      {children}
    </h2>
  );
}

function ErrorBox({ message }: { message: string }) {
  return (
    <div className="rounded-xl border border-signal-red/30 bg-signal-red/[0.07] p-3.5" role="alert">
      <p className="flex items-start gap-2 text-sm text-signal-red">
        <Icon name="alert" className="mt-0.5 h-4 w-4" />
        {message}
      </p>
    </div>
  );
}

function Onboarding({ onComplete }: { onComplete: () => void }) {
  const language = useLanguage();
  const [step, setStep] = useState<OnboardingStep>("checking");
  const [screenGranted, setScreenGranted] = useState(false);
  const [modelLoaded, setModelLoaded] = useState(false);
  const [apiKeyProvider, setApiKeyProvider] = useState("openai");
  const [apiKeyModel, setApiKeyModel] = useState("");
  const [hasSavedApiKey, setHasSavedApiKey] = useState(false);
  const [folderPath, setFolderPath] = useState("");
  const [indexing, setIndexing] = useState(false);
  const [indexResult, setIndexResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pathError = useMemo(() => isValidFolderPath(folderPath), [folderPath, language]);

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const hasPermission: boolean = await invoke(
          "check_screen_recording_permission",
        );
        if (cancelled) return;
        setScreenGranted(hasPermission);

        const modelReady: boolean = await invoke(
          "is_embedding_model_loaded",
        );
        if (cancelled) return;
        setModelLoaded(modelReady);

        if (cancelled) return;
        if (!hasPermission) {
          setStep("screen_permission");
        } else if (!modelReady) {
          setStep("embedding_model");
        } else {
          setStep("api_key");
        }
      } catch (e) {
        if (!cancelled) {
          setError(sanitizeError(e));
          setStep("screen_permission");
        }
      }
    };
    check();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    if (step !== "embedding_model" || modelLoaded) return;
    const interval = setInterval(async () => {
      try {
        const ready: boolean = await invoke("is_embedding_model_loaded");
        if (ready) {
          setModelLoaded(true);
          setStep("api_key");
        }
      } catch {
        // keep polling
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [step, modelLoaded]);

  const handleGrantPermission = useCallback(async () => {
    setError(null);
    try {
      const granted: boolean = await invoke(
        "check_screen_recording_permission",
      );
      if (granted) {
        setScreenGranted(true);
        const modelReady: boolean = await invoke("is_embedding_model_loaded");
        setModelLoaded(modelReady);
        setStep(modelReady ? "api_key" : "embedding_model");
      } else {
        setError(
          `${t("permissionDenied")} ${t("screenPermissionInstructions")}`,
        );
      }
    } catch (e) {
      setError(sanitizeError(e));
    }
  }, []);

  const handleIndexFolder = useCallback(async () => {
    const trimmed = folderPath.trim();
    const validationError = isValidFolderPath(trimmed);
    if (validationError) {
      setError(validationError);
      return;
    }
    setIndexing(true);
    setError(null);
    try {
      const summary: IndexSummary = await invoke("index_folder_cmd", {
        path: trimmed,
      });
      setIndexResult(formatIndexResult(summary));
    } catch (e) {
      console.error("index_folder_cmd failed:", e);
      setError(sanitizeError(e));
    } finally {
      setIndexing(false);
    }
  }, [folderPath]);

  const handleSkipApiKey = useCallback(async () => {
    try {
      await invoke("set_setting", { key: "hint_provider", value: apiKeyProvider });
      if (apiKeyModel.trim()) {
        await invoke("set_setting", { key: "hint_model", value: apiKeyModel.trim() });
      }
    } catch { /* ignore */ }
    setStep("folder_selection");
  }, [apiKeyProvider, apiKeyModel]);

  const handleSkipFolder = useCallback(async () => {
    setStep("done");
    await invoke("mark_onboarding_done");
    onComplete();
  }, [onComplete]);

  const handleComplete = useCallback(async () => {
    await invoke("mark_onboarding_done");
    onComplete();
  }, [onComplete]);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center p-8 text-white">
      <div className="mb-6 flex items-center gap-3 animate-fade-up">
        <img src="/kue-icon.svg" alt="" className="h-11 w-11 rounded-2xl shadow-card ring-1 ring-white/10" />
        <div className="leading-tight">
          <p className="text-lg font-bold tracking-tight">{t("appTitle")}</p>
          <p className="font-mono text-[10px] uppercase tracking-[0.22em] text-zinc-500">{t("tagline")}</p>
        </div>
      </div>

      <div className="w-full max-w-lg rounded-2xl border border-white/5 bg-ink-900 p-8 shadow-card animate-fade-up" style={{ animationDelay: "60ms" }}>
        <h1 className="mb-1.5 text-2xl font-bold tracking-tight">{t("onboardingTitle")}</h1>
        <p className="mb-7 text-sm leading-relaxed text-zinc-400">
          {t("onboardingSubtitle")}
        </p>

        <Stepper step={step} />

        <div key={step} className="animate-fade-up">
          {step === "screen_permission" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-white/5 bg-ink-850 p-5">
                <StepHeading icon="shield">{t("stepScreenPermission")}</StepHeading>
                <p className="mb-3 text-sm leading-relaxed text-zinc-300">
                  {t("screenPermissionDescription")}
                </p>
                <p className="mb-4 rounded-lg border border-white/5 bg-ink-800 p-3 text-xs leading-relaxed text-zinc-500">
                  {t("screenPermissionInstructions")}
                </p>
                {screenGranted ? (
                  <p className="flex items-center gap-2 text-sm font-medium text-volt-400">
                    <Icon name="check" className="h-4 w-4" />
                    {t("permissionGranted")}
                  </p>
                ) : (
                  <button
                    className="flex w-full items-center justify-center gap-2 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99]"
                    onClick={handleGrantPermission}
                  >
                    <Icon name="shield" className="h-4 w-4" />
                    {t("grantPermission")}
                  </button>
                )}
              </div>
              {error && <ErrorBox message={error} />}
            </div>
          )}

          {step === "embedding_model" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-white/5 bg-ink-850 p-5">
                <StepHeading icon="cpu">{t("stepEmbeddingModel")}</StepHeading>
                <div className="flex items-center gap-3 text-sm text-zinc-300">
                  <Spinner />
                  {t("loadingModel")}
                </div>
                <div aria-hidden="true" className="mt-4 h-1 w-full overflow-hidden rounded-full bg-ink-700">
                  <div className="h-full w-1/3 animate-shimmer rounded-full bg-volt-400/70" />
                </div>
                <p className="mt-3 text-xs leading-relaxed text-zinc-500">
                  {t("embeddingModelHint")}
                </p>
              </div>
            </div>
          )}

          {step === "api_key" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-white/5 bg-ink-850 p-5">
                <StepHeading icon="key">{t("stepApiKey")}</StepHeading>
                <p className="mb-4 text-sm leading-relaxed text-zinc-300">
                  {t("apiKeyDescription")}
                </p>
                <div className="mb-3 grid grid-cols-2 gap-3">
                  <div>
                    <label className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
                      {t("provider")}
                    </label>
                    <StyledSelect
                      value={apiKeyProvider}
                      onChange={setApiKeyProvider}
                      options={PROVIDERS}
                      ariaLabel={t("provider")}
                    />
                  </div>
                  <div>
                    <label htmlFor="onboarding-model" className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
                      {t("model")} <span className="normal-case text-zinc-600">({t("modelOptional")})</span>
                    </label>
                    <input
                      id="onboarding-model"
                      type="text"
                      value={apiKeyModel}
                      onChange={(e) => setApiKeyModel(e.target.value)}
                      placeholder={apiKeyProvider === "ollama" ? "llama3.1" : t("modelOptional")}
                      className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
                    />
                  </div>
                </div>
                <ApiKeyInput
                  provider={apiKeyProvider}
                  hasSavedKey={hasSavedApiKey}
                  onKeySaved={() => setHasSavedApiKey(true)}
                />
              </div>
              <div className="flex gap-3">
                <button
                  className="flex-1 rounded-xl border border-white/10 py-3 font-medium text-zinc-300 transition-colors hover:border-white/25 hover:text-white"
                  onClick={handleSkipApiKey}
                >
                  {t("configureLater")}
                </button>
                {hasSavedApiKey && (
                  <button
                    className="flex-1 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99]"
                    onClick={handleSkipApiKey}
                  >
                    {t("start")}
                  </button>
                )}
              </div>
            </div>
          )}

          {step === "folder_selection" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-white/5 bg-ink-850 p-5">
                <StepHeading icon="folder">{t("stepIndexProjects")}</StepHeading>
                <p className="mb-4 text-sm leading-relaxed text-zinc-300">
                  {t("indexProjectsDescription")}
                </p>
                <label htmlFor="onboarding-folder" className="visually-hidden">
                  {t("folderPathLabel")}
                </label>
                <input
                  id="onboarding-folder"
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
                {folderPath.trim() && (
                  <button
                    className="mt-3.5 flex w-full items-center justify-center gap-2 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99] disabled:cursor-not-allowed disabled:bg-ink-700 disabled:text-zinc-500"
                    onClick={handleIndexFolder}
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
                )}
              </div>
              {indexResult && (
                <p className="flex items-center gap-2 text-sm font-medium text-volt-400">
                  <Icon name="check" className="h-4 w-4" />
                  {indexResult}
                </p>
              )}
              {error && <ErrorBox message={error} />}
              <div className="flex gap-3">
                <button
                  className="flex-1 rounded-xl border border-white/10 py-3 font-medium text-zinc-300 transition-colors hover:border-white/25 hover:text-white"
                  onClick={handleSkipFolder}
                >
                  {t("skipIndex")}
                </button>
                {indexResult && (
                  <button
                    className="flex-1 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99]"
                    onClick={handleComplete}
                  >
                    {t("start")}
                  </button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export default Onboarding;
