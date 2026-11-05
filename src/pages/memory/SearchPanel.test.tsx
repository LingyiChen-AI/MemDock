import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  searchMemories: vi.fn(),
}));

import { searchMemories } from "@/api/memory";
import { renderApp, sampleIndex, globalClaude, projClaude } from "@/test/fixtures";
import { SearchPanel } from "./SearchPanel";

const hits = [
  { file_id: globalClaude.id, line: 12, preview: "本地 MySQL 一主一从" },
  { file_id: globalClaude.id, line: 30, preview: "生产 mysql 读写分离" },
  { file_id: projClaude.id, line: 3, preview: "用 MySQL 8" },
];

beforeEach(() => {
  vi.clearAllMocks();
  (searchMemories as Mock).mockResolvedValue(hits);
});

describe("SearchPanel", () => {
  it("search debounces and groups by file", async () => {
    renderApp(<SearchPanel index={sampleIndex} onOpen={() => {}} />);
    await userEvent.type(screen.getByRole("searchbox"), "mysql");
    expect(searchMemories).not.toHaveBeenCalled();
    await waitFor(() => expect(searchMemories).toHaveBeenCalledTimes(1));
    expect(searchMemories).toHaveBeenCalledWith("mysql");

    const groups = await screen.findAllByRole("group");
    expect(groups).toHaveLength(2);
    // global file: tool tag + file name, two hits with line numbers
    expect(within(groups[0]).getByText("Claude Code")).toBeInTheDocument();
    expect(within(groups[0]).getByText("CLAUDE.md")).toBeInTheDocument();
    expect(within(groups[0]).getAllByRole("button")).toHaveLength(2);
    expect(within(groups[0]).getByText("12")).toBeInTheDocument();
    // project file: project name shown
    expect(within(groups[1]).getByText("jev-rpa")).toBeInTheDocument();
    // matches are highlighted case-insensitively
    const marks = [...document.querySelectorAll("mark")].map((m) => m.textContent);
    expect(marks).toEqual(["MySQL", "mysql", "MySQL"]);
  });

  it("clicking hit calls onOpen with line", async () => {
    const onOpen = vi.fn();
    renderApp(<SearchPanel index={sampleIndex} onOpen={onOpen} initialQuery="mysql" />);
    const buttons = await screen.findAllByRole("button", { name: /生产/ });
    await userEvent.click(buttons[0]);
    expect(onOpen).toHaveBeenCalledWith(globalClaude.id, 30);
  });

  it("shows an empty message when nothing matches", async () => {
    (searchMemories as Mock).mockResolvedValue([]);
    renderApp(<SearchPanel index={sampleIndex} onOpen={() => {}} initialQuery="zzz" />);
    expect(await screen.findByText("没有匹配的记忆")).toBeInTheDocument();
  });
});
