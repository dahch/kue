import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { PostCallPanel, type SessionRow } from "../App";

function session(overrides?: Partial<SessionRow>): SessionRow {
  return {
    id: "session-1",
    started_at: "2024-01-01T10:00:00Z",
    ended_at: null,
    company: "Acme",
    role: "Engineer",
    mode: "practice",
    line_count: 42,
    ...overrides,
  };
}

const eventListeners = new Map<string, (event: { payload: Record<string, unknown> }) => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (event: string, cb: (event: { payload: Record<string, unknown> }) => void) => {
      eventListeners.set(event, cb);
      return Promise.resolve(vi.fn());
    },
  ),
}));

function emitEvent(event: string, payload: Record<string, unknown>) {
  const cb = eventListeners.get(event);
  if (cb) cb({ payload });
}

describe("PostCallPanel gating", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
  });

  it("disables the button and shows processing state when transcript is not ready", async () => {
    vi.mocked(invoke).mockResolvedValue(false);

    render(<PostCallPanel session={session()} />);

    const btn = await screen.findByRole("button", { name: /procesando transcripción/i });
    expect(btn).toBeDisabled();
  });

  it("enables the button when post-call-transcript-ready fires for the current session", async () => {
    vi.mocked(invoke).mockResolvedValue(false);

    render(<PostCallPanel session={session()} />);

    let btn = await screen.findByRole("button", { name: /procesando transcripción/i });
    expect(btn).toBeDisabled();

    emitEvent("post-call-transcript-ready", { session_id: "session-1" });

    btn = await screen.findByRole("button", { name: /^analizar$/i });
    expect(btn).toBeEnabled();
  });

  it("stays disabled when the event fires for a different session", async () => {
    vi.mocked(invoke).mockResolvedValue(false);

    render(<PostCallPanel session={session()} />);

    let btn = await screen.findByRole("button", { name: /procesando transcripción/i });
    expect(btn).toBeDisabled();

    emitEvent("post-call-transcript-ready", { session_id: "other-session" });

    btn = await screen.findByRole("button", { name: /procesando transcripción/i });
    expect(btn).toBeDisabled();
  });
});
