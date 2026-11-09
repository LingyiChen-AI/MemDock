import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Update } from "@tauri-apps/plugin-updater";
import { renderApp } from "@/test/fixtures";
import { AppUpdateSection } from "./AppUpdateSection";

it("renders release headings, fixes and PR links in the update panel", async () => {
  const update = {
    version: "0.1.2",
    body: "## 更新与改进\n\n- **更新面板**\n\n## 问题修复\n\n- 修复链接显示\n\n[PR #42](https://github.com/example/project/pull/42)\n\n<!-- Release notes generated using configuration in .github/release.yml at v0.1.2 -->\n\n**Full Changelog**: https://github.com/example/project/compare/v0.1.1...v0.1.2",
  } as Update;
  renderApp(<AppUpdateSection state={{
    enabled: true,
    version: "0.1.1",
    update,
    status: "available",
    blocked: false,
    restartFailed: false,
    progress: undefined,
    checkNow: async () => {},
    install: async () => {},
    restart: async () => {},
  }} />);

  await userEvent.click(await screen.findByText("更新说明"));
  expect(screen.getByRole("heading", { name: "更新与改进" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "问题修复" })).toBeVisible();
  expect(screen.getAllByRole("listitem")).toHaveLength(2);
  expect(screen.queryByText(/Release notes generated|Full Changelog/)).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "PR #42" })).toHaveAttribute("href", "https://github.com/example/project/pull/42");
});

it.each([
  ["downloading", undefined, "正在下载更新…", undefined],
  ["downloading", 42, "正在下载更新…", 42],
  ["installing", undefined, "正在验证并安装更新…", 100],
] as const)("presents %s progress without guessing the download size", async (status, progress, label, percent) => {
  renderApp(<AppUpdateSection state={{
    enabled: true,
    version: "0.1.1",
    update: { version: "0.1.2" } as Update,
    status,
    blocked: false,
    restartFailed: false,
    progress,
    checkNow: async () => {},
    install: async () => {},
    restart: async () => {},
  }} />);
  const bar = await screen.findByRole("progressbar", { name: label });
  if (percent === undefined) {
    expect(bar).not.toHaveAttribute("aria-valuenow");
  } else {
    expect(bar).toHaveAttribute("aria-valuenow", String(percent));
    expect(screen.getByText(`${percent}%`)).toBeVisible();
  }
});
