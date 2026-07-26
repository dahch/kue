import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";

interface HintPayload {
  text: string;
  type: string;
  session_id: string;
}

interface SessionStartedPayload {
  mode: string;
  session_id: string;
}

interface PanicPayload {
  until_secs: number;
}

function Overlay() {
  const [hint, setHint] = useState<HintPayload | null>(null);
  const [visible, setVisible] = useState(false);
  const [panicking, setPanicking] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const panicTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hintRef = useRef(hint);
  hintRef.current = hint;

  useEffect(() => {
    const unlistenHint = listen<HintPayload>("new-hint", (event) => {
      if (timerRef.current) clearTimeout(timerRef.current);
      setHint(event.payload);
      setVisible(true);
      getCurrentWebviewWindow().show().catch((err) =>
        console.warn("Overlay: failed to show window on new-hint", err),
      );
      timerRef.current = setTimeout(() => setVisible(false), 3000);
    });

    const unlistenPanic = listen<PanicPayload>("panic-mode", (event) => {
      setPanicking(true);
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
      const ms = event.payload.until_secs * 1000;
      panicTimerRef.current = setTimeout(() => {
        setPanicking(false);
        if (!hintRef.current) setVisible(false);
      }, ms);
    });

    const unlistenStart = listen<SessionStartedPayload>("session-started", () => {
      getCurrentWebviewWindow().show().catch((err) =>
        console.warn("Overlay: failed to show window on session-started", err),
      );
    });

    const unlistenStop = listen("session-stopped", () => {
      getCurrentWebviewWindow().hide().catch((err) =>
        console.warn("Overlay: failed to hide window on session-stopped", err),
      );
      setVisible(false);
      setHint(null);
    });

    return () => {
      unlistenHint.then((fn) => fn());
      unlistenPanic.then((fn) => fn());
      unlistenStart.then((fn) => fn());
      unlistenStop.then((fn) => fn());
      if (timerRef.current) clearTimeout(timerRef.current);
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
    };
    // Tauri event listeners are global singletons; register once on mount
  }, []);

  // Top-center: transparent floating 400×100 window, unobtrusive above main content
  return (
    <div
      className="fixed inset-0 flex items-start justify-center pt-8 transition-opacity duration-500"
      style={{ opacity: visible ? 1 : 0 }}
    >
      <div
        className={`rounded-xl px-8 py-5 text-center text-xl font-medium leading-relaxed text-white shadow-2xl backdrop-blur-md ${
          panicking ? "bg-orange-700/70" : "bg-black/60"
        }`}
      >
        {panicking ? "🔇" : hint?.text ?? ""}
      </div>
    </div>
  );
}

export default Overlay;
