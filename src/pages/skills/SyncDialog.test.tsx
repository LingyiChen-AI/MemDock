import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({ syncSkill: vi.fn() }));

import { syncSkill } from "@/api/skills";
import { mkProject, renderApp } from "@/test/fixtures";
import { SyncDialog } from "./SyncDialog";
import { mkSkill } from "./testUtils";

const entry = mkSkill({ name: "alpha" });
const projects = [mkProject({ id: "c:/work/jev", name: "jev", path: "C:\\work\\jev" })];

function setup() {
  const props = { entry, open: true, onOpenChange: vi.fn(), projects, onSynced: vi.fn() };
  renderApp(<SyncDialog {...props} />);
  return props;
}

beforeEach(() => vi.clearAllMocks());

describe("SyncDialog", () => {
  it("sync dialog installs to other tool", async () => {
    (syncSkill as Mock).mockResolvedValue(mkSkill({ name: "alpha", tool: "codex" }));
    const props = setup();
    // the other tool is preselected
    expect(screen.getByRole("radio", { name: "Codex" })).toBeChecked();
    await userEvent.click(screen.getByRole("button", { name: "同步" }));
    await waitFor(() => expect(syncSkill).toHaveBeenCalledWith(entry.id, { tool: "codex", scope: { type: "global" } }, false));
    await waitFor(() => expect(props.onSynced).toHaveBeenCalled());
  });

  it("refuses the current location", async () => {
    setup();
    await userEvent.click(screen.getByRole("radio", { name: "Claude Code" }));
    expect(screen.getByText(/目标和当前位置相同/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "同步" })).toBeDisabled();
  });

  it("asks before overwriting an existing copy, then retries", async () => {
    (syncSkill as Mock)
      .mockRejectedValueOnce({ code: "exists", message: "exists", dir: "C:\\x" })
      .mockResolvedValueOnce(mkSkill({ name: "alpha", tool: "codex" }));
    setup();
    await userEvent.click(screen.getByRole("button", { name: "同步" }));
    expect(await screen.findByText(/已存在，覆盖（会先备份）？/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "覆盖" }));
    await waitFor(() => expect(syncSkill).toHaveBeenCalledTimes(2));
    expect((syncSkill as Mock).mock.calls[1][2]).toBe(true);
  });
});
