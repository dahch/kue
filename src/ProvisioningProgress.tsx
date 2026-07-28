import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { t, useLanguage } from "./i18n";
import { Icon } from "./Icon";
import { Spinner } from "./ui";

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
    <div className="flex min-h-screen flex-col items-center justify-center p-8 text-white">
      <div className="mb-8 flex flex-col items-center gap-4 animate-fade-up">
        <div className="relative">
          <span aria-hidden="true" className="absolute inset-0 rounded-3xl bg-volt-400/20 blur-xl animate-pulse" />
          <img
            src="/kue-icon.svg"
            alt=""
            className="relative h-16 w-16 rounded-3xl shadow-card ring-1 ring-white/10"
          />
        </div>
      </div>

      <div className="w-full max-w-md rounded-2xl border border-white/5 bg-ink-900 p-8 shadow-card animate-fade-up" style={{ animationDelay: "60ms" }}>
        <h1 className="mb-1.5 text-2xl font-bold tracking-tight">{t("preparingKue")}</h1>
        <p className="mb-7 text-sm leading-relaxed text-zinc-400">
          {t("provisioningSubtitle")}
        </p>

        {state === "downloading" && progress && (
          <div className="space-y-3 animate-fade-in">
            <div className="flex items-baseline justify-between text-sm">
              <span className="text-zinc-300">{stageLabel}</span>
              <span className="font-mono text-lg font-semibold tabular-nums text-volt-400">{percent}%</span>
            </div>
            <div
              role="progressbar"
              aria-valuenow={percent}
              aria-valuemin={0}
              aria-valuemax={100}
              className="h-2.5 w-full overflow-hidden rounded-full bg-ink-700"
            >
              <div
                className="relative h-full overflow-hidden rounded-full bg-volt-400 transition-all duration-300"
                style={{ width: `${percent}%` }}
              >
                <span aria-hidden="true" className="absolute inset-y-0 w-1/3 animate-shimmer bg-white/30 blur-[2px]" />
              </div>
            </div>
            <p className="font-mono text-[11px] uppercase tracking-wider text-zinc-500">
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
            <Spinner />
            {t("startingDownload")}
          </div>
        )}

        {state === "error" && (
          <div className="space-y-4 animate-fade-in">
            <div className="rounded-xl border border-signal-red/30 bg-signal-red/[0.07] p-4" role="alert">
              <p className="flex items-start gap-2 text-sm text-signal-red">
                <Icon name="alert" className="mt-0.5 h-4 w-4" />
                {error || t("downloadError")}
              </p>
            </div>
            <button
              className="flex w-full items-center justify-center gap-2 rounded-xl bg-volt-400 py-3 font-semibold text-ink-950 transition-all hover:bg-volt-300 active:scale-[0.99]"
              onClick={handleRetry}
            >
              <Icon name="refresh" className="h-4 w-4" />
              {t("retry")}
            </button>
          </div>
        )}

        {state === "retrying" && (
          <div className="flex items-center gap-3 text-sm text-zinc-400">
            <Spinner />
            {t("retrying")}
          </div>
        )}
      </div>
    </div>
  );
}

export default ProvisioningProgress;
