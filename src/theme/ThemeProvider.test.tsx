import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { Settings } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn(),
  updateUiPrefs: vi.fn(),
}));

import { getSettings, updateUiPrefs } from "@/api/memory";
import { PrefsProvider, usePrefs } from "@/prefs/PrefsProvider";
import { ThemeProvider } from "./ThemeProvider";

const base: Settings = { language: "zh", theme: "dark", accent: null, manual_projects: [] };

function Probe() {
  const p = usePrefs();
  return (
    <div>
      <span data-testid="accent">{String(p.accent)}</span>
      <button onClick={() => p.setAccent("blue")}>blue</button>
    </div>
  );
}

function setup() {
  render(
    <PrefsProvider>
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    </PrefsProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  document.documentElement.removeAttribute("data-accent");
  (getSettings as Mock).mockResolvedValue(base);
});
afterEach(() => vi.restoreAllMocks());

describe("accent color", () => {
  it("accent defaults to green data attribute", async () => {
    setup();
    await waitFor(() => expect(document.documentElement.dataset.accent).toBe("green"));
  });

  it("adopts the stored accent", async () => {
    (getSettings as Mock).mockResolvedValue({ ...base, accent: "orange" });
    setup();
    await waitFor(() => expect(document.documentElement.dataset.accent).toBe("orange"));
  });

  it("choosing accent sets data-accent and persists", async () => {
    (updateUiPrefs as Mock).mockResolvedValue({ ...base, accent: "blue" });
    setup();
    await act(async () => {
      await userEvent.click(screen.getByText("blue"));
    });
    expect(document.documentElement.dataset.accent).toBe("blue");
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, undefined, "blue");
  });

  it("accent failure rolls back", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    (updateUiPrefs as Mock).mockRejectedValue(new Error("disk full"));
    setup();
    await waitFor(() => expect(document.documentElement.dataset.accent).toBe("green"));
    await act(async () => {
      await userEvent.click(screen.getByText("blue"));
    });
    await waitFor(() => expect(document.documentElement.dataset.accent).toBe("green"));
    expect(screen.getByTestId("accent")).toHaveTextContent("null");
    expect(err).toHaveBeenCalled();
  });

  it("ignores unknown stored accents", async () => {
    (getSettings as Mock).mockResolvedValue({ ...base, accent: "pink" });
    setup();
    await waitFor(() => expect(getSettings).toHaveBeenCalled());
    expect(document.documentElement.dataset.accent).toBe("green");
  });

  it("first render uses the cached accent and theme (no flash)", async () => {
    localStorage.setItem("memdock.accent", "violet");
    localStorage.setItem("memdock.theme", "light");
    (getSettings as Mock).mockReturnValue(new Promise(() => {}));
    document.documentElement.classList.add("dark");
    setup();
    expect(document.documentElement.dataset.accent).toBe("violet");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
