import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, useLanguage } from "./i18n";

interface DownloadProgress {
  stage: string;
  file_index: number;
  file_count: number;
  downloaded_bytes: number;
  total_bytes: number;
}

type ProvisionState = "checking" | "downloading" | "error" | "retrying";

function ProvisioningProgress({ onProvisioned }: { onProvisioned: () => void }) {
  useLanguage();

  const [state, setState] = useState<ProvisionState>("checking");
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const unlistenProgress = listen<DownloadProgress>("moonshine-download-progress", (event) => {
      if (cancelled) return;
      setProgress(event.payload);
      setState((prev) => prev === "retrying" ? "downloading" : prev);
    });

    const unlistenError = listen<string>("moonshine-provision-error", (event) => {
      if (cancelled) return;
      setError(event.payload);
      setState("error");
    });

    const unlistenDone = listen("moonshine-provisioned", () => {
      if (cancelled) return;
      onProvisioned();
    });

    invoke<boolean>("is_moonshine_provisioned")
      .then((provisioned) => {
        if (cancelled) return;
        if (provisioned) {
          onProvisioned();
        } else {
          setState("downloading");
        }
      })
      .catch(() => {
        if (!cancelled) setState("downloading");
      });

    return () => {
      cancelled = true;
      unlistenProgress.then((fn) => fn());
      unlistenError.then((fn) => fn());
      unlistenDone.then((fn) => fn());
    };
  }, [onProvisioned]);

  const handleRetry = useCallback(async () => {
    setState("retrying");
    setError(null);
    try {
      await invoke("retry_moonshine_download");
    } catch (e) {
      setError(`${e}`);
      setState("error");
    }
  }, []);

  const percent = progress && progress.total_bytes > 0
    ? Math.min(100, Math.round((progress.downloaded_bytes / progress.total_bytes) * 100))
    : 0;
  const stageLabel =
    progress?.stage === "model"
      ? t("downloadingModel")
      : t("downloadingLibs");

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 p-8 text-white">
      <div className="w-full max-w-md rounded-xl border border-zinc-700 bg-zinc-900 p-8">
        <h1 className="mb-2 text-2xl font-bold">{t("preparingKue")}</h1>
        <p className="mb-6 text-sm text-zinc-400">
          {t("provisioningSubtitle")}
        </p>

        {state === "downloading" && progress && (
          <div className="space-y-3">
            <div className="flex justify-between text-sm text-zinc-300">
              <span>{stageLabel}</span>
              <span>{percent}%</span>
            </div>
            <div className="h-3 w-full overflow-hidden rounded-full bg-zinc-700">
              <div
                className="h-full rounded-full bg-emerald-500 transition-all duration-300"
                style={{ width: `${percent}%` }}
              />
            </div>
            <p className="text-xs text-zinc-500">
              {t("fileXOfY", {
                fileIndex: progress.file_index + 1,
                fileCount: progress.file_count,
              })} &middot;{" "}
              {progress?.stage === "model" ? t("modelLabel") : t("dylibsLabel")}
            </p>
          </div>
        )}

        {state === "downloading" && !progress && (
          <div className="flex items-center gap-3 text-sm text-zinc-400">
            <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-zinc-500 border-t-zinc-300" />
            {t("startingDownload")}
          </div>
        )}

        {state === "error" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-red-800 bg-red-900/30 p-4">
              <p className="text-sm text-red-400">{error || t("downloadError")}</p>
            </div>
            <button
              className="w-full rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
              onClick={handleRetry}
            >
              {t("retry")}
            </button>
          </div>
        )}

        {state === "retrying" && (
          <div className="flex items-center gap-3 text-sm text-zinc-400">
            <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-zinc-500 border-t-zinc-300" />
            {t("retrying")}
          </div>
        )}
      </div>
    </div>
  );
}

export default ProvisioningProgress;
