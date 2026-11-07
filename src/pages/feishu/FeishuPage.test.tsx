import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { listFeishuBots, listFeishuProjects, saveFeishuBot } from "@/api/feishu";
import { renderApp } from "@/test/fixtures";
import { FeishuPage } from "./FeishuPage";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "en" }),
}));

vi.mock("@/api/feishu", () => ({
  listFeishuBots: vi.fn(),
  listFeishuProjects: vi.fn(),
  saveFeishuBot: vi.fn(),
  removeFeishuBot: vi.fn(),
  startFeishuBot: vi.fn(),
  stopFeishuBot: vi.fn(),
  testFeishuBot: vi.fn(),
}));

it("creates one connection with independently scoped permissions for multiple projects", async () => {
  vi.mocked(listFeishuBots).mockResolvedValue([]);
  vi.mocked(listFeishuProjects).mockResolvedValue([
    { name: "MemDock", path: "/work/memdock" },
    { name: "Docs", path: "/work/docs" },
  ]);
  vi.mocked(saveFeishuBot).mockImplementation(async (input) => {
    const { appSecret: _, ...bot } = input;
    return { ...bot, id: "demo", hasSecret: true, status: "stopped", error: null };
  });
  renderApp(<FeishuPage />);
  await userEvent.click(await screen.findByRole("button", { name: "Add connection" }));
  const form = await screen.findByRole("dialog");
  for (const [label, value] of [
    ["Connection name", "Dev bot"], ["App ID", "cli_demo"], ["App Secret", "secret"],
    ["Tenant key", "tenant"], ["Allowed user Open IDs", "ou_owner"],
  ]) {
    await userEvent.type(within(form).getByLabelText(label), value);
  }
  await userEvent.click(within(form).getByRole("checkbox", { name: "Bind MemDock" }));
  await userEvent.click(within(form).getByRole("checkbox", { name: "Bind Docs" }));
  await userEvent.click(within(form).getByRole("checkbox", { name: "Modify files in MemDock" }));
  await userEvent.click(within(form).getByRole("button", { name: "Save connection" }));
  expect(await screen.findByText("Dev bot")).toBeInTheDocument();
  expect(saveFeishuBot).toHaveBeenCalledWith(expect.objectContaining({
    projects: [
      { path: "/work/memdock", read: true, create: false, edit: true, delete: false },
      { path: "/work/docs", read: true, create: false, edit: false, delete: false },
    ],
  }));
});
