import { act, render, screen, waitFor } from "@testing-library/react";
import { vi, type Mock } from "vitest";
import type { Settings } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn(),
  updateUiPrefs: vi.fn(),
}));

import { getSettings, updateUiPrefs } from "@/api/memory";
import { PrefsProvider, usePrefs } from "./PrefsProvider";

const base: Settings = { language: "zh", theme: "dark", accent: null, manual_projects: [] };

function Probe() {
  const p = usePrefs();
  return (
    <div>
      <span data-testid="state">{`${p.language}|${p.theme}`}</span>
      <button onClick={() => p.setLanguage("en")}>lang</button>
      <button onClick={() => p.setTheme("light")}>theme</button>
      <button onClick={() => p.setAccent("teal")}>accent</button>
      <span data-testid="accent">{String(p.accent)}</span>
    </div>
  );
}

async function setup() {
  (getSettings as Mock).mockResolvedValue(base);
  render(
    <PrefsProvider>
      <Probe />
    </PrefsProvider>,
  );
  await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("zh|dark"));
}

const click = (name: string) =>
  act(async () => {
    screen.getByText(name).click();
  });

beforeEach(() => vi.clearAllMocks());
afterEach(() => vi.restoreAllMocks());

describe("PrefsProvider", () => {
  it("rolls back and logs when persisting fails", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    await setup();
    (updateUiPrefs as Mock).mockRejectedValue(new Error("disk full"));
    await click("lang");
    await waitFor(() => expect(screen.getByTestId("state")).toHaveTextContent("zh|dark"));
    expect(err).toHaveBeenCalled();
  });

  it("keeps both new values when responses arrive out of order", async () => {
    await setup();
    let resolveLang!: (s: Settings) => void;
    (updateUiPrefs as Mock)
      .mockImplementationOnce(() => new Promise<Settings>((r) => (resolveLang = r)))
      .mockImplementationOnce(async () => ({ ...base, language: "zh", theme: "light" }));
    await click("lang");
    await click("theme");
    // The first response carries the stale theme and arrives last.
    await act(async () => resolveLang({ ...base, language: "en", theme: "dark" }));
    expect(screen.getByTestId("state")).toHaveTextContent("en|light");
  });

  it("accent only adopts its own field and rolls back on failure", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    await setup();
    (updateUiPrefs as Mock).mockRejectedValue(new Error("nope"));
    await click("accent");
    await waitFor(() => expect(screen.getByTestId("accent")).toHaveTextContent("null"));
    expect(err).toHaveBeenCalled();
    (updateUiPrefs as Mock).mockResolvedValue({ ...base, language: "en", theme: "light", accent: "teal" });
    await click("accent");
    await waitFor(() => expect(screen.getByTestId("accent")).toHaveTextContent("teal"));
    // Fields other than accent are not taken from the accent response.
    expect(screen.getByTestId("state")).toHaveTextContent("zh|dark");
  });
});
