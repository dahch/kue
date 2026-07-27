import { t } from "./i18n";
import type { IndexSummary } from "./types";

const MAX_ERROR_LENGTH = 300;

export function sanitizeError(err: unknown): string {
  const msg = `${err}`;
  const known: Record<string, string> = {
    PERMISSION_DENIED: t("permissionDenied"),
    DEVICE_NOT_FOUND: t("deviceNotFound"),
    STREAM_ERROR: t("streamError"),
  };
  for (const [key, friendly] of Object.entries(known)) {
    if (msg.includes(key)) return friendly;
  }
  const cleaned = msg.replace(/^Error:\s*/, "").trim();
  if (cleaned && cleaned !== "undefined" && cleaned !== "null") {
    return cleaned.length > MAX_ERROR_LENGTH
      ? `${cleaned.slice(0, MAX_ERROR_LENGTH)}…`
      : cleaned;
  }
  return t("unexpectedError");
}

export function isValidFolderPath(path: string): string | null {
  if (!path.trim()) return t("pathRequired");
  if (path.includes("..")) return t("pathTraversal");
  if (/[<">|?*]/.test(path)) return t("invalidPathChars");
  if (!path.startsWith("/")) return t("absolutePathRequired");
  return null;
}

export function formatIndexResult(summary: IndexSummary): string {
  const chunksText = summary.chunks_created > 0
    ? ` (${summary.chunks_created.toLocaleString()} ${t("chunks")})`
    : "";
  const errorsText = summary.error_count > 0
    ? ` — ${summary.error_count} ${summary.error_count === 1 ? t("error") : t("errors")}`
    : "";
  return `${t("indexedDocuments", { count: summary.files_indexed })}${chunksText}.${errorsText}`;
}
