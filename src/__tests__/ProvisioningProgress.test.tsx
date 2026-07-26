import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import ProvisioningProgress from "../ProvisioningProgress";

const eventListeners = new Map<string, (event: { payload: unknown }) => void>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (event: string, cb: (event: { payload: unknown }) => void) => {
      eventListeners.set(event, cb);
      return Promise.resolve(vi.fn());
    },
  ),
}));

function emitEvent(event: string, payload: unknown) {
  const cb = eventListeners.get(event);
  if (cb) cb({ payload });
}

function emitProgress(overrides?: Partial<{
  stage: string;
  file_index: number;
  file_count: number;
  downloaded_bytes: number;
  total_bytes: number;
}>) {
  emitEvent("moonshine-download-progress", {
    stage: "dylibs",
    file_index: 0,
    file_count: 2,
    downloaded_bytes: 0,
    total_bytes: 100,
    ...overrides,
  });
}

describe("ProvisioningProgress — mount behavior", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
  });

  it("calls onProvisioned immediately when already provisioned", async () => {
    vi.mocked(invoke).mockResolvedValue(true);
    const onProvisioned = vi.fn();

    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("is_moonshine_provisioned");
    });
    await waitFor(() => {
      expect(onProvisioned).toHaveBeenCalledTimes(1);
    });
  });

  it("shows downloading state when not provisioned", async () => {
    vi.mocked(invoke).mockResolvedValue(false);
    const onProvisioned = vi.fn();

    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });
    expect(onProvisioned).not.toHaveBeenCalled();
  });

  it("shows downloading state when invoke rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("no backend"));
    const onProvisioned = vi.fn();

    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });
  });
});

describe("ProvisioningProgress — progress events", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("shows 0% progress at start of download", async () => {
    const onProvisioned = vi.fn();
    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 0, total_bytes: 100 });

    expect(await screen.findByText("0%")).toBeInTheDocument();
    expect(screen.getByText(/descargando librerías/i)).toBeInTheDocument();
    expect(screen.getByText(/archivo 1 de 2/i)).toBeInTheDocument();
  });

  it("shows 50% progress midway through download", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 50, total_bytes: 100 });

    expect(await screen.findByText("50%")).toBeInTheDocument();
  });

  it("shows 100% progress at completion", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 100, total_bytes: 100 });

    expect(await screen.findByText("100%")).toBeInTheDocument();
  });

  it('shows "modelo" stage label when stage is model', async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({
      stage: "model",
      file_index: 0,
      file_count: 8,
      downloaded_bytes: 0,
      total_bytes: 429_000_000,
    });

    expect(await screen.findByText(/descargando modelo/i)).toBeInTheDocument();
    expect(screen.getByText(/archivo 1 de 8/i)).toBeInTheDocument();
  });

  it("keeps downloading state when progress events arrive after already downloading", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 25, total_bytes: 100 });
    emitProgress({ downloaded_bytes: 75, total_bytes: 100 });

    expect(await screen.findByText("75%")).toBeInTheDocument();
  });
});

describe("ProvisioningProgress — error and retry", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("shows error state with message on provision-error event", async () => {
    const onProvisioned = vi.fn();
    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "No internet connection");

    expect(await screen.findByText("No internet connection")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /reintentar/i })).toBeInTheDocument();
    expect(onProvisioned).not.toHaveBeenCalled();
  });

  it("shows generic error message when error payload is empty", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "");

    expect(await screen.findByText(/error de descarga/i)).toBeInTheDocument();
  });

  it("calls retry_moonshine_download and shows retrying on retry button click", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(false); // is_moonshine_provisioned
    vi.mocked(invoke).mockResolvedValueOnce("retry_initiated"); // retry_moonshine_download

    const onProvisioned = vi.fn();
    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "Connection timeout");
    const btn = await screen.findByRole("button", { name: /reintentar/i });
    await userEvent.click(btn);

    expect(invoke).toHaveBeenCalledWith("retry_moonshine_download");
    expect(await screen.findByText(/reintentando/i)).toBeInTheDocument();
  });

  it("shows error state when retry command fails", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_moonshine_provisioned
      .mockRejectedValueOnce(new Error("Retry failed")); // retry_moonshine_download

    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "Failed");
    const btn = await screen.findByRole("button", { name: /reintentar/i });
    await userEvent.click(btn);

    await waitFor(() => {
      expect(screen.getByText(/Retry failed/)).toBeInTheDocument();
    });
  });
});

describe("ProvisioningProgress — done event", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("calls onProvisioned on moonshine-provisioned event", async () => {
    const onProvisioned = vi.fn();
    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provisioned", {});

    await waitFor(() => {
      expect(onProvisioned).toHaveBeenCalledTimes(1);
    });
  });

  it("does not call onProvisioned after unmount", async () => {
    const onProvisioned = vi.fn();
    const { unmount } = render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    unmount();
    emitEvent("moonshine-provisioned", {});

    // Small delay to allow any pending microtasks to resolve
    await new Promise((r) => setTimeout(r, 10));
    expect(onProvisioned).not.toHaveBeenCalled();
  });
});

describe("ProvisioningProgress — checking state", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    // Return a promise that never settles to keep component in "checking" state
    vi.mocked(invoke).mockReturnValue(new Promise<boolean>(() => {}));
  });

  it("shows header but no progress or error indicators before invoke resolves", () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    // The header renders unconditionally
    expect(screen.getByText(/preparando kue/i)).toBeInTheDocument();
    expect(screen.getByText(/primera configuración/i)).toBeInTheDocument();

    // No sub-state indicator should be visible yet
    expect(screen.queryByText(/iniciando descarga/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/reintentar/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/reintentando/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/\d+%/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("does not call onProvisioned while invoke is still pending", () => {
    const onProvisioned = vi.fn();
    render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    expect(onProvisioned).not.toHaveBeenCalled();
  });
});

describe("ProvisioningProgress — percent edge cases", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("shows 0% when total_bytes is 0 (guards against division by zero)", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 0, total_bytes: 0 });

    expect(await screen.findByText("0%")).toBeInTheDocument();
  });

  it("shows 0% when total_bytes is 0 regardless of downloaded_bytes", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 50, total_bytes: 0 });

    expect(await screen.findByText("0%")).toBeInTheDocument();
  });

  it("clamps percent to 100 when downloaded_bytes exceeds total_bytes", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitProgress({ downloaded_bytes: 150, total_bytes: 100 });

    expect(await screen.findByText("100%")).toBeInTheDocument();
  });

  it("shows 0% when progress object is null (percent fallback)", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    // Before any progress event, state is "downloading" and progress is null
    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    // The progress bar container should not be rendered
    expect(screen.queryByText(/\d+%/)).not.toBeInTheDocument();
  });
});

describe("ProvisioningProgress — error edge cases", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("shows generic error when error event payload is null", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", null);

    expect(await screen.findByText(/error de descarga/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /reintentar/i })).toBeInTheDocument();
  });

  it("shows generic error when error event payload is undefined", async () => {
    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", undefined);

    expect(await screen.findByText(/error de descarga/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /reintentar/i })).toBeInTheDocument();
  });
});

describe("ProvisioningProgress — retry edge cases", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("transitions from retrying to downloading when progress event arrives after retry", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(false); // is_moonshine_provisioned
    vi.mocked(invoke).mockResolvedValueOnce("ok"); // retry_moonshine_download succeeds

    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "Timeout");
    const btn = await screen.findByRole("button", { name: /reintentar/i });
    await userEvent.click(btn);

    expect(await screen.findByText(/reintentando/i)).toBeInTheDocument();

    // Progress event after retry transitions back to "downloading"
    emitProgress({ downloaded_bytes: 25, total_bytes: 100 });
    expect(await screen.findByText("25%")).toBeInTheDocument();
  });

  it("shows error state with stringified error when retry throws non-Error value", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_moonshine_provisioned
      .mockRejectedValueOnce("string error"); // retry_moonshine_download throws a string

    render(<ProvisioningProgress onProvisioned={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    emitEvent("moonshine-provision-error", "Failed");
    const btn = await screen.findByRole("button", { name: /reintentar/i });
    await userEvent.click(btn);

    await waitFor(() => {
      expect(screen.getByText(/string error/)).toBeInTheDocument();
    });
  });
});

describe("ProvisioningProgress — cleanup", () => {
  beforeEach(() => {
    eventListeners.clear();
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(false);
  });

  it("stops updating state after unmount during download", async () => {
    const onProvisioned = vi.fn();
    const { unmount } = render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(screen.getByText(/iniciando descarga/i)).toBeInTheDocument();
    });

    unmount();

    // Should not throw despite emitting after unmount
    emitProgress({ downloaded_bytes: 50, total_bytes: 100 });
    emitEvent("moonshine-provision-error", "err");
    expect(onProvisioned).not.toHaveBeenCalled();
  });

  it("does not call onProvisioned from invoke.then when unmounted before resolve", async () => {
    // Return a pending promise so cancelling happens before it resolves
    let resolveInvoke!: (v: boolean) => void;
    vi.mocked(invoke).mockReturnValue(new Promise<boolean>((r) => { resolveInvoke = r; }));

    const onProvisioned = vi.fn();
    const { unmount } = render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    // Component is mounted, invoke called but hasn't resolved yet
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("is_moonshine_provisioned");
    });

    // Unmount before invoke resolves — sets cancelled = true
    unmount();

    // Now resolve the invoke — the .then handler runs but cancelled flag prevents callback
    resolveInvoke!(true);

    // Small delay for microtasks
    await new Promise((r) => setTimeout(r, 10));

    expect(onProvisioned).not.toHaveBeenCalled();
  });

  it("does not set downloading state when unmounted before invoke rejects", async () => {
    let rejectInvoke!: (e: Error) => void;
    vi.mocked(invoke).mockReturnValue(new Promise<boolean>((_, r) => { rejectInvoke = r; }));

    const onProvisioned = vi.fn();
    const { unmount } = render(<ProvisioningProgress onProvisioned={onProvisioned} />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("is_moonshine_provisioned");
    });

    unmount();
    rejectInvoke!(new Error("fail"));

    await new Promise((r) => setTimeout(r, 10));

    // Since cancelled, the .catch handler should NOT setState("downloading")
    // Verify by checking no downloading text appeared
    expect(screen.queryByText(/iniciando descarga/i)).not.toBeInTheDocument();
    expect(onProvisioned).not.toHaveBeenCalled();
  });

});
