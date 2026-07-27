import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, useLanguage } from "./i18n";
import type { IndexSummary } from "./types";
import { formatIndexResult, isValidFolderPath, sanitizeError } from "./validation";

type OnboardingStep =
  | "checking"
  | "screen_permission"
  | "embedding_model"
  | "folder_selection"
  | "done";

function Onboarding({ onComplete }: { onComplete: () => void }) {
  const language = useLanguage();
  const [step, setStep] = useState<OnboardingStep>("checking");
  const [screenGranted, setScreenGranted] = useState(false);
  const [modelLoaded, setModelLoaded] = useState(false);
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
          setStep("folder_selection");
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
          setStep("folder_selection");
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
        setStep(modelReady ? "folder_selection" : "embedding_model");
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
    <div className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 p-8 text-white">
      <div className="w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-8">
        <h1 className="mb-2 text-2xl font-bold">{t("onboardingTitle")}</h1>
        <p className="mb-6 text-sm text-zinc-400">
          {t("onboardingSubtitle")}
        </p>

        {step === "screen_permission" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                {t("stepScreenPermission")}
              </h2>
              <p className="mb-3 text-sm text-zinc-300">
                {t("screenPermissionDescription")}
              </p>
              <p className="mb-3 text-xs text-zinc-500">
                {t("screenPermissionInstructions")}
              </p>
              {screenGranted ? (
                <p className="text-sm text-emerald-400">
                  {t("permissionGranted")}
                </p>
              ) : (
                <button
                  className="w-full rounded-lg bg-blue-600 py-3 font-medium transition-colors hover:bg-blue-500"
                  onClick={handleGrantPermission}
                >
                  {t("grantPermission")}
                </button>
              )}
            </div>
            {error && (
              <div className="rounded-lg border border-red-800 bg-red-900/30 p-3">
                <p className="text-sm text-red-400">{error}</p>
              </div>
            )}
          </div>
        )}

        {step === "embedding_model" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                {t("stepEmbeddingModel")}
              </h2>
              <div className="flex items-center gap-3 text-sm text-zinc-300">
                <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-zinc-500 border-t-zinc-300" />
                {t("loadingModel")}
              </div>
              <p className="mt-3 text-xs text-zinc-500">
                {t("embeddingModelHint")}
              </p>
            </div>
          </div>
        )}

        {step === "folder_selection" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                {t("stepIndexProjects")}
              </h2>
              <p className="mb-3 text-sm text-zinc-300">
                {t("indexProjectsDescription")}
              </p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={folderPath}
                  onChange={(e) => setFolderPath(e.target.value)}
                  placeholder={t("folderPathPlaceholder")}
                  className="flex-1 rounded-lg border border-zinc-600 bg-zinc-800 px-3 py-2 text-sm text-white placeholder-zinc-500"
                />
              </div>
              {pathError && folderPath.trim() && (
                <p className="mt-1 text-xs text-amber-400">{pathError}</p>
              )}
              {folderPath.trim() && (
                <button
                  className="mt-3 w-full rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500 disabled:opacity-50"
                  onClick={handleIndexFolder}
                  disabled={indexing || !!pathError}
                >
                  {indexing
                    ? t("indexing")
                    : t("indexFolder")}
                </button>
              )}
            </div>
            {indexResult && (
              <p className="text-sm text-emerald-400">{indexResult}</p>
            )}
            {error && (
              <div className="rounded-lg border border-red-800 bg-red-900/30 p-3">
                <p className="text-sm text-red-400">{error}</p>
              </div>
            )}
            <div className="flex gap-3">
              <button
                className="flex-1 rounded-lg bg-zinc-700 py-3 font-medium transition-colors hover:bg-zinc-600"
                onClick={handleSkipFolder}
              >
                {t("skipIndex")}
              </button>
              {indexResult && (
                <button
                  className="flex-1 rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
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
  );
}

export default Onboarding;
