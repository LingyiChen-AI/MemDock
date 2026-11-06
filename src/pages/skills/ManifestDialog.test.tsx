import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/skills", () => ({
  installSkill: vi.fn().mockResolvedValue([]),
  skillManifestCompare: vi.fn(),
  skillManifestExport: vi.fn(),
  skillManifestImport: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

import { open, save } from "@tauri-apps/plugin-dialog";
import { installSkill, skillManifestCompare, skillManifestExport, skillManifestImport } from "@/api/skills";
import type { ManifestCompare, ManifestSkill, MissingManifestSkill, SkillManifest } from "@/api/types";
import { renderApp } from "@/test/fixtures";
import { ManifestDialog } from "./ManifestDialog";

function mk(name: string, over: Partial<ManifestSkill> = {}): ManifestSkill {
  return {
    name,
    url: `https://github.com/o/${name}`,
    git_ref: null,
    commit: "c0ffee",
    skill_path: "",
    tool: "claude",
    scope: { type: "global" },
    installed_on: ["B"],
    removed_on: [],
    ...over,
  };
}

const missing = (name: string, over: Partial<MissingManifestSkill> = {}): MissingManifestSkill => ({
  ...mk(name),
  local_scope: { type: "global" },
  ...over,
});

function compareResult(over: Partial<ManifestCompare> = {}): ManifestCompare {
  return {
    local: { format: 1, skills: [] },
    remote: { format: 1, skills: [] },
    missing_here: [
      missing("pdf"),
      missing("docx"),
      missing("notes", { url: "", local_only: true }),
      missing("proj", { scope: { type: "project", key: "github.com/o/p" }, local_scope: null }),
    ],
    only_here: [mk("mine", { url: "", local_only: true })],
    in_both: [mk("shared"), mk("other")],
    local_only: [mk("mine", { url: "", local_only: true })],
    remote_updated_at: null,
    logged_in: true,
    ...over,
  };
}

function setup(props: Partial<React.ComponentProps<typeof ManifestDialog>> = {}) {
  const all = { open: true, onOpenChange: vi.fn(), notify: vi.fn(), onOpenSync: vi.fn(), onInstalled: vi.fn(), ...props };
  renderApp(<ManifestDialog {...all} />);
  return all;
}

beforeEach(() => {
  vi.clearAllMocks();
  (skillManifestCompare as Mock).mockResolvedValue(compareResult());
});

describe("ManifestDialog", () => {
  it("shows the three groups", async () => {
    setup();
    const missingSec = await screen.findByRole("region", { name: "云端有、本机缺少（4）" });
    expect(skillManifestCompare).toHaveBeenCalledWith(null);
    expect(within(missingSec).getByText("pdf")).toBeInTheDocument();
    expect(within(missingSec).getByText("需手动拷贝")).toBeInTheDocument();
    expect(within(missingSec).getByText("本机没有这个项目")).toBeInTheDocument();
    expect(within(missingSec).getByRole("checkbox", { name: "notes" })).toBeDisabled();
    expect(within(missingSec).getByRole("checkbox", { name: "proj" })).toBeDisabled();
    const onlyHere = screen.getByRole("region", { name: "本机有、云端没有（1）" });
    expect(within(onlyHere).getByText("mine")).toBeInTheDocument();
    const both = screen.getByRole("button", { name: "两边都有（2）" });
    expect(both).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("shared")).not.toBeInTheDocument();
    await userEvent.click(both);
    expect(screen.getByText("shared")).toBeInTheDocument();
    expect(screen.getByText("其中 1 个没有来源地址，只记录名字，需要手动拷贝")).toBeInTheDocument();
  });

  it("installs the checked skills after confirming", async () => {
    const props = setup();
    const sec = await screen.findByRole("region", { name: "云端有、本机缺少（4）" });
    const installBtn = within(sec).getByRole("button", { name: "安装选中（0）" });
    expect(installBtn).toBeDisabled();
    await userEvent.click(within(sec).getByRole("checkbox", { name: "pdf" }));
    await userEvent.click(within(sec).getByRole("button", { name: "安装选中（1）" }));
    expect(installSkill).not.toHaveBeenCalled();
    const dlg = await screen.findByRole("alertdialog");
    await userEvent.click(within(dlg).getByRole("button", { name: "安装" }));
    await waitFor(() =>
      expect(installSkill).toHaveBeenCalledWith(
        { type: "git", url: "https://github.com/o/pdf", clone_url: "https://github.com/o/pdf", git_ref: null, skill_path: "", commit: "c0ffee" },
        [{ tool: "claude", scope: { type: "global" } }],
        false,
      ),
    );
    expect(installSkill).toHaveBeenCalledTimes(1);
    expect(await within(sec).findByText("已安装")).toBeInTheDocument();
    expect(props.notify).toHaveBeenCalledWith("success", "安装完成：成功 1 个，失败 0 个");
    expect(props.onInstalled).toHaveBeenCalled();
  });

  it("select all picks only installable entries and reports failures per skill", async () => {
    (installSkill as Mock).mockResolvedValueOnce([]).mockRejectedValueOnce({ code: "git_failed", message: "boom" });
    const props = setup();
    const sec = await screen.findByRole("region", { name: "云端有、本机缺少（4）" });
    await userEvent.click(within(sec).getByRole("button", { name: "全选可安装的" }));
    await userEvent.click(within(sec).getByRole("button", { name: "安装选中（2）" }));
    await userEvent.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "安装" }));
    await waitFor(() => expect(installSkill).toHaveBeenCalledTimes(2));
    expect(await within(sec).findByText("失败：boom")).toBeInTheDocument();
    expect(props.notify).toHaveBeenCalledWith("error", "安装完成：成功 1 个，失败 1 个");
  });

  it("asks to sign in or import when there is no remote", async () => {
    (skillManifestCompare as Mock).mockResolvedValue(
      compareResult({ remote: null, logged_in: false, missing_here: [], in_both: [], only_here: [mk("mine")] }),
    );
    setup();
    expect(await screen.findByText(/还没登录 GitHub/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: /云端有、本机缺少/ })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "本机有、云端没有（1）" })).toBeInTheDocument();
  });

  it("goes to the sync page", async () => {
    const props = setup();
    await userEvent.click(await screen.findByRole("button", { name: "去同步" }));
    expect(props.onOpenSync).toHaveBeenCalled();
    expect(props.onOpenChange).toHaveBeenCalledWith(false);
  });

  it("exports the manifest file", async () => {
    (save as Mock).mockResolvedValue("C:\\tmp\\skills.json");
    (skillManifestExport as Mock).mockResolvedValue(3);
    const props = setup();
    await screen.findByRole("region", { name: "云端有、本机缺少（4）" });
    await userEvent.click(screen.getByRole("button", { name: "导出清单文件" }));
    await waitFor(() => expect(skillManifestExport).toHaveBeenCalledWith("C:\\tmp\\skills.json"));
    expect(props.notify).toHaveBeenCalledWith("success", "已导出 3 个 skill 到 C:\\tmp\\skills.json");
  });

  it("imports a manifest file and compares with it", async () => {
    const file: SkillManifest = { format: 1, skills: [mk("pdf")] };
    (open as Mock).mockResolvedValue("C:\\tmp\\other.json");
    (skillManifestImport as Mock).mockResolvedValue(file);
    setup();
    await screen.findByRole("region", { name: "云端有、本机缺少（4）" });
    (skillManifestCompare as Mock).mockResolvedValue(compareResult({ missing_here: [missing("pdf")], remote: file }));
    await userEvent.click(screen.getByRole("button", { name: "导入清单文件" }));
    await waitFor(() => expect(skillManifestImport).toHaveBeenCalledWith("C:\\tmp\\other.json"));
    await waitFor(() => expect(skillManifestCompare).toHaveBeenLastCalledWith(file));
    expect(await screen.findByText("对比对象：导入的文件 other.json")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "云端有、本机缺少（1）" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "改回对比云端" }));
    await waitFor(() => expect(skillManifestCompare).toHaveBeenLastCalledWith(null));
  });
});
