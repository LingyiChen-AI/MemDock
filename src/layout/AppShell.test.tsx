import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { Settings } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn(),
  updateUiPrefs: vi.fn(),
}));

import { getSettings, updateUiPrefs } from "@/api/memory";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "./AppShell";

const base: Settings = { language: null, theme: null, accent: null, manual_projects: [] };

function mockSettings(s: Partial<Settings>) {
  (getSettings as Mock).mockResolvedValue({ ...base, ...s });
  (updateUiPrefs as Mock).mockImplementation(async (language?: string, theme?: string, accent?: string) => ({
    ...base,
    ...s,
    language: language === undefined ? (s.language ?? null) : language === "system" ? null : language,
    theme: theme ?? s.theme ?? null,
    accent: accent ?? s.accent ?? null,
  }));
}

function setup(current: NavKey = "memory", onNavigate = vi.fn()) {
  render(
    <AppProviders>
      <AppShell current={current} onNavigate={onNavigate}>
        <div>memory-page</div>
      </AppShell>
    </AppProviders>,
  );
  return onNavigate;
}

function mockMatchMedia(matches: boolean) {
  const listeners = new Set<(e: { matches: boolean }) => void>();
  const mql = {
    matches,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_: string, cb: (e: { matches: boolean }) => void) => listeners.add(cb),
    removeEventListener: (_: string, cb: (e: { matches: boolean }) => void) => listeners.delete(cb),
  };
  window.matchMedia = vi.fn().mockReturnValue(mql) as unknown as typeof window.matchMedia;
  return {
    set(next: boolean) {
      mql.matches = next;
      listeners.forEach((cb) => cb({ matches: next }));
    },
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

beforeEach(() => {
  vi.clearAllMocks();
  document.documentElement.className = "";
  document.documentElement.removeAttribute("lang");
});

describe("AppShell", () => {
  it("renders grouped nav items in order (zh)", async () => {
    mockSettings({ language: "zh" });
    setup();
    await screen.findByText("记忆总览");
    const labels = screen.getAllByRole("button", { name: /^(记忆总览|记忆提取|待审核记忆|Skill 管理|会话转换|记忆同步|用量统计|环境与设置)$/ });
    expect(labels.map((b) => b.textContent)).toEqual([
      "记忆总览",
      "记忆提取",
      "待审核记忆",
      "Skill 管理",
      "会话转换",
      "记忆同步",
      "用量统计",
      "环境与设置",
    ]);
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
  });

  it("switches language to en", async () => {
    mockSettings({ language: "zh" });
    const user = userEvent.setup();
    setup("settings");
    await screen.findByText("即将推出");
    await user.click(screen.getByRole("radio", { name: "English" }));
    expect(updateUiPrefs).toHaveBeenCalledWith("en", undefined);
    expect(await screen.findByText("Memories")).toBeInTheDocument();
    expect(screen.getByText("Coming soon")).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.lang).toBe("en"));
  });

  it("clicking nav calls onNavigate", async () => {
    mockSettings({ language: "zh" });
    const user = userEvent.setup();
    const onNavigate = setup();
    await user.click(await screen.findByRole("button", { name: "待审核记忆" }));
    expect(onNavigate).toHaveBeenCalledWith("review");
  });

  it("non-memory pages show placeholder", async () => {
    mockSettings({ language: "zh" });
    setup("usage");
    expect(await screen.findByText("即将推出")).toBeInTheDocument();
    expect(screen.queryByText("memory-page")).not.toBeInTheDocument();
  });

  it("memory page renders its children, not the placeholder", async () => {
    mockSettings({ language: "zh" });
    setup("memory");
    expect(await screen.findByText("memory-page")).toBeInTheDocument();
    expect(screen.queryByText("即将推出")).not.toBeInTheDocument();
  });

  it("falls back to navigator language when settings.language is empty", async () => {
    mockSettings({ language: null });
    vi.spyOn(window.navigator, "language", "get").mockReturnValue("zh-TW");
    setup();
    expect(await screen.findByText("记忆总览")).toBeInTheDocument();
  });

  it("marks the current item with aria-current", async () => {
    mockSettings({ language: "zh" });
    setup("review");
    expect(await screen.findByRole("button", { name: "待审核记忆" })).toHaveAttribute("aria-current", "page");
  });
});

describe("ThemeProvider", () => {
  it("theme defaults to dark class on html", async () => {
    mockSettings({ theme: null });
    setup();
    await waitFor(() => expect(getSettings).toHaveBeenCalled());
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("explicit light removes dark class", async () => {
    mockSettings({ theme: "light" });
    setup();
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });

  it("system theme follows prefers-color-scheme", async () => {
    const mm = mockMatchMedia(false);
    mockSettings({ theme: "system" });
    setup();
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
    mm.set(true);
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(true));
    mm.set(false);
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });

  it("theme switch on placeholder calls updateUiPrefs", async () => {
    mockSettings({ language: "zh", theme: "dark" });
    const user = userEvent.setup();
    setup("skills");
    await user.click(await screen.findByRole("radio", { name: "亮色" }));
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, "light");
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });
});

describe("Appearance popover", () => {
  it("opens from the sidebar footer on the memory page and shows the three controls", async () => {
    mockSettings({ language: "zh", theme: "dark" });
    const user = userEvent.setup();
    setup("memory");
    await user.click(await screen.findByRole("button", { name: "外观 / Appearance" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("radiogroup", { name: "模式" })).toBeInTheDocument();
    expect(within(dialog).getByRole("radiogroup", { name: "主题色" })).toBeInTheDocument();
    expect(within(dialog).getByRole("radiogroup", { name: "语言" })).toBeInTheDocument();
  });

  it("choosing light calls updateUiPrefs and updates html", async () => {
    mockSettings({ language: "zh", theme: "dark" });
    const user = userEvent.setup();
    setup("memory");
    await user.click(await screen.findByRole("button", { name: "外观 / Appearance" }));
    await user.click(await screen.findByRole("radio", { name: "亮色" }));
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, "light");
    await waitFor(() => expect(document.documentElement.classList.contains("dark")).toBe(false));
  });

  it("choosing an accent calls updateUiPrefs and sets data-accent", async () => {
    mockSettings({ language: "zh", theme: "dark" });
    const user = userEvent.setup();
    setup("memory");
    await user.click(await screen.findByRole("button", { name: "外观 / Appearance" }));
    await user.click(await screen.findByRole("radio", { name: "紫" }));
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, undefined, "violet");
    await waitFor(() => expect(document.documentElement.dataset.accent).toBe("violet"));
  });

  it("is also reachable on other pages", async () => {
    mockSettings({ language: "zh" });
    setup("usage");
    expect(await screen.findByRole("button", { name: "外观 / Appearance" })).toBeInTheDocument();
  });
});

describe("AppearanceControls labels", () => {
  it("label column is wide enough for English (min-w-20)", async () => {
    mockSettings({ language: "en" });
    setup("usage");
    const label = await screen.findByText("Accent color");
    expect(label.className).toMatch(/min-w-20/);
  });
});
