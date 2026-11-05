import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Settings as SettingsIcon } from "lucide-react";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: "dark", accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn().mockResolvedValue({ language: "zh", theme: "dark", accent: "blue", manual_projects: [] }),
}));

import { updateUiPrefs } from "@/api/memory";
import { renderApp } from "@/test/fixtures";
import { Placeholder } from "./Placeholder";

describe("accent swatches", () => {
  it("is a keyboard-operable radiogroup with named swatches", async () => {
    renderApp(<Placeholder icon={SettingsIcon} />);
    const green = await screen.findByRole("radio", { name: "绿" });
    expect(green).toHaveAttribute("aria-checked", "true");
    expect(screen.getAllByRole("radio", { name: /^[绿蓝紫橙青]$/ })).toHaveLength(5);
    green.focus();
    await act(async () => {
      await userEvent.keyboard("{ArrowRight}");
    });
    expect(screen.getByRole("radio", { name: "蓝" })).toHaveAttribute("aria-checked", "true");
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, undefined, "blue");
    expect(screen.getByRole("radio", { name: "蓝" })).toHaveFocus();
  });

  it("selects on click and ArrowLeft wraps to the last swatch", async () => {
    renderApp(<Placeholder icon={SettingsIcon} />);
    const green = await screen.findByRole("radio", { name: "绿" });
    await act(async () => {
      await userEvent.click(screen.getByRole("radio", { name: "紫" }));
    });
    expect(updateUiPrefs).toHaveBeenLastCalledWith(undefined, undefined, "violet");
    green.focus();
    (updateUiPrefs as Mock).mockClear();
    await act(async () => {
      await userEvent.keyboard("{ArrowLeft}");
    });
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, undefined, "teal");
    expect(screen.getByRole("radio", { name: "青" })).toHaveFocus();
  });
});
