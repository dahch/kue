/**
 * Kue icon set — geometric stroke icons, 24×24 grid.
 * All icons are decorative: they render aria-hidden and must be paired
 * with a text label or an aria-label on the interactive parent.
 */
export type IconName =
  | "alert"
  | "bolt"
  | "chart"
  | "check"
  | "chevron-down"
  | "clock"
  | "cpu"
  | "eye"
  | "eye-off"
  | "file-text"
  | "folder"
  | "globe"
  | "history"
  | "key"
  | "mic"
  | "mute"
  | "next"
  | "play"
  | "refresh"
  | "shield"
  | "skip"
  | "sliders"
  | "sparkle"
  | "stop"
  | "x";

const PATHS: Record<IconName, string[]> = {
  alert: [
    "M12 3.5 2.5 20h19L12 3.5Z",
    "M12 10v4.5",
    "M12 17.5v.1",
  ],
  bolt: ["M13 2 4.6 13.4h6.3L10 22l8.4-11.4h-6.3L13 2Z"],
  chart: ["M3 20h18", "M6.5 20v-6", "M11.5 20V6", "M16.5 20v-9"],
  check: ["m5 12.5 4.5 4.5L19 7.5"],
  "chevron-down": ["m6 9.5 6 6 6-6"],
  clock: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z", "M12 7.5V12l3 2"],
  cpu: [
    "M7 5h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2Z",
    "M9.5 9.5h5v5h-5z",
    "M9 2.5V5M15 2.5V5M9 19v2.5M15 19v2.5M2.5 9H5M2.5 15H5M19 9h2.5M19 15h2.5",
  ],
  eye: [
    "M2.5 12S6 5.3 12 5.3 21.5 12 21.5 12 18 18.7 12 18.7 2.5 12 2.5 12Z",
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z",
  ],
  "eye-off": [
    "M3.5 3.5l17 17",
    "M10.7 5.5A11 11 0 0 1 12 5.3c6 0 9.5 6.7 9.5 6.7a17 17 0 0 1-2.8 3.4",
    "M6.7 6.7C4.2 8.4 2.5 12 2.5 12S6 18.7 12 18.7c1.5 0 2.8-.4 4-1",
    "M9.9 9.9a3 3 0 0 0 4.2 4.2",
  ],
  "file-text": [
    "M14 2.5H7a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-12l-5-5Z",
    "M14 2.5v5h5",
    "M9 13.5h6M9 17.5h6",
  ],
  folder: [
    "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9L9.6 3.9A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h16Z",
  ],
  globe: [
    "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z",
    "M3 12h18",
    "M12 3a14.5 14.5 0 0 1 0 18 14.5 14.5 0 0 1 0-18Z",
  ],
  history: [
    "M3.5 12a8.5 8.5 0 1 0 2.5-6",
    "M3.5 4.5v4h4",
    "M12 8v4.2l3 1.8",
  ],
  key: [
    "M14 10a5.5 5.5 0 1 1-5.2 3.8",
    "M12.8 11.2 21 3",
    "m15.5 6.5 3 3",
  ],
  mic: [
    "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z",
    "M19 10v2a7 7 0 0 1-14 0v-2",
    "M12 19v3",
  ],
  mute: [
    "M11 5 6.7 9H3.5v6h3.2L11 19V5Z",
    "m16 9.5 5 5",
    "m21 9.5-5 5",
  ],
  next: ["M4.5 5v14l9-7-9-7Z", "M19.5 5v14"],
  play: ["M7.5 4.8v14.4L20 12 7.5 4.8Z"],
  refresh: [
    "M21 12a9 9 0 1 1-2.6-6.3",
    "M21 3.5V9h-5.5",
  ],
  shield: [
    "M12 22s8-3.6 8-10V5.4L12 2 4 5.4V12c0 6.4 8 10 8 10Z",
    "m9 11.6 2.1 2.1 4-4.1",
  ],
  skip: ["M5.5 4.8v14.4L15 12 5.5 4.8Z", "M18.5 5.5v13"],
  sliders: [
    "M4 8h9.5M17.5 8H20",
    "M15.5 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5Z",
    "M4 16h3.5M11.5 16H20",
    "M9.5 13.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5Z",
  ],
  sparkle: [
    "M12 3.5 13.8 9l5.5 1.8-5.5 1.8L12 18l-1.8-5.4L4.7 10.8 10.2 9 12 3.5Z",
    "M18.8 15.5l.9 2.6 2.6.9-2.6.9-.9 2.6-.9-2.6-2.6-.9 2.6-.9.9-2.6Z",
  ],
  stop: ["M8 6.5h8A1.5 1.5 0 0 1 17.5 8v8a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 16V8A1.5 1.5 0 0 1 8 6.5Z"],
  x: ["M18 6 6 18", "M6 6l12 12"],
};

export function Icon({
  name,
  className = "h-4 w-4",
  strokeWidth = 1.8,
}: {
  name: IconName;
  className?: string;
  strokeWidth?: number;
}) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${className}`}
    >
      {PATHS[name].map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}
