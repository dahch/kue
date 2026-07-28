import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { listen } from "@tauri-apps/api/event";
import { Icon } from "./Icon";

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
      aria-live="polite"
      className="fixed inset-0 flex items-start justify-center pt-7 transition-all duration-500"
      style={{
        opacity: visible ? 1 : 0,
        transform: visible ? "translateY(0)" : "translateY(-8px)",
      }}
    >
      <div
        className={`flex max-w-full items-center gap-3 rounded-2xl px-6 py-4 shadow-pop backdrop-blur-xl ring-1 ${
          panicking
            ? "bg-signal-amber/85 text-ink-950 ring-signal-amber/50"
            : "bg-ink-950/75 text-white ring-white/10"
        }`}
      >
        {panicking ? (
          <Icon name="mute" className="h-5 w-5" />
        ) : (
          <span aria-hidden="true" className="h-2 w-2 shrink-0 rounded-full bg-volt-400 shadow-[0_0_12px_2px_rgba(201,242,75,0.7)]" />
        )}
        <p className="text-lg font-medium leading-snug">
          {panicking ? "" : hint?.text ?? ""}
        </p>
      </div>
    </div>
  );
}

export default Overlay;
