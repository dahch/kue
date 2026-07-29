import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n";
import { Icon } from "./Icon";

export default function ApiKeyInput({
  provider,
  hasSavedKey,
  onKeySaved,
  onKeyDeleted,
  showDelete,
}: {
  provider: string;
  hasSavedKey: boolean;
  onKeySaved: () => void;
  onKeyDeleted?: () => void;
  showDelete?: boolean;
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

  const handleDelete = useCallback(async () => {
    if (!window.confirm(`${t("deleteKeyConfirm")} (${provider})`)) return;
    try {
      await invoke("delete_key", { provider });
      onKeyDeleted?.();
    } catch (e) {
      setError(`${t("error")}: ${e}`);
    }
  }, [provider, onKeyDeleted]);

  return (
    <div className="mb-4">
      <div className="mb-1.5 flex items-center justify-between">
        <label htmlFor={`api-key-${provider}`} className="block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
          {t("apiKey")} ({provider})
        </label>
        <div className="flex items-center gap-2">
          {hasSavedKey && (
            <>
              <span className="flex items-center gap-1 font-mono text-[10px] font-medium uppercase tracking-wider text-volt-400">
                <Icon name="check" className="h-3 w-3" />
                {t("apiKeySaved")}
              </span>
              {showDelete && (
                <button
                  type="button"
                  onClick={handleDelete}
                  className="flex items-center gap-1 font-mono text-[10px] font-medium uppercase tracking-wider text-signal-red/70 transition-colors hover:text-signal-red"
                >
                  <Icon name="x" className="h-3 w-3" />
                  {t("delete")}
                </button>
              )}
            </>
          )}
        </div>
      </div>
      <div className="flex gap-2">
        <div className="relative flex-1">
          <input
            id={`api-key-${provider}`}
            type={showKey ? "text" : "password"}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={t("apiKey")}
            autoComplete="off"
            className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 pr-10 font-mono text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
          />
          <button
            type="button"
            aria-label={showKey ? t("hide") : t("show")}
            className="absolute right-1.5 top-1/2 flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded-md text-zinc-500 transition-colors hover:bg-white/5 hover:text-zinc-200"
            onClick={() => setShowKey(!showKey)}
          >
            <Icon name={showKey ? "eye-off" : "eye"} className="h-4 w-4" />
          </button>
        </div>
        <button
          type="button"
          className="rounded-lg bg-ink-700 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-ink-750 hover:text-volt-300 disabled:cursor-not-allowed disabled:opacity-40"
          onClick={handleSave}
          disabled={!apiKey.trim()}
        >
          {t("save")}
        </button>
      </div>
      {error && (
        <p className="mt-1.5 flex items-center gap-1.5 text-xs text-signal-red" role="alert">
          <Icon name="alert" className="h-3.5 w-3.5" />
          {error}
        </p>
      )}
    </div>
  );
}
