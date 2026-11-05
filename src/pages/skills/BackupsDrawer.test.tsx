import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  listSkillBackups: vi.fn(),
  rollbackSkill: vi.fn(),
}));

import { listSkillBackups, rollbackSkill } from "@/api/skills";
import { renderApp } from "@/test/fixtures";
import { BackupsDrawer } from "./BackupsDrawer";
import { mkSkill } from "./testUtils";

const entry = mkSkill({ name: "alpha" });
const backups = [
  { id: "1700000000000", created_at: 1_700_000_000_000, size_bytes: 2048 },
  { id: "1700000900000", created_at: 1_700_000_900_000, size_bytes: 4096 },
];

function setup() {
  const props = { entry, open: true, onOpenChange: vi.fn(), onRestored: vi.fn() };
  renderApp(<BackupsDrawer {...props} />);
  return props;
}

beforeEach(() => vi.clearAllMocks());

describe("BackupsDrawer", () => {
  it("backups drawer restores selected backup", async () => {
    (listSkillBackups as Mock).mockResolvedValue(backups);
    (rollbackSkill as Mock).mockResolvedValue(entry);
    const props = setup();
    const rows = await screen.findAllByRole("radio");
    expect(rows).toHaveLength(2);
    // newest first and selected by default
    expect(rows[0]).toHaveAttribute("aria-checked", "true");
    expect(rows[0]).toHaveTextContent("4.0 KB");
    await userEvent.click(rows[1]);
    await userEvent.click(screen.getByRole("button", { name: "恢复此备份" }));
    await waitFor(() => expect(rollbackSkill).toHaveBeenCalledWith(entry.id, "1700000000000"));
    await waitFor(() => expect(props.onRestored).toHaveBeenCalled());
    expect(props.onOpenChange).toHaveBeenCalledWith(false);
  });

  it("shows an empty state", async () => {
    (listSkillBackups as Mock).mockResolvedValue([]);
    setup();
    expect(await screen.findByText("还没有备份")).toBeInTheDocument();
  });

  it("keeps the drawer open and shows the restore error", async () => {
    (listSkillBackups as Mock).mockResolvedValue(backups);
    (rollbackSkill as Mock).mockRejectedValue({ code: "io", message: "disk full" });
    const props = setup();
    await userEvent.click(await screen.findByRole("button", { name: "恢复此备份" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
    expect(props.onRestored).not.toHaveBeenCalled();
  });
});
