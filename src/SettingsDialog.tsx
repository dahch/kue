import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t, useLanguage, saveLanguage, type Language } from "./i18n";
import { PROVIDERS } from "./constants";
import { usePersistedSetting } from "./hooks";
import ApiKeyInput from "./ApiKeyInput";
import { Icon } from "./Icon";
import { StyledSelect } from "./ui";

type Tab = "api-keys" | "llm-defaults" | "general";

const TABS: Tab[] = ["api-keys", "llm-defaults", "general"];

export default function SettingsDialog({
  onClose,
  initialTab = "api-keys",
}: {
  onClose: () => void;
  initialTab?: Tab;
}) {
  const [tab, setTab] = useState<Tab>(initialTab);

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
        aria-labelledby="settings-title"
        className="flex max-h-[85vh] w-full max-w-xl flex-col rounded-2xl border border-white/10 bg-ink-900 p-6 shadow-pop animate-scale-in"
      >
        <div className="mb-5 flex items-center justify-between">
          <h2 id="settings-title" className="flex items-center gap-2.5 text-lg font-semibold text-white">
            <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-volt-400/10 text-volt-400">
              <Icon name="sliders" className="h-4 w-4" />
            </span>
            {t("settings")}
          </h2>
          <button
            onClick={onClose}
            aria-label={t("close")}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-zinc-500 transition-colors hover:bg-white/5 hover:text-white"
          >
            <Icon name="x" className="h-4 w-4" />
          </button>
        </div>

        <div role="tablist" aria-label={t("settings")} className="mb-6 flex gap-1 rounded-lg border border-white/5 bg-ink-850 p-1">
          {TABS.map((tabId) => (
            <button
              key={tabId}
              role="tab"
              aria-selected={tab === tabId}
              onClick={() => setTab(tabId)}
              className={`flex-1 rounded-md px-3 py-2 font-mono text-[11px] font-medium uppercase tracking-wider transition-colors ${
                tab === tabId ? "bg-ink-700 text-white shadow-sm" : "text-zinc-500 hover:text-zinc-300"
              }`}
            >
              {tabId === "api-keys" && t("tabApiKeys")}
              {tabId === "llm-defaults" && t("tabLlmDefaults")}
              {tabId === "general" && t("tabGeneral")}
            </button>
          ))}
        </div>

        <div className="flex-1 overflow-y-auto pr-1">
          {tab === "api-keys" && <ApiKeysPanel />}
          {tab === "llm-defaults" && <LlmDefaultsPanel />}
          {tab === "general" && <GeneralPanel />}
        </div>
      </div>
    </div>
  );
}

/* ---------- API Keys tab ---------- */

function ApiKeysPanel() {
  const [savedKeys, setSavedKeys] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);

  const refreshKeys = useCallback(async () => {
    try {
      const keys: string[] = await invoke("list_saved_keys", {
        providers: PROVIDERS.map((p) => p.value),
      });
      setSavedKeys(keys);
    } catch {
      // keep current
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshKeys();
  }, [refreshKeys]);

  if (loading) return null;

  return (
    <div className="space-y-3">
      {PROVIDERS.map((p) => {
        const hasKey = savedKeys.includes(p.value);
        const isOllama = p.value === "ollama";

        return (
          <div key={p.value} className="rounded-xl border border-white/5 bg-ink-850 p-4">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-sm font-medium text-white">{p.label}</span>
              <span
                className={`flex items-center gap-1 font-mono text-[10px] font-medium uppercase tracking-wider ${
                  isOllama ? "text-zinc-500" : hasKey ? "text-volt-400" : "text-signal-amber"
                }`}
              >
                {isOllama ? (
                  <>{t("noKeyRequired")}</>
                ) : hasKey ? (
                  <><Icon name="check" className="h-3 w-3" />{t("apiKeySaved")}</>
                ) : (
                  <>{t("keyNotSaved")}</>
                )}
              </span>
            </div>
            {!isOllama && (
              <ApiKeyInput
                provider={p.value}
                hasSavedKey={hasKey}
                onKeySaved={refreshKeys}
                onKeyDeleted={refreshKeys}
                showDelete
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ---------- LLM Defaults tab ---------- */

const FEATURES = [
  { key: "hint", labelKey: "hints" as const },
  { key: "analyze", labelKey: "analyze" as const },
  { key: "plan", labelKey: "aiInterview" as const },
];

function LlmDefaultsPanel() {
  const [globalProvider, setGlobalProvider] = usePersistedSetting("default_provider", "openai");
  const [globalModel, setGlobalModel] = usePersistedSetting("default_model");

  return (
    <div className="space-y-4">
      <div className="rounded-xl border border-white/5 bg-ink-850 p-4">
        <h3 className="mb-3 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-volt-400">
          {t("globalDefault")}
        </h3>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
              {t("provider")}
            </label>
            <StyledSelect value={globalProvider} onChange={setGlobalProvider} options={PROVIDERS} ariaLabel={t("provider")} />
          </div>
          <div>
            <label className="mb-1.5 block font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-500">
              {t("model")} <span className="normal-case text-zinc-600">({t("modelOptional")})</span>
            </label>
            <input
              type="text"
              value={globalModel}
              onChange={(e) => setGlobalModel(e.target.value)}
              placeholder={globalProvider === "ollama" ? "llama3.1" : "default"}
              className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
            />
          </div>
        </div>
      </div>

      {FEATURES.map((feat) => (
        <FeatureRow
          key={feat.key}
          featureKey={feat.key}
          label={t(feat.labelKey)}
          globalProvider={globalProvider}
          globalModel={globalModel}
        />
      ))}
    </div>
  );
}

function FeatureRow({
  featureKey,
  label,
  globalProvider,
  globalModel,
}: {
  featureKey: string;
  label: string;
  globalProvider: string;
  globalModel: string;
}) {
  const [provider, setProvider] = usePersistedSetting(`${featureKey}_provider`);
  const [model, setModel] = usePersistedSetting(`${featureKey}_model`);
  const [hasKey, setHasKey] = useState(false);

  const useGlobal = !provider;
  const resolvedProvider = provider || globalProvider;
  const resolvedModel = model || globalModel;

  useEffect(() => {
    invoke<boolean>("has_key", { provider: resolvedProvider })
      .then(setHasKey)
      .catch(() => setHasKey(false));
  }, [resolvedProvider]);

  const handleUseGlobal = useCallback(() => {
    setProvider("");
    setModel("");
  }, [setProvider, setModel]);

  const handleUseCustom = useCallback(() => {
    setProvider(resolvedProvider);
    setModel(resolvedModel);
  }, [resolvedProvider, resolvedModel, setProvider, setModel]);

  return (
    <div className="rounded-xl border border-white/5 bg-ink-850 p-4">
      <div className="mb-2.5 flex items-center justify-between">
        <h3 className="text-sm font-medium text-white">{label}</h3>
        <span
          className={`flex items-center gap-1 font-mono text-[10px] font-medium uppercase tracking-wider ${
            hasKey ? "text-volt-400" : "text-signal-amber"
          }`}
        >
          {hasKey ? (
            <><Icon name="check" className="h-3 w-3" />{t("apiKeySaved")}</>
          ) : (
            t("keyNotSaved")
          )}
        </span>
      </div>

      <div className="mb-3 flex gap-4">
        <label className="flex cursor-pointer items-center gap-2">
          <input
            type="radio"
            name={`${featureKey}-mode`}
            checked={useGlobal}
            onChange={handleUseGlobal}
            className="h-4 w-4 accent-volt-400"
          />
          <span className="text-xs text-zinc-400">{t("useGlobal")}</span>
        </label>
        <label className="flex cursor-pointer items-center gap-2">
          <input
            type="radio"
            name={`${featureKey}-mode`}
            checked={!useGlobal}
            onChange={handleUseCustom}
            className="h-4 w-4 accent-volt-400"
          />
          <span className="text-xs text-zinc-400">{t("custom")}</span>
        </label>
      </div>

      {!useGlobal && (
        <div className="grid grid-cols-2 gap-3">
          <StyledSelect
            value={provider || globalProvider}
            onChange={setProvider}
            options={PROVIDERS}
            ariaLabel={t("provider")}
          />
          <input
            type="text"
            value={model || globalModel}
            onChange={(e) => setModel(e.target.value)}
            placeholder={(provider || globalProvider) === "ollama" ? "llama3.1" : "default"}
            className="w-full rounded-lg border border-white/10 bg-ink-750 px-3 py-2 text-sm text-white placeholder-zinc-600 transition-colors focus:border-volt-400/50"
          />
        </div>
      )}
    </div>
  );
}

/* ---------- General tab ---------- */

function GeneralPanel() {
  const lang = useLanguage();

  const handleLanguageChange = useCallback((l: Language) => {
    saveLanguage(l).catch(() => {});
  }, []);

  return (
    <div className="space-y-4">
      <div className="rounded-xl border border-white/5 bg-ink-850 p-4">
        <h3 className="mb-3 font-mono text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-400">
          {t("language")}
        </h3>
        <div role="group" aria-label={t("selectLanguage")} className="flex gap-1 rounded-lg border border-white/10 bg-ink-800 p-0.5">
          {(["es", "en"] as const).map((l) => (
            <button
              key={l}
              type="button"
              aria-pressed={lang === l}
              onClick={() => handleLanguageChange(l)}
              className={`rounded-md px-3 py-1.5 font-mono text-[11px] font-medium uppercase tracking-wider transition-colors ${
                lang === l ? "bg-volt-400 text-ink-950" : "text-zinc-400 hover:text-zinc-200"
              }`}
            >
              {l}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
