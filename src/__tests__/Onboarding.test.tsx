import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import Onboarding, { sanitizeError, isValidFolderPath } from "../Onboarding";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(vi.fn())),
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: vi.fn(() => ({ label: "main" })),
}));

describe("Onboarding — checking state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows header and description before invoke resolves", () => {
    vi.mocked(invoke).mockReturnValue(new Promise<boolean>(() => {}));
    render(<Onboarding onComplete={vi.fn()} />);

    expect(screen.getByText(/configuración inicial/i)).toBeInTheDocument();
    expect(screen.getByText(/preparar kue/i)).toBeInTheDocument();
  });

  it("does not call onComplete while checking", () => {
    vi.mocked(invoke).mockReturnValue(new Promise<boolean>(() => {}));
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);
    expect(onComplete).not.toHaveBeenCalled();
  });
});

describe("Onboarding — screen_permission step", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows screen_permission step when permission denied", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // check_screen_recording_permission
      .mockResolvedValueOnce(false); // is_embedding_model_loaded
    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/permiso de grabación/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /conceder permiso/i })).toBeInTheDocument();
  });

  it("transitions to folder_selection when permission granted and model loaded", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true); // is_embedding_model_loaded
    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/indexar proyectos/i)).toBeInTheDocument();
  });

  it("transitions to embedding_model when permission granted but model not loaded", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(false); // is_embedding_model_loaded
    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/modelo de embeddings/i)).toBeInTheDocument();
    expect(screen.getByText(/cargando modelo/i)).toBeInTheDocument();
  });

  it("calls check_screen_recording_permission on mount", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)
      .mockResolvedValueOnce(true);
    render(<Onboarding onComplete={vi.fn()} />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("check_screen_recording_permission");
    });
  });
});

describe("Onboarding — handleGrantPermission", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows success message when grant succeeds", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // initial check_screen_recording_permission
      .mockResolvedValueOnce(false) // initial is_embedding_model_loaded
      .mockResolvedValueOnce(true)  // grant: check_screen_recording_permission
      .mockResolvedValueOnce(true); // grant: is_embedding_model_loaded
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    await screen.findByText(/permiso de grabación/i);
    const btn = screen.getByRole("button", { name: /conceder permiso/i });
    await userEvent.click(btn);

    expect(await screen.findByText(/indexar proyectos/i)).toBeInTheDocument();
  });

  it("shows error message when grant still denied", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // initial check_screen_recording_permission
      .mockResolvedValueOnce(false) // initial is_embedding_model_loaded
      .mockResolvedValueOnce(false); // grant: still denied
    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/permiso de grabación/i);
    const btn = screen.getByRole("button", { name: /conceder permiso/i });
    await userEvent.click(btn);

    expect(await screen.findByText(/permiso denegado/i)).toBeInTheDocument();
  });

  it("shows friendly error when invoke rejects", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // initial check
      .mockResolvedValueOnce(false) // initial model check
      .mockRejectedValueOnce(new Error("PERMISSION_DENIED: some internal detail")); // grant
    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/permiso de grabación/i);
    const btn = screen.getByRole("button", { name: /conceder permiso/i });
    await userEvent.click(btn);

    expect(await screen.findByText(/permiso denegado por el sistema/i)).toBeInTheDocument();
  });
});

describe("Onboarding — embedding_model step", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows embedding_model step when model not loaded", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(false); // is_embedding_model_loaded → not ready

    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/modelo de embeddings/i)).toBeInTheDocument();
    expect(screen.getByText(/cargando modelo/i)).toBeInTheDocument();
  });

  it("transitions to folder_selection when model finishes loading via poll", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(false) // initial is_embedding_model_loaded → not ready
      .mockResolvedValueOnce(true); // poll: ready!

    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/modelo de embeddings/i);

    vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => {
      expect(screen.getByText(/indexar proyectos/i)).toBeInTheDocument();
    });

    vi.useRealTimers();
  });

  it("stops polling when step changes away", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(false) // initial is_embedding_model_loaded
      .mockResolvedValueOnce(true); // poll resolves → transitions

    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/modelo de embeddings/i);
    const invokeCountBefore = vi.mocked(invoke).mock.calls.length;

    vi.advanceTimersByTimeAsync(1000);
    await screen.findByText(/indexar proyectos/i);
    vi.advanceTimersByTimeAsync(3000);

    expect(vi.mocked(invoke).mock.calls.length).toBe(invokeCountBefore + 1);

    vi.useRealTimers();
  });
});

describe("Onboarding — folder_selection step", () => {
  function renderAtFolderStep(onComplete = vi.fn()) {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true); // is_embedding_model_loaded
    return render(<Onboarding onComplete={onComplete} />);
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows folder selection input and skip button", async () => {
    renderAtFolderStep();

    expect(await screen.findByText(/indexar proyectos/i)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/ruta absoluta/i)).toBeInTheDocument();
    expect(screen.getByText(/saltar/i)).toBeInTheDocument();
  });

  it("disables index button when path contains ..", async () => {
    renderAtFolderStep();

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/Users/test/../../etc");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    expect(btn).toBeDisabled();
    expect(screen.getByText(/path traversal/i)).toBeInTheDocument();
  });

  it("disables index button when path is not absolute", async () => {
    renderAtFolderStep();

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "relative/path");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    expect(btn).toBeDisabled();
    expect(screen.getByText(/ruta absoluta/i)).toBeInTheDocument();
  });

  it("shows error and does not invoke when path validation fails on click", async () => {
    renderAtFolderStep();

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/valid/path/../../escape");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    await userEvent.click(btn);

    expect(invoke).not.toHaveBeenCalledWith("index_folder_cmd", expect.anything());
    expect(screen.getByText(/path traversal/i)).toBeInTheDocument();
  });

  it("calls index_folder_cmd with valid path", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true)  // is_embedding_model_loaded
      .mockResolvedValueOnce(5);    // index_folder_cmd → 5 docs
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/Users/test/Documents");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    await userEvent.click(btn);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("index_folder_cmd", {
        path: "/Users/test/Documents",
      });
    });
    expect(await screen.findByText(/indexados 5 documentos/i)).toBeInTheDocument();
  });

  it("shows friendly error when index_folder_cmd fails", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true)  // is_embedding_model_loaded
      .mockRejectedValueOnce(new Error("STREAM_ERROR: something broke")); // index_folder_cmd
    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/Users/test/Documents");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    await userEvent.click(btn);

    expect(await screen.findByText(/error al iniciar la captura/i)).toBeInTheDocument();
  });

  it("calls mark_onboarding_done on skip", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true)  // is_embedding_model_loaded
      .mockResolvedValueOnce(undefined); // mark_onboarding_done
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    await screen.findByText(/indexar proyectos/i);
    const skipBtn = screen.getByText(/saltar/i).closest("button")!;
    await userEvent.click(skipBtn);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("mark_onboarding_done");
    });
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it("shows Comenzar button and calls onComplete after indexing", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true)  // is_embedding_model_loaded
      .mockResolvedValueOnce(3)     // index_folder_cmd
      .mockResolvedValueOnce(undefined); // mark_onboarding_done
    const onComplete = vi.fn();
    render(<Onboarding onComplete={onComplete} />);

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/Users/test/Documents");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    await userEvent.click(btn);

    await waitFor(() => {
      expect(screen.getByText(/comenzar/i)).toBeInTheDocument();
    });
    const startBtn = screen.getByText(/comenzar/i).closest("button")!;
    await userEvent.click(startBtn);

    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });
  });
});

describe("sanitizeError", () => {
  it("returns generic message for unknown errors", () => {
    expect(sanitizeError("random unknown error")).toBe("Ocurrió un error inesperado. Intenta de nuevo.");
  });

  it("maps PERMISSION_DENIED to friendly message", () => {
    expect(sanitizeError("PERMISSION_DENIED: need access")).toBe("Permiso denegado por el sistema.");
  });

  it("maps STREAM_ERROR to friendly message", () => {
    expect(sanitizeError("STREAM_ERROR: failed")).toBe("Error al iniciar la captura de audio.");
  });

  it("handles empty string", () => {
    expect(sanitizeError("")).toBe("Ocurrió un error inesperado. Intenta de nuevo.");
  });
});

describe("isValidFolderPath", () => {
  it("rejects empty path", () => {
    expect(isValidFolderPath("")).toBe("La ruta no puede estar vacía.");
  });

  it("rejects paths with ..", () => {
    expect(isValidFolderPath("/Users/../etc")).toContain("path traversal");
  });

  it("rejects relative paths", () => {
    expect(isValidFolderPath("relative/path")).toContain("ruta absoluta");
  });

  it("rejects paths with shell metacharacters", () => {
    expect(isValidFolderPath("/foo|bar")).toContain("caracteres no válidos");
  });

  it("accepts valid absolute path", () => {
    expect(isValidFolderPath("/Users/test/Documents")).toBeNull();
  });

  it("rejects whitespace-only path as empty", () => {
    expect(isValidFolderPath("   ")).toBe("La ruta no puede estar vacía.");
  });

  it("rejects path with < character", () => {
    expect(isValidFolderPath("/foo<bar")).toContain("caracteres no válidos");
  });

  it("rejects path with > character", () => {
    expect(isValidFolderPath("/foo>bar")).toContain("caracteres no válidos");
  });

  it('rejects path with double quote character', () => {
    expect(isValidFolderPath('/foo"bar')).toContain("caracteres no válidos");
  });

  it("rejects path with ? character", () => {
    expect(isValidFolderPath("/foo?bar")).toContain("caracteres no válidos");
  });

  it("rejects path with * character", () => {
    expect(isValidFolderPath("/foo*bar")).toContain("caracteres no válidos");
  });
});

describe("sanitizeError — additional branches", () => {
  it("maps DEVICE_NOT_FOUND to friendly message", () => {
    expect(sanitizeError("DEVICE_NOT_FOUND: microphone")).toBe("No se encontró el dispositivo de audio.");
  });

  it("maps error with extra context still finds the key", () => {
    expect(sanitizeError("Error: STREAM_ERROR: buffer overflow at line 42")).toBe("Error al iniciar la captura de audio.");
  });

  it("handles Error object", () => {
    expect(sanitizeError(new Error("PERMISSION_DENIED"))).toBe("Permiso denegado por el sistema.");
  });

  it("handles null/undefined gracefully", () => {
    // @ts-expect-error — testing runtime robustness
    expect(sanitizeError(null)).toBe("Ocurrió un error inesperado. Intenta de nuevo.");
  });
});

describe("Onboarding — error on mount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows error and transitions to screen_permission when initial check rejects", async () => {
    vi.mocked(invoke).mockRejectedValueOnce(new Error("PERMISSION_DENIED: system denied"));
    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/permiso de grabación/i)).toBeInTheDocument();
    expect(screen.getByText(/permiso denegado por el sistema/i)).toBeInTheDocument();
  });

  it("calls sanitizeError with unknown error on mount reject", async () => {
    vi.mocked(invoke).mockRejectedValueOnce("DBus error: connection refused");
    render(<Onboarding onComplete={vi.fn()} />);

    expect(await screen.findByText(/error inesperado/i)).toBeInTheDocument();
  });
});

describe("Onboarding — cleanup / unmount", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does not set state after unmount during initial check", async () => {
    let deferredResolve!: (value: boolean) => void;
    const deferred = new Promise<boolean>((resolve) => {
      deferredResolve = resolve;
    });
    vi.mocked(invoke).mockReturnValueOnce(deferred);
    const onComplete = vi.fn();

    const { unmount } = render(<Onboarding onComplete={onComplete} />);
    unmount();

    // Now resolve — state setters should be no-ops
    deferredResolve(false);
    await vi.waitFor(() => {
      expect(onComplete).not.toHaveBeenCalled();
    });
  });
});

describe("Onboarding — embedding_model polling error paths", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("keeps polling when invoke rejects during model check", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)   // check_screen_recording_permission
      .mockResolvedValueOnce(false)  // initial is_embedding_model_loaded → not ready
      .mockRejectedValueOnce(new Error("temporary error")) // poll rejects
      .mockResolvedValueOnce(true);  // next poll succeeds → transition

    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/modelo de embeddings/i);

    // First poll tick — invoke rejects, stays on embedding_model
    await vi.advanceTimersByTimeAsync(1000);
    expect(screen.getByText(/modelo de embeddings/i)).toBeInTheDocument();

    // Second poll tick — invoke succeeds
    await vi.advanceTimersByTimeAsync(1000);
    await vi.waitFor(() => {
      expect(screen.getByText(/indexar proyectos/i)).toBeInTheDocument();
    });

    vi.useRealTimers();
  });
});

describe("Onboarding — folder_selection indexing state", () => {
  function renderAtFolderStep() {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true); // is_embedding_model_loaded
    return render(<Onboarding onComplete={vi.fn()} />);
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows Indexando... and disables button while indexing", async () => {
    // Keep the index_folder_cmd promise pending so we can observe intermediate state
    let deferredResolve!: (value: number) => void;
    const deferred = new Promise<number>((resolve) => {
      deferredResolve = resolve;
    });
    vi.mocked(invoke)
      .mockResolvedValueOnce(true)  // check_screen_recording_permission
      .mockResolvedValueOnce(true)  // is_embedding_model_loaded
      .mockReturnValueOnce(deferred); // index_folder_cmd stays pending

    render(<Onboarding onComplete={vi.fn()} />);

    await screen.findByText(/indexar proyectos/i);
    const input = screen.getByPlaceholderText(/ruta absoluta/i);
    await userEvent.type(input, "/Users/test/Documents");

    const btn = screen.getByRole("button", { name: /indexar carpeta/i });
    await userEvent.click(btn);

    // Button should show loading state
    expect(screen.getByRole("button", { name: /indexando/i })).toBeDisabled();

    // Resolve the indexing
    deferredResolve(3);
    await vi.waitFor(() => {
      expect(screen.getByText(/indexados 3 documentos/i)).toBeInTheDocument();
    });
  });
});
