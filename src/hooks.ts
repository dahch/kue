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
