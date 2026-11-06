import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { PrefsFile, PrefsInfo, SyncStatus } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn(),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/extract", () => ({
  getExtractConfig: vi.fn(),
  setExtractConfig: vi.fn(),
}));
vi.mock("@/api/usage", () => ({
  setClaudeQuotaEnabled: vi.fn(),
  setCodexQuotaEnabled: vi.fn(),
}));
vi.mock("@/api/sync", () => ({
  syncStatus: vi.fn(),
  prefsSyncInfo: vi.fn(),
  prefsSetWithMemory: vi.fn(),
  prefsUpload: vi.fn(),
  prefsDownload: vi.fn(),
  prefsMarkApplied: vi.fn(),
}));

import { getExtractConfig, setExtractConfig } from "@/api/extract";
import { getSettings, updateUiPrefs } from "@/api/memory";
import { prefsDownload, prefsMarkApplied, prefsSetWithMemory, prefsSyncInfo, prefsUpload, syncStatus } from "@/api/sync";
import { setClaudeQuotaEnabled } from "@/api/usage";
import { renderApp } from "@/test/fixtures";
import { PrefsSyncSection } from "./PrefsSyncSection";

const status: SyncStatus = {
  logged_in: true,
  login: "octo",
  repo: { owner: "octo", name: "MemDockHub", url: "https://github.com/octo/MemDockHub", private: true },
  last_sync_at: 1,
  pending: 0,
  needs: null,
  remote_has_memory: false,
};

const info: PrefsInfo = {
  with_memory: true,
  remote: { updated_at: "2026-11-07T10:00:00Z", device: "OFFICE-PC", app_version: "0.1.0" },
  newer: true,
  seen: null,
};

const settings = { language: "zh", theme: "dark", accent: "green", manual_projects: [], claude_quota_enabled: false, codex_quota_enabled: false };
const extract = {
  tool: null,
  schedule: { kind: "off" },
  projects: { kind: "selected", ids: ["c:/codes/a"] },
  include_global: true,
  lookback_days: 7,
  lang: null,
  custom_prompt: null,
};

const notify = vi.fn();

function renderCard() {
  return renderApp(<PrefsSyncSection notify={notify} frame={(c) => <section aria-label="prefs-card">{c}</section>} />);
}

beforeEach(() => {
  localStorage.clear();
  notify.mockReset();
  (getSettings as Mock).mockResolvedValue(settings);
  (updateUiPrefs as Mock).mockImplementation((language?: string, theme?: string, accent?: string) =>
    Promise.resolve({ ...settings, ...(language ? { language } : {}), ...(theme ? { theme } : {}), ...(accent ? { accent } : {}) }),
  );
  (getExtractConfig as Mock).mockResolvedValue(extract);
  (setExtractConfig as Mock).mockImplementation((c: unknown) => Promise.resolve(c));
  (setClaudeQuotaEnabled as Mock).mockResolvedValue({ ...settings, claude_quota_enabled: true });
  (syncStatus as Mock).mockResolvedValue(status);
  (prefsSyncInfo as Mock).mockResolvedValue(info);
  (prefsMarkApplied as Mock).mockResolvedValue({ ...info, newer: false, seen: info.remote!.updated_at });
});

describe("PrefsSyncSection", () => {
  it("is hidden until GitHub sync is set up", async () => {
    (syncStatus as Mock).mockResolvedValue({ ...status, logged_in: false });
    renderCard();
    await waitFor(() => expect(syncStatus).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "上传到云端" })).toBeNull();
  });

  it("shows the cloud copy and uploads only the portable localStorage part", async () => {
    localStorage.setItem(
      "memdock.chat.prefs",
      JSON.stringify({ agent: "codex", permission: { claude: "plan", codex: "ask" }, cwd: "C:\\codes\\private" }),
    );
    localStorage.setItem("memdock.chat.newChatDir", JSON.stringify({ mode: "fixed", fixed: "D:\\work" }));
    localStorage.setItem("memdock.diffLayout", "unified");
    localStorage.setItem("memdock.nav.collapsed", JSON.stringify(["tools"]));
    localStorage.setItem("memdock.hiddenProjects", JSON.stringify(["c:/codes/a"]));
    localStorage.setItem("memdock.treeWidth", "300");
    (prefsUpload as Mock).mockResolvedValue({ committed: true, info: { ...info, newer: false } });
    renderCard();
    expect(await screen.findByText(/来自 OFFICE-PC/)).toBeInTheDocument();
    expect(screen.getByText("云端有更新")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "上传到云端" }));
    await waitFor(() => expect(prefsUpload).toHaveBeenCalledTimes(1));
    expect((prefsUpload as Mock).mock.calls[0][0]).toEqual({
      "memdock.chat.prefs": { agent: "codex", permission: { claude: "plan", codex: "ask" } },
      "memdock.chat.newChatDir": { mode: "fixed" },
      "memdock.diffLayout": "unified",
      "memdock.nav.collapsed": ["tools"],
    });
    await waitFor(() => expect(notify).toHaveBeenCalledWith("success", "偏好已上传到云端"));
    expect(screen.queryByText("云端有更新")).toBeNull();
  });

  it("lists what will change, then applies it", async () => {
    localStorage.setItem("memdock.diffLayout", "split");
    localStorage.setItem("memdock.chat.prefs", JSON.stringify({ agent: "claude", cwd: "C:/codes/here" }));
    const file: PrefsFile = {
      version: 1,
      updated_at: "2026-11-07T10:00:00Z",
      device: "OFFICE-PC",
      app_version: "0.1.0",
      backend: {
        language: "zh",
        theme: "light",
        accent: "green",
        claude_quota_enabled: true,
        codex_quota_enabled: false,
        extract: { tool: null, schedule: { kind: "off" }, include_global: true, lookback_days: 14, lang: null, custom_prompt: null },
      },
      frontend: {
        "memdock.diffLayout": "unified",
        "memdock.chat.prefs": { agent: "codex" },
        "memdock.chat.newChatDir": { mode: "fixed" },
      },
    };
    (prefsDownload as Mock).mockResolvedValue(file);
    renderCard();
    await userEvent.click(await screen.findByRole("button", { name: "从云端获取" }));
    const dialog = await screen.findByRole("alertdialog");
    const rows = within(dialog).getAllByRole("row").map((r) => r.textContent);
    expect(rows).toContain("外观模式darklight");
    expect(rows).toContain("Claude Code 额度查询关开");
    expect(rows).toContain("提取：回看天数714");
    expect(rows).toContain("差异视图布局splitunified");
    expect(rows).toContain("对话偏好 · agentclaudecodex");
    // a fixed folder without a local one falls back to "follow": nothing changes, no path shown
    expect(rows.some((r) => r?.includes("新对话的默认目录"))).toBe(false);
    expect(rows.some((r) => r?.includes("codes"))).toBe(false);

    await userEvent.click(within(dialog).getByRole("button", { name: "应用" }));
    await waitFor(() => expect(prefsMarkApplied).toHaveBeenCalledWith("2026-11-07T10:00:00Z"));
    expect(updateUiPrefs).toHaveBeenCalledWith(undefined, "light");
    expect(setClaudeQuotaEnabled).toHaveBeenCalledWith(true);
    expect(setExtractConfig).toHaveBeenCalledWith({ ...extract, lookback_days: 14 });
    expect(localStorage.getItem("memdock.diffLayout")).toBe("unified");
    expect(JSON.parse(localStorage.getItem("memdock.chat.prefs")!)).toEqual({ agent: "codex", cwd: "C:/codes/here" });
    // resolves to the default ("follow"), so nothing is stored
    expect(localStorage.getItem("memdock.chat.newChatDir")).toBeNull();
    expect(await screen.findByText("部分偏好需要重新加载界面后生效")).toBeInTheDocument();
    expect(notify).toHaveBeenCalledWith("success", "已应用云端偏好");
  });

  it("cancel applies nothing; identical preferences need no dialog", async () => {
    const file: PrefsFile = {
      version: 1,
      updated_at: "2026-11-07T10:00:00Z",
      device: "OFFICE-PC",
      app_version: "0.1.0",
      backend: { ...settings, extract: null },
      frontend: { "memdock.diffLayout": "unified" },
    };
    (prefsDownload as Mock).mockResolvedValue(file);
    renderCard();
    await userEvent.click(await screen.findByRole("button", { name: "从云端获取" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(localStorage.getItem("memdock.diffLayout")).toBeNull();
    expect(prefsMarkApplied).not.toHaveBeenCalled();

    localStorage.setItem("memdock.diffLayout", "unified");
    await userEvent.click(screen.getByRole("button", { name: "从云端获取" }));
    await waitFor(() => expect(notify).toHaveBeenCalledWith("info", "本机偏好已与云端一致"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("reports a missing cloud copy and toggles the memory switch", async () => {
    (prefsDownload as Mock).mockRejectedValue({ code: "prefs_missing", message: "none" });
    (prefsSetWithMemory as Mock).mockResolvedValue({ ...info, with_memory: false });
    renderCard();
    await userEvent.click(await screen.findByRole("button", { name: "从云端获取" }));
    await waitFor(() => expect(notify).toHaveBeenCalledWith("info", "云端还没有偏好设置，先在另一台电脑上传"));
    const sw = screen.getByRole("switch", { name: "上传记忆时一起上传偏好" });
    expect(sw).toHaveAttribute("aria-checked", "true");
    await userEvent.click(sw);
    await waitFor(() => expect(prefsSetWithMemory).toHaveBeenCalledWith(false));
    await waitFor(() => expect(sw).toHaveAttribute("aria-checked", "false"));
  });
});
