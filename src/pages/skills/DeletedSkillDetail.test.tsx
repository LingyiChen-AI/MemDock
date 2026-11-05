import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  restoreDeletedSkill: vi.fn(),
  revealBackupFolder: vi.fn(),
}));

import { restoreDeletedSkill, revealBackupFolder } from "@/api/skills";
import type { DeletedSkill } from "@/api/types";
import { renderApp } from "@/test/fixtures";
import { DeletedSkillDetail } from "./DeletedSkillDetail";

const item: DeletedSkill = {
  bucket: "pdf-notes-1a2b3c4d",
  name: "pdf-notes",
  tool: "claude",
  scope: { type: "global" },
  dir: "C:\\Users\\u\\.claude\\skills\\pdf-notes",
  backups: [
    { id: "1700000900000", created_at: 1_700_000_900_000, size_bytes: 4096 },
    { id: "1700000000000", created_at: 1_700_000_000_000, size_bytes: 2048 },
  ],
};

function setup() {
  const props = { item, projectName: (id: string) => id, onRestored: vi.fn() };
  renderApp(<DeletedSkillDetail {...props} />);
  return props;
}

beforeEach(() => {
  vi.clearAllMocks();
  (revealBackupFolder as Mock).mockResolvedValue("C:\\data\\skill-backups\\pdf-notes-1a2b3c4d");
});

describe("DeletedSkillDetail", () => {
  it("shows the original location, the backup folder and the backups", async () => {
    setup();
    expect(screen.getByRole("heading", { name: "pdf-notes" })).toBeInTheDocument();
    expect(screen.getByText("已删除")).toBeInTheDocument();
    expect(screen.getByText(item.dir)).toBeInTheDocument();
    expect(await screen.findByText("C:\\data\\skill-backups\\pdf-notes-1a2b3c4d")).toBeInTheDocument();
    expect(revealBackupFolder).toHaveBeenCalledWith(item.bucket);
    const rows = screen.getAllByRole("radio");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveAttribute("aria-checked", "true");
    expect(rows[0]).toHaveTextContent("4.0 KB");
    expect(screen.getByRole("button", { name: "复制路径" })).toBeEnabled();
  });

  it("restores the selected backup to the original location after a confirmation", async () => {
    (restoreDeletedSkill as Mock).mockResolvedValue(item.dir);
    const props = setup();
    await userEvent.click(screen.getAllByRole("radio")[1]);
    await userEvent.click(screen.getByRole("button", { name: "恢复到原位置" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByText(new RegExp(item.dir.replace(/\\/g, "\\\\")))).toBeInTheDocument();
    // nothing is restored before the confirmation
    expect(restoreDeletedSkill).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
    await waitFor(() => expect(restoreDeletedSkill).toHaveBeenCalledWith(item.bucket, "1700000000000"));
    await waitFor(() => expect(props.onRestored).toHaveBeenCalledWith(item, item.dir));
  });

  it("shows the reason when the restore is refused and stays open", async () => {
    (restoreDeletedSkill as Mock).mockRejectedValue({ code: "invalid", message: "原位置已经存在同名目录，不会覆盖" });
    const props = setup();
    await userEvent.click(screen.getByRole("button", { name: "恢复到原位置" }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("恢复失败：原位置已经存在同名目录，不会覆盖");
    expect(props.onRestored).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "恢复到原位置" })).toBeEnabled();
  });

  it("reports an unreadable backup folder", async () => {
    (revealBackupFolder as Mock).mockRejectedValue({ code: "invalid", message: "backup folder not found" });
    setup();
    expect(await screen.findByText(/读取备份位置失败：backup folder not found/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "复制路径" })).toBeNull();
  });
});
