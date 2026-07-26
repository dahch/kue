import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Overlay from "./Overlay";

interface TranscriptEvent {
  session_id: string;
  text: string;
  speaker: string;
}

interface HintPayload {
  text: string;
  type: string;
  session_id: string;
}

interface PanicPayload {
  until_secs: number;
}

type SessionMode = "practice" | "shadow";

function MainApp() {
  const [mode, setMode] = useState<SessionMode>("practice");
  const [running, setRunning] = useState(false);
  const [lastTranscript, setLastTranscript] = useState("");
  const [lastHint, setLastHint] = useState("");
  const [panicking, setPanicking] = useState(false);
  const panicTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const unlistenTranscript = listen<TranscriptEvent>("new-transcript", (event) => {
      setLastTranscript(event.payload.text);
    });

    const unlistenHint = listen<HintPayload>("new-hint", (event) => {
      setLastHint(event.payload.text);
    });

    const unlistenPanic = listen<PanicPayload>("panic-mode", (event) => {
      setPanicking(true);
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
      const ms = event.payload.until_secs * 1000;
      panicTimerRef.current = setTimeout(() => setPanicking(false), ms);
    });

    return () => {
      unlistenTranscript.then((fn) => fn());
      unlistenHint.then((fn) => fn());
      unlistenPanic.then((fn) => fn());
      if (panicTimerRef.current) clearTimeout(panicTimerRef.current);
    };
  }, []);

  const handleStart = useCallback(async () => {
    try {
      await invoke("start_session", { mode });
      setRunning(true);
    } catch (e) {
      console.error("Failed to start session:", e);
    }
  }, [mode]);

  const handleStop = useCallback(async () => {
    try {
      await invoke("stop_session");
      setRunning(false);
    } catch (e) {
      console.error("Failed to stop session:", e);
    }
  }, []);

  const handlePanic = useCallback(async () => {
    try {
      await invoke("panic_mode");
    } catch (e) {
      console.error("Failed to activate panic mode:", e);
    }
  }, []);

  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-zinc-950 p-8 text-white">
      <h1 className="mb-8 text-4xl font-bold">Kue</h1>

      {/* Mode selector */}
      <div className="mb-6 w-full max-w-lg rounded-xl border border-zinc-700 bg-zinc-900 p-6">
        <h2 className="mb-3 text-lg font-semibold text-blue-400">Modo</h2>
        <div className="flex gap-3">
          <button
            className={`flex-1 rounded-lg py-2 font-medium transition-colors ${
              mode === "practice"
                ? "bg-blue-600 text-white"
                : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
            }`}
            onClick={() => setMode("practice")}
            disabled={running}
          >
            Practice
          </button>
          <button
            className={`flex-1 rounded-lg py-2 font-medium transition-colors ${
              mode === "shadow"
                ? "bg-blue-600 text-white"
                : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
            }`}
            onClick={() => setMode("shadow")}
            disabled={running}
          >
            Shadow
          </button>
        </div>
      </div>

      {/* Session controls */}
      <div className="mb-6 flex w-full max-w-lg gap-3">
        {!running ? (
          <button
            className="flex-1 rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
            onClick={handleStart}
          >
            Iniciar Sesión
          </button>
        ) : (
          <button
            className="flex-1 rounded-lg bg-red-600 py-3 font-medium transition-colors hover:bg-red-500"
            onClick={handleStop}
          >
            Detener Sesión
          </button>
        )}

        <button
          className={`rounded-lg px-6 py-3 font-medium transition-colors ${
            panicking
              ? "bg-orange-500 text-white"
              : "border border-zinc-600 text-zinc-400 hover:border-zinc-500"
          }`}
          onClick={handlePanic}
          disabled={!running}
        >
          {panicking ? "🔇 10s" : "Pánico"}
        </button>
      </div>

      {/* Log area */}
      <div className="w-full max-w-lg space-y-3">
        <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-4">
          <h3 className="mb-2 text-sm font-semibold text-zinc-500 uppercase tracking-wide">
            Último transcript
          </h3>
          <p className="text-sm text-zinc-300 min-h-[1.25rem]">
            {lastTranscript || "\u00a0"}
          </p>
        </div>

        <div className="rounded-xl border border-zinc-700 bg-zinc-900 p-4">
          <h3 className="mb-2 text-sm font-semibold text-zinc-500 uppercase tracking-wide">
            Último hint
          </h3>
          <p className="text-sm text-zinc-300 min-h-[1.25rem]">
            {lastHint || "\u00a0"}
          </p>
        </div>
      </div>
    </div>
  );
}

function App() {
  const [isOverlay, setIsOverlay] = useState(false);

  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (win.label === "overlay") {
      setIsOverlay(true);
    }
  }, []);

  if (isOverlay) {
    return <Overlay />;
  }

  return <MainApp />;
}

export default App;
