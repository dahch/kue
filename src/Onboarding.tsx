import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type OnboardingStep =
  | "checking"
  | "screen_permission"
  | "embedding_model"
  | "folder_selection"
  | "done";

export function sanitizeError(err: unknown): string {
  const msg = `${err}`;
  const known: Record<string, string> = {
    PERMISSION_DENIED: "Permiso denegado por el sistema.",
    DEVICE_NOT_FOUND: "No se encontró el dispositivo de audio.",
    STREAM_ERROR: "Error al iniciar la captura de audio.",
  };
  for (const [key, friendly] of Object.entries(known)) {
    if (msg.includes(key)) return friendly;
  }
  return "Ocurrió un error inesperado. Intenta de nuevo.";
}

export function isValidFolderPath(path: string): string | null {
  if (!path.trim()) return "La ruta no puede estar vacía.";
  if (path.includes("..")) return "La ruta no puede contener '..' (path traversal no permitido).";
  if (/[<>"|?*]/.test(path)) return "La ruta contiene caracteres no válidos.";
  if (!path.startsWith("/")) return "Debes introducir una ruta absoluta (que empiece con /).";
  return null;
}

function Onboarding({ onComplete }: { onComplete: () => void }) {
  const [step, setStep] = useState<OnboardingStep>("checking");
  const [screenGranted, setScreenGranted] = useState(false);
  const [modelLoaded, setModelLoaded] = useState(false);
  const [folderPath, setFolderPath] = useState("");
  const [indexing, setIndexing] = useState(false);
  const [indexResult, setIndexResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pathError = useMemo(() => isValidFolderPath(folderPath), [folderPath]);

  // On mount, check current state
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

  // Poll embedding model load status
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
          "Permiso denegado. Ve a System Settings → Privacy & Security → " +
            "Screen & System Audio Recording y activa Kue. Luego haz clic en Reintentar.",
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
      const count: number = await invoke("index_folder_cmd", {
        path: trimmed,
      });
      setIndexResult(`Indexados ${count} documentos.`);
    } catch (e) {
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
        <h1 className="mb-2 text-2xl font-bold">Configuración inicial</h1>
        <p className="mb-6 text-sm text-zinc-400">
          Vamos a preparar Kue para tu primera sesión.
        </p>

        {/* Screen recording permission */}
        {step === "screen_permission" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                1. Permiso de grabación de pantalla
              </h2>
              <p className="mb-3 text-sm text-zinc-300">
                Kue necesita permiso para capturar el audio del entrevistador
                a través de ScreenCaptureKit. Sin este permiso no podremos
                transcribir las preguntas.
              </p>
              <p className="mb-3 text-xs text-zinc-500">
                Si el sistema no te ha pedido permiso aún, haz clic en el
                botón. Se abrirá una ventana del sistema — concede el
                permiso y luego haz clic en "Reintentar".
              </p>
              {screenGranted ? (
                <p className="text-sm text-emerald-400">
                  Permiso concedido ✓
                </p>
              ) : (
                <button
                  className="w-full rounded-lg bg-blue-600 py-3 font-medium transition-colors hover:bg-blue-500"
                  onClick={handleGrantPermission}
                >
                  Conceder permiso
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

        {/* Embedding model loading */}
        {step === "embedding_model" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                2. Modelo de embeddings
              </h2>
              <div className="flex items-center gap-3 text-sm text-zinc-300">
                <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-zinc-500 border-t-zinc-300" />
                Cargando modelo (primera vez: descarga ~95 MB)...
              </div>
              <p className="mt-3 text-xs text-zinc-500">
                El modelo se descarga e indexa en segundo plano. Suele
                tardar unos segundos.
              </p>
            </div>
          </div>
        )}

        {/* Folder selection */}
        {step === "folder_selection" && (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-700 bg-zinc-800/50 p-4">
              <h2 className="mb-2 font-semibold text-blue-400">
                3. Indexar proyectos
              </h2>
              <p className="mb-3 text-sm text-zinc-300">
                Selecciona la carpeta donde tienes tus CV, proyectos y
                métricas. Kue indexará los archivos PDF, TXT y MD para
                usarlos como contexto durante las entrevistas.
              </p>
              <div className="flex gap-2">
                <input
                  type="text"
                  value={folderPath}
                  onChange={(e) => setFolderPath(e.target.value)}
                  placeholder="Ruta absoluta, ej. /Users/tu/Documents/proyectos"
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
                    ? "Indexando..."
                    : "Indexar carpeta"}
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
                Saltar (puedes indexar luego)
              </button>
              {indexResult && (
                <button
                  className="flex-1 rounded-lg bg-emerald-600 py-3 font-medium transition-colors hover:bg-emerald-500"
                  onClick={handleComplete}
                >
                  Comenzar
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
