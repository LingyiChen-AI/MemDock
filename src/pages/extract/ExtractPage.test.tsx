import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { ExtractConfig, ExtractProgress, RunRecord } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
}));
vi.mock("@/api/extract", () => ({
  getExtractConfig: vi.fn(),
  setExtractConfig: vi.fn(),
  extractRunners: vi.fn(),
  startExtraction: vi.fn(),
  cancelExtraction: vi.fn(),
  listExtractRuns: vi.fn(),
  onExtractProgress: vi.fn(),
  onExtractFinished: vi.fn(),
  onSuggestionsChanged: vi.fn().mockResolvedValue(() => {}),
}));

import {
  cancelExtraction,
  extractRunners,
  getExtractConfig,
  listExtractRuns,
  onExtractFinished,
  onExtractProgress,
  setExtractConfig,
  startExtraction,
} from "@/api/extract";
import { listMemories } from "@/api/memory";
import { mkProject, renderApp } from "@/test/fixtures";
import { ExtractPage } from "./ExtractPage";

const config: ExtractConfig = {
  tool: "claude",
  schedule: { kind: "off" },
  projects: { kind: "all" },
  include_global: true,
  lookback_days: 7,
  lang: null,
};

const run = (over: Partial<RunRecord> = {}): RunRecord => ({
  id: "r1",
  started_at: new Date(2026, 10, 5, 9, 30).getTime(),
  finished_at: new Date(2026, 10, 5, 9, 41).getTime(),
  trigger: "manual",
  tool: "claude",
  status: "succeeded",
  sessions_read: 12,
  projects: 3,
  suggestions_new: 4,
  warnings: [],
  error: null,
  ...over,
});

let progressCb: (p: ExtractProgress) => void = () => {};
let finishedCb: (r: RunRecord) => void = () => {};

beforeEach(() => {
  vi.clearAllMocks();
  (getExtractConfig as Mock).mockResolvedValue(config);
  (setExtractConfig as Mock).mockImplementation((c: ExtractConfig) => Promise.resolve(c));
  (extractRunners as Mock).mockResolvedValue([
    { tool: "claude", available: true, program: "claude" },
    { tool: "codex", available: false },
  ]);
  (listExtractRuns as Mock).mockResolvedValue([]);
  (listMemories as Mock).mockResolvedValue({
    global: [],
    projects: [
      { project: mkProject({ id: "/work/a", name: "alpha" }), files: [] },
      { project: mkProject({ id: "/work/b", name: "beta" }), files: [] },
    ],
  });
  (onExtractProgress as Mock).mockImplementation(async (cb) => {
    progressCb = cb;
    return () => {};
  });
  (onExtractFinished as Mock).mockImplementation(async (cb) => {
    finishedCb = cb;
    return () => {};
  });
});

describe("ExtractPage", () => {
  it("unavailable runner disabled", async () => {
    renderApp(<ExtractPage />);
    const codex = await screen.findByRole("radio", { name: /Codex/ });
    expect(codex).toBeDisabled();
    expect(within(codex).getByText("未检测到")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Claude Code/ })).toBeEnabled();
    expect(
      screen.getByText("提取会调用你本机的 Claude Code / Codex，消耗你的订阅用量；会话内容在发送前会去除密钥。"),
    ).toBeInTheDocument();
  });

  it("schedule weekly saves", async () => {
    renderApp(<ExtractPage />);
    const save = await screen.findByRole("button", { name: "保存配置" });
    expect(save).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("定时"), "weekly");
    await userEvent.selectOptions(screen.getByLabelText("星期"), "1");
    fireEvent.change(screen.getByLabelText("时间"), { target: { value: "09:30" } });
    expect(save).toBeEnabled();
    await userEvent.click(save);
    await waitFor(() =>
      expect(setExtractConfig).toHaveBeenCalledWith({
        ...config,
        schedule: { kind: "weekly", weekday: 1, hh: 9, mm: 30 },
      }),
    );
    await waitFor(() => expect(save).toBeDisabled());
  });

  it("saves selected projects", async () => {
    renderApp(<ExtractPage />);
    await screen.findByRole("button", { name: "保存配置" });
    await userEvent.click(screen.getByRole("radio", { name: "选择项目" }));
    await userEvent.click(screen.getByRole("checkbox", { name: /beta/ }));
    await userEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() =>
      expect(setExtractConfig).toHaveBeenCalledWith({ ...config, projects: { kind: "selected", ids: ["/work/b"] } }),
    );
  });

  it("run now shows progress and cancel", async () => {
    (startExtraction as Mock).mockResolvedValue(run({ id: "r2", status: "running", finished_at: null }));
    renderApp(<ExtractPage />);
    await userEvent.click(await screen.findByRole("button", { name: "立即运行" }));
    await screen.findByText("运行中…");
    act(() => progressCb({ project: "alpha", index: 2, total: 5 }));
    expect(await screen.findByText("运行中… 第 2 / 5 个项目")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(cancelExtraction).toHaveBeenCalled();
    (listExtractRuns as Mock).mockResolvedValue([run({ id: "r2", status: "cancelled" })]);
    act(() => finishedCb(run({ id: "r2", status: "cancelled" })));
    expect(await screen.findByRole("button", { name: "立即运行" })).toBeInTheDocument();
    expect(await screen.findByText("已取消")).toBeInTheDocument();
  });

  it("shows running state when a run is already in progress on mount", async () => {
    (listExtractRuns as Mock).mockResolvedValue([run({ status: "running", finished_at: null })]);
    renderApp(<ExtractPage />);
    expect(await screen.findByText("运行中…")).toBeInTheDocument();
    expect(startExtraction).not.toHaveBeenCalled();
  });

  it("already running notice", async () => {
    (startExtraction as Mock).mockRejectedValue({ code: "already_running", message: "x" });
    renderApp(<ExtractPage />);
    await userEvent.click(await screen.findByRole("button", { name: "立即运行" }));
    expect(await screen.findByText("已有提取任务在运行")).toBeInTheDocument();
  });

  it("failed run expands error", async () => {
    (listExtractRuns as Mock).mockResolvedValue([
      run({ id: "f1", status: "failed", error: "claude exited with code 1", warnings: ["alpha: 超时"], suggestions_new: 0 }),
      run({ id: "ok" }),
    ]);
    renderApp(<ExtractPage />);
    expect(await screen.findByText("失败")).toBeInTheDocument();
    expect(screen.queryByText("claude exited with code 1")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "展开详情" }));
    expect(screen.getByText("claude exited with code 1")).toBeInTheDocument();
    expect(screen.getByText("alpha: 超时")).toBeInTheDocument();
  });

  it("new-suggestion count navigates to review and times are local", async () => {
    (listExtractRuns as Mock).mockResolvedValue([run()]);
    const onNavigate = vi.fn();
    renderApp(<ExtractPage onNavigate={onNavigate} />);
    expect(await screen.findByText("11-05 09:30")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "4" }));
    expect(onNavigate).toHaveBeenCalledWith("review");
  });

  it("empty runs state", async () => {
    renderApp(<ExtractPage />);
    expect(await screen.findByText("还没有运行记录")).toBeInTheDocument();
  });
});
