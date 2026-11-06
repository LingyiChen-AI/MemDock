import { fireEvent, screen } from "@testing-library/react";
import { vi } from "vitest";

const openUrl = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (u: string) => openUrl(u) }));
vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
}));

import { renderApp } from "@/test/fixtures";
import { ChatMarkdown, externalHref } from "./ChatMarkdown";

describe("chat links", () => {
  beforeEach(() => openUrl.mockClear());

  it("only web and mail links are external", () => {
    expect(externalHref("https://example.com/a?b=1")).toBe("https://example.com/a?b=1");
    expect(externalHref("mailto:x@y.z")).toBe("mailto:x@y.z");
    expect(externalHref("file:///C:/x")).toBeNull();
    expect(externalHref("javascript:alert(1)")).toBeNull();
    expect(externalHref("relative/path")).toBeNull();
  });

  it("opens a bare URL in the browser on Ctrl+click only", async () => {
    renderApp(<ChatMarkdown text="see https://freecallai.feishu.cn/docx/abc for details" />);
    const link = await screen.findByRole("link", { name: "https://freecallai.feishu.cn/docx/abc" });
    fireEvent.click(link);
    expect(openUrl).not.toHaveBeenCalled();
    fireEvent.click(link, { ctrlKey: true });
    expect(openUrl).toHaveBeenCalledWith("https://freecallai.feishu.cn/docx/abc");
  });
});
