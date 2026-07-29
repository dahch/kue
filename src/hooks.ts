import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export function usePersistedSetting(key: string, defaultValue = "") {
  const [value, setValue] = useState(defaultValue);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const v = await invoke<string | null>("get_setting", { key });
        if (!cancelled && v) setValue(v);
      } catch {
        // keep default
      } finally {
        if (!cancelled) setLoaded(true);
      }
    })();
    return () => { cancelled = true; };
  }, [key]);

  useEffect(() => {
    if (!loaded) return;
    const v = value.trim();
    if (v) {
      invoke("set_setting", { key, value: v }).catch(() =>
        console.warn(`Failed to persist setting "${key}"`)
      );
    }
  }, [value, loaded, key]);

  return [value, setValue] as const;
}

export function useLLMSettings(featureKey: string, providerHardDefault = "openai") {
  const [featureProvider, setFeatureProvider] = usePersistedSetting(`${featureKey}_provider`);
  const [featureModel, setFeatureModel] = usePersistedSetting(`${featureKey}_model`);
  const [globalProvider] = usePersistedSetting("default_provider", providerHardDefault);
  const [globalModel] = usePersistedSetting("default_model");

  const provider = featureProvider || globalProvider;
  const model = featureModel || globalModel;

  return { provider, setProvider: setFeatureProvider, model, setModel: setFeatureModel };
}

export function useTauriEvent<T>(
  event: string,
  handler: (payload: T) => void,
  deps: React.DependencyList = [],
) {
  useEffect(() => {
    const unlisten = listen<T>(event, (e) => handler(e.payload));
    return () => { unlisten.then((fn) => fn()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [event, ...deps]);
}
