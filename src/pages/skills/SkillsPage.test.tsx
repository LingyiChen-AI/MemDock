import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn().mockResolvedValue({ global: [], projects: [] }),
}));
vi.mock("@/api/skills", () => ({
  listSkills: vi.fn(),
  gitAvailable: vi.fn().mockResolvedValue(true),
  checkSkillUpdates: vi.fn(),
  onSkillsChanged: vi.fn().mockResolvedValue(() => {}),
  listSkillFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
  readSkillFile: vi.fn(),
  writeSkillFile: vi.fn(),
  registerSkillSource: vi.fn(),
}));

import { checkSkillUpdates, listSkills } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { SkillsPage } from "./SkillsPage";
import { mkSkill, mkUpdate } from "./testUtils";

const a = mkSkill({ name: "alpha" });
const b = mkSkill({ name: "beta" });

beforeEach(() => vi.clearAllMocks());

describe("SkillsPage", () => {
  it("shows the empty state with an add button", async () => {
    (listSkills as Mock).mockResolvedValue([]);
    const onAdd = vi.fn();
    renderApp(<SkillsPage onAdd={onAdd} />);
    expect(await screen.findByText("还没有 skill")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "添加 skill" }));
    expect(onAdd).toHaveBeenCalled();
  });

  it("does not check updates until the button is clicked, then merges results into rows", async () => {
    (listSkills as Mock).mockResolvedValue([a, b]);
    (checkSkillUpdates as Mock).mockResolvedValue([mkUpdate({ skill_id: b.id, has_update: true })]);
    renderApp(<SkillsPage />);
    await screen.findByRole("listbox");
    expect(checkSkillUpdates).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "检查更新" }));
    const row = within(screen.getByRole("listbox")).getAllByRole("option").find((o) => within(o).queryByText("beta"))!;
    expect(await within(row).findByText("有更新")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查更新（1）" })).toBeInTheDocument();
  });

  it("opens the first skill and switches on click", async () => {
    (listSkills as Mock).mockResolvedValue([a, b]);
    renderApp(<SkillsPage />);
    expect(await screen.findByRole("heading", { name: "alpha" })).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("listbox")).getByText("beta"));
    expect(await screen.findByRole("heading", { name: "beta" })).toBeInTheDocument();
  });
});
