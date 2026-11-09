import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { createFeishuApp, listFeishuBots, listFeishuProjects, saveFeishuBot, setFeishuStartWithClient, startFeishuBot } from "@/api/feishu";
import { openUrl } from "@tauri-apps/plugin-opener";
import { renderApp } from "@/test/fixtures";
import { FeishuPage } from "./FeishuPage";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "en" }),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock("@/api/feishu", () => ({
  createFeishuApp: vi.fn(),
  cancelFeishuApp: vi.fn().mockResolvedValue(undefined),
  listFeishuBots: vi.fn(),
  listFeishuProjects: vi.fn(),
  saveFeishuBot: vi.fn(),
  removeFeishuBot: vi.fn(),
  startFeishuBot: vi.fn(),
  setFeishuStartWithClient: vi.fn().mockResolvedValue(undefined),
  stopFeishuBot: vi.fn(),
  testFeishuBot: vi.fn(),
}));

it("allows rebinding a deleted app without first stopping the connection", async () => {
  vi.mocked(listFeishuProjects).mockResolvedValue([]);
  vi.mocked(listFeishuBots).mockResolvedValue([{
    id: "deleted", name: "Old bot", appId: "cli_old", engine: "codex", projects: [],
    hasSecret: true, registeredCredentials: true, startWithClient: true,
    status: "deleted", error: "飞书应用已被删除",
  }]);
  renderApp(<FeishuPage />);
  expect(await screen.findByText("The Feishu app has been deleted")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Edit connection" }));
  const appId = screen.getByRole("textbox", { name: "App ID" });
  await userEvent.clear(appId);
  await userEvent.type(appId, "cli_new");
  expect(appId).toHaveValue("cli_new");
});

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
  ]) {
    await userEvent.type(within(form).getByLabelText(label), value);
  }
  await userEvent.click(within(form).getByRole("checkbox", { name: "Bind MemDock" }));
  await userEvent.click(within(form).getByRole("checkbox", { name: "Bind Docs" }));
  await userEvent.click(within(form).getByRole("checkbox", { name: "Modify files in MemDock" }));
  await userEvent.click(within(form).getByRole("button", { name: "Save connection" }));
  expect(await screen.findByText("Dev bot")).toBeInTheDocument();
  const startup = screen.getByRole("switch", { name: "Start Dev bot with MemDock" });
  expect(startup).not.toBeChecked();
  await userEvent.click(startup);
  expect(startup).toBeChecked();
  expect(setFeishuStartWithClient).toHaveBeenCalledWith("demo", true);
  expect(startFeishuBot).not.toHaveBeenCalled();
  expect(saveFeishuBot).toHaveBeenCalledWith(expect.objectContaining({
    projects: [
      { path: "/work/memdock", read: true, create: false, edit: true, delete: false },
      { path: "/work/docs", read: true, create: false, edit: false, delete: false },
    ],
  }));
});

it("opens the Feishu creation page and fills in the returned App ID", async () => {
  const url = "https://open.feishu.cn/page/cli?user_code=TEST";
  vi.mocked(listFeishuBots).mockResolvedValue([]);
  vi.mocked(listFeishuProjects).mockResolvedValue([{ name: "MemDock", path: "/work/memdock" }]);
  vi.mocked(saveFeishuBot).mockImplementationOnce(async (input) => ({ ...input, id: "new", hasSecret: true, status: "stopped", error: null }));
  vi.mocked(createFeishuApp).mockImplementation(async (_attempt, onUrl) => {
    onUrl(url);
    return { appId: "cli_new_bot", botName: "Feishu robot" };
  });
  renderApp(<FeishuPage />);
  await userEvent.click(await screen.findByRole("button", { name: "Add connection" }));
  const form = within(await screen.findByRole("dialog"));
  await userEvent.click(form.getByRole("button", { name: "Choose an app in Feishu" }));
  await form.findByText(/App ID and App Secret received/);
  expect(openUrl).toHaveBeenCalledWith(url);
  expect(form.getByLabelText("App ID")).toHaveValue("cli_new_bot");
  expect(form.queryByRole("button", { name: /Allowed (user|group)/i })).not.toBeInTheDocument();
  expect(form.getByLabelText("App ID")).toHaveAttribute("readonly");
  expect(form.getByLabelText("App Secret")).toHaveAttribute("readonly");
  expect(form.getByLabelText("App Secret")).toHaveValue("••••••••");
  const name = form.getByLabelText("Connection name");
  expect(name).toHaveValue("Feishu robot");
  await userEvent.clear(name);
  await userEvent.type(name, "New bot");
  await userEvent.click(form.getByRole("button", { name: "Save connection" }));
  expect(form.getByRole("alert")).toHaveTextContent("Bind at least one project.");
  await userEvent.click(form.getByRole("checkbox", { name: "Bind MemDock" }));
  await userEvent.click(form.getByRole("button", { name: "Save connection" }));
  expect(saveFeishuBot).toHaveBeenLastCalledWith(expect.objectContaining({
    appId: "cli_new_bot",
    name: "New bot",
    appSecret: undefined,
    registrationId: vi.mocked(createFeishuApp).mock.calls[0][0],
  }));
});
