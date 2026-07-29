import { useCallback, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";

export type Language = "en" | "es";

export const LANGUAGE_STORAGE_KEY = "kue-language";
export const LANGUAGE_SETTING_KEY = "language";

const translations = {
  en: {
    appTitle: "Kue",
    tagline: "Interview copilot",
    mode: "Mode",
    practice: "Practice",
    shadow: "Shadow",
    company: "Company",
    companyPlaceholder: "Acme Corp",
    role: "Role",
    rolePlaceholder: "Senior Engineer",
    optional: "optional",
    startSession: "Start session",
    stopSession: "Stop session",
    stopping: "Stopping...",
    panic: "Panic",
    panicActive: "Panic active",
    panicBanner: "Hints muted for 10s",
    transcript: "Transcript",
    hint: "Latest hint",
    noHint: "No hint yet",
    sessions: "Sessions",
    previousSessions: "Previous sessions",
    reindexFolder: "Re-index folder",
    indexFolder: "Index folder",
    folderPathPlaceholder: "Absolute path, e.g. /Users/you/Documents/projects",
    analyzing: "Analyzing...",
    analyze: "Analyze",
    processingTranscript: "Processing transcript...",
    transcriptComplete: "Transcript complete",
    summary: "Summary",
    weakQuestions: "Weak questions",
    forgottenProjects: "Unmentioned projects",
    starImprovements: "STAR improvements",
    language: "Language",
    settings: "Settings",
    save: "Save",
    show: "show",
    hide: "hide",
    apiKey: "API Key",
    apiKeySaved: "saved",
    provider: "Provider",
    model: "Model",
    modelOptional: "optional",
    error: "Error",
    errors: "errors",
    close: "Close",
    listening: "Listening",
    processing: "Processing",
    lines: "lines",
    line: "line",
    unknown: "Unknown",
    interviewer: "Interviewer",
    user: "You",
    documents: "documents",
    chunks: "chunks",
    indexedDocuments: "Indexed {{count}} documents",
    onboardingTitle: "Initial setup",
    onboardingSubtitle: "Let's get Kue ready for your first session.",
    stepScreenPermission: "1. Audio permissions",
    screenPermissionDescription:
      "Kue needs permission to record your microphone and to capture system audio (the interviewer's voice) via ScreenCaptureKit. macOS groups both under 'Screen & System Audio Recording' even though Kue only captures a 1×1 pixel video stream for the audio.",
    screenPermissionInstructions:
      'If the system has not asked yet, click the button. A system window will open — grant the permission and then click "Retry". The microphone prompt will appear the first time you start a session.',
    permissionGranted: "Permission granted ✓",
    grantPermission: "Grant permission",
    retry: "Retry",
    stepEmbeddingModel: "2. Embedding model",
    loadingModel: "Loading model (first time: downloading ~95 MB)...",
    embeddingModelHint: "The model downloads and indexes in the background. Usually takes a few seconds.",
    stepApiKey: "3. API Key",
    apiKeyDescription: "Configure an LLM provider to enable AI-powered hints during interviews. You can skip this and configure it later.",
    configureLater: "Configure later",
    stepIndexProjects: "4. Index projects",
    indexProjectsDescription:
      "Select the folder with your CV, projects and metrics. Kue will recursively index PDF, TXT and MD files (including subfolders) to use as context during interviews.",
    indexing: "Indexing...",
    skipIndex: "Skip (you can index later)",
    start: "Start",
    preparingKue: "Preparing Kue",
    provisioningSubtitle: "Initial setup — downloading Moonshine (~482 MB)",
    downloadingModel: "Downloading model...",
    downloadingLibs: "Downloading libraries...",
    startingDownload: "Starting download...",
    downloadError: "Download error",
    retrying: "Retrying...",
    fileXOfY: "File {{fileIndex}} of {{fileCount}}",
    modelLabel: "model",
    dylibsLabel: "dylibs",
    permissionDenied: "Permission denied by the system.",
    deviceNotFound: "Audio device not found.",
    streamError: "Error starting audio capture.",
    unexpectedError: "An unexpected error occurred. Please try again.",
    pathRequired: "Path cannot be empty.",
    pathTraversal: "Path cannot contain '..' (path traversal not allowed).",
    invalidPathChars: "Path contains invalid characters.",
    absolutePathRequired: "You must enter an absolute path (starting with /).",
    jobDescriptionPlaceholder: "Paste the job description here...",
    durationMinutes: "Duration (min)",
    generateQuestions: "Generate questions",
    generating: "Generating",
    questionsGenerated: "questions generated",
    question: "Question",
    interviewStatus_speaking: "Kue is speaking",
    interviewStatus_listening: "Listening to your answer",
    interviewStatus_finished: "Interview finished",
    skipQuestion: "Skip question",
    endInterview: "End interview",
    starting: "Starting",
    viewTranscript: "View transcript",
    hideTranscript: "Hide transcript",
    noTranscriptLines: "No transcript lines in this session.",
    loading: "Loading",
    live: "Live",
    sessionSetup: "Session setup",
    practiceDesc: "An AI interviewer drills you with a tailored question plan.",
    shadowDesc: "Kue listens to a real call and feeds you hints live.",
    aiInterview: "AI Interview",
    jobDescription: "Job description",
    planDesc: "Paste a job description and Kue will craft a question plan for the session.",
    viewLogs: "View logs",
    postCall: "Post-call analysis",
    emptyTranscript: "Start a session — the transcript will appear here in real time.",
    listeningCaption: "Capturing the conversation…",
    panicTooltip: "Mute all hints for 10 seconds",
    selectLanguage: "Select language",
    stepLabelPermission: "Permissions",
    stepLabelModel: "Model",
    stepLabelKey: "API Key",
    stepLabelIndex: "Index",
    onboardingStepOf: "Step {{current}} of {{total}}",
    folderPathLabel: "Project folder path",
    tabApiKeys: "API Keys",
    tabLlmDefaults: "LLM Defaults",
    tabGeneral: "General",
    delete: "Delete",
    deleteKeyConfirm: "Are you sure you want to delete the API key for",
    keyNotSaved: "Not configured",
    noKeyRequired: "No API key required",
    globalDefault: "Global default",
    useGlobal: "Use global",
    custom: "Custom",
    hints: "Hints",
    configureAllInSettings: "Configure all in Settings",
    noKeyForProviderMsg: "No API key saved for {{provider}}. Configure one in Settings.",
    openSettings: "Open Settings",
    dataFolder: "Data folder",
  },
  es: {
    appTitle: "Kue",
    tagline: "Copiloto de entrevistas",
    mode: "Modo",
    practice: "Practice",
    shadow: "Shadow",
    company: "Empresa",
    companyPlaceholder: "Acme Corp",
    role: "Rol",
    rolePlaceholder: "Senior Engineer",
    optional: "opcional",
    startSession: "Iniciar sesión",
    stopSession: "Detener sesión",
    stopping: "Deteniendo...",
    panic: "Pánico",
    panicActive: "Pánico activo",
    panicBanner: "Hints silenciados 10s",
    transcript: "Transcripción",
    hint: "Último hint",
    noHint: "Aún no hay hint",
    sessions: "Sesiones",
    previousSessions: "Sesiones anteriores",
    reindexFolder: "Re-indexar carpeta",
    indexFolder: "Indexar carpeta",
    folderPathPlaceholder: "Ruta absoluta, ej. /Users/tu/Documents/proyectos",
    analyzing: "Analizando...",
    analyze: "Analizar",
    processingTranscript: "Procesando transcripción...",
    transcriptComplete: "Transcripción completa",
    summary: "Resumen",
    weakQuestions: "Preguntas débiles",
    forgottenProjects: "Proyectos no mencionados",
    starImprovements: "Mejoras STAR",
    language: "Idioma",
    settings: "Ajustes",
    save: "Guardar",
    show: "mostrar",
    hide: "ocultar",
    apiKey: "API Key",
    apiKeySaved: "guardado",
    provider: "Proveedor",
    model: "Modelo",
    modelOptional: "opcional",
    error: "Error",
    errors: "errores",
    close: "Cerrar",
    listening: "Escuchando",
    processing: "Procesando",
    lines: "líneas",
    line: "línea",
    unknown: "Desconocido",
    interviewer: "Entrevistador",
    user: "Tú",
    documents: "documentos",
    chunks: "fragmentos",
    indexedDocuments: "Indexados {{count}} documentos",
    onboardingTitle: "Configuración inicial",
    onboardingSubtitle: "Vamos a preparar Kue para tu primera sesión.",
    stepScreenPermission: "1. Permisos de audio",
    screenPermissionDescription:
      "Kue necesita permiso para grabar tu micrófono y para capturar el audio del sistema (la voz del entrevistador) a través de ScreenCaptureKit. macOS agrupa ambos bajo 'Grabación de audio del sistema y pantalla' aunque Kue solo captura un video de 1×1 píxel para obtener el audio.",
    screenPermissionInstructions:
      'Si el sistema no te ha pedido permiso aún, haz clic en el botón. Se abrirá una ventana del sistema — concede el permiso y luego haz clic en "Reintentar". El diálogo del micrófono aparecerá la primera vez que inicies una sesión.',
    permissionGranted: "Permiso concedido ✓",
    grantPermission: "Conceder permiso",
    retry: "Reintentar",
    stepEmbeddingModel: "2. Modelo de embeddings",
    loadingModel: "Cargando modelo (primera vez: descarga ~95 MB)...",
    embeddingModelHint: "El modelo se descarga e indexa en segundo plano. Suele tardar unos segundos.",
    stepApiKey: "3. API Key",
    apiKeyDescription: "Configura un proveedor LLM para recibir hints inteligentes durante las entrevistas. Puedes saltar este paso y configurarlo después.",
    configureLater: "Configurar después",
    stepIndexProjects: "4. Indexar proyectos",
    indexProjectsDescription:
      "Selecciona la carpeta donde tienes tus CV, proyectos y métricas. Kue indexará recursivamente los archivos PDF, TXT y MD (incluyendo subcarpetas) para usarlos como contexto durante las entrevistas.",
    indexing: "Indexando...",
    skipIndex: "Saltar (puedes indexar luego)",
    start: "Comenzar",
    preparingKue: "Preparando Kue",
    provisioningSubtitle: "Primera configuración — descargando Moonshine (~482 MB)",
    downloadingModel: "Descargando modelo...",
    downloadingLibs: "Descargando librerías...",
    startingDownload: "Iniciando descarga...",
    downloadError: "Error de descarga",
    retrying: "Reintentando...",
    fileXOfY: "Archivo {{fileIndex}} de {{fileCount}}",
    modelLabel: "modelo",
    dylibsLabel: "dylibs",
    permissionDenied: "Permiso denegado por el sistema.",
    deviceNotFound: "No se encontró el dispositivo de audio.",
    streamError: "Error al iniciar la captura de audio.",
    unexpectedError: "Ocurrió un error inesperado. Intenta de nuevo.",
    pathRequired: "La ruta no puede estar vacía.",
    pathTraversal: "La ruta no puede contener '..' (path traversal no permitido).",
    invalidPathChars: "La ruta contiene caracteres no válidos.",
    absolutePathRequired: "Debes introducir una ruta absoluta (que empiece con /).",
    jobDescriptionPlaceholder: "Pega la descripción del puesto aquí...",
    durationMinutes: "Duración (min)",
    generateQuestions: "Generar preguntas",
    generating: "Generando",
    questionsGenerated: "preguntas generadas",
    question: "Pregunta",
    interviewStatus_speaking: "Kue está hablando",
    interviewStatus_listening: "Escuchando tu respuesta",
    interviewStatus_finished: "Entrevista finalizada",
    skipQuestion: "Saltar pregunta",
    endInterview: "Terminar entrevista",
    starting: "Iniciando",
    viewTranscript: "Ver transcripción",
    hideTranscript: "Ocultar transcripción",
    noTranscriptLines: "No hay líneas de transcripción en esta sesión.",
    loading: "Cargando",
    live: "En vivo",
    sessionSetup: "Configuración de sesión",
    practiceDesc: "Un entrevistador IA te evalúa con un plan de preguntas a medida.",
    shadowDesc: "Kue escucha una llamada real y te da hints en vivo.",
    aiInterview: "Entrevista IA",
    jobDescription: "Descripción del puesto",
    planDesc: "Pega la descripción del puesto y Kue creará el plan de preguntas de la sesión.",
    viewLogs: "Ver logs",
    postCall: "Análisis post-llamada",
    emptyTranscript: "Inicia una sesión — la transcripción aparecerá aquí en tiempo real.",
    listeningCaption: "Capturando la conversación…",
    panicTooltip: "Silencia todos los hints durante 10 s",
    selectLanguage: "Seleccionar idioma",
    stepLabelPermission: "Permisos",
    stepLabelModel: "Modelo",
    stepLabelKey: "API Key",
    stepLabelIndex: "Indexar",
    onboardingStepOf: "Paso {{current}} de {{total}}",
    folderPathLabel: "Ruta de la carpeta de proyectos",
    tabApiKeys: "API Keys",
    tabLlmDefaults: "Defaults LLM",
    tabGeneral: "General",
    delete: "Eliminar",
    deleteKeyConfirm: "¿Estás seguro de eliminar la API key para",
    keyNotSaved: "Sin configurar",
    noKeyRequired: "No requiere API key",
    globalDefault: "Default global",
    useGlobal: "Usar global",
    custom: "Personalizado",
    hints: "Hints",
    configureAllInSettings: "Configurar todo en Ajustes",
    noKeyForProviderMsg: "No hay API key guardada para {{provider}}. Configúrala en Ajustes.",
    openSettings: "Abrir Ajustes",
    dataFolder: "Carpeta de datos",
  },
} as const;

export type Translations = typeof translations.en;

let currentLanguage: Language = "es";

const languageListeners = new Set<() => void>();

export function setLanguage(lang: Language) {
  currentLanguage = lang;
  for (const listener of languageListeners) {
    listener();
  }
}

export function getLanguage(): Language {
  return currentLanguage;
}

function subscribeToLanguage(listener: () => void): () => void {
  languageListeners.add(listener);
  return () => { languageListeners.delete(listener); };
}

export function useLanguage(): Language {
  const subscribe = useCallback((listener: () => void) => subscribeToLanguage(listener), []);
  return useSyncExternalStore(subscribe, getLanguage, getLanguage);
}

export function t(key: keyof Translations, vars?: Record<string, string | number>): string {
  let text: string = translations[currentLanguage][key];
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      text = text.replace(`{{${k}}}`, String(v));
    }
  }
  return text;
}

export function formatLines(count: number): string {
  return `${count} ${count === 1 ? t("line") : t("lines")}`;
}

export function speakerLabel(speaker: string): string {
  if (speaker === "interviewer") return t("interviewer");
  if (speaker === "user") return t("user");
  return t("unknown");
}

function isLanguage(value: string): value is Language {
  return value === "en" || value === "es";
}

/**
 * Synchronously restore the language from localStorage. This avoids a
 * flash of the default language on first render.
 */
export function initLanguage(): void {
  if (typeof window === "undefined") return;
  const stored = window.localStorage.getItem(LANGUAGE_STORAGE_KEY);
  if (stored && isLanguage(stored)) {
    setLanguage(stored);
  }
}

/**
 * Load the persisted language from the backend settings table and update
 * both the in-memory language and localStorage cache.
 */
export async function loadLanguageFromBackend(): Promise<void> {
  try {
    const value = await invoke<string | null>("get_setting", {
      key: LANGUAGE_SETTING_KEY,
    });
    if (value && isLanguage(value)) {
      setLanguage(value);
      if (typeof window !== "undefined") {
        window.localStorage.setItem(LANGUAGE_STORAGE_KEY, value);
      }
    }
  } catch (e) {
    console.warn("Failed to load language from backend:", e);
  }
}

/**
 * Persist the language to localStorage and the backend settings table.
 */
export async function saveLanguage(lang: Language): Promise<void> {
  setLanguage(lang);
  if (typeof window !== "undefined") {
    window.localStorage.setItem(LANGUAGE_STORAGE_KEY, lang);
  }
  try {
    await invoke("set_setting", { key: LANGUAGE_SETTING_KEY, value: lang });
  } catch (e) {
    console.warn("Failed to save language to backend:", e);
  }
}
