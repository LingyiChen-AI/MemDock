import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, type Mock } from "vitest";
import type { LoginEvent, SyncStatus } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));
vi.mock("@/api/sync", () => ({
  loginStart: vi.fn(),
  loginCancel: vi.fn().mockResolvedValue(undefined),
  openVerificationUrl: vi.fn().mockResolvedValue(undefined),
  setRepoName: vi.fn(),
  ackPublicRepo: vi.fn(),
  syncStatus: vi.fn(),
  onLoginEvent: vi.fn(),
}));

import { ackPublicRepo, loginStart, onLoginEvent, openVerificationUrl, setRepoName, syncStatus } from "@/api/sync";
import { renderApp } from "@/test/fixtures";
import { LoginDialog } from "./LoginDialog";

const status = (over: Partial<SyncStatus> = {}): SyncStatus => ({
  logged_in: true,
  login: "octo",
  repo: { owner: "octo", name: "MemDockHub", url: "https://github.com/octo/MemDockHub", private: true },
  last_sync_at: null,
  pending: 0,
  needs: null,
  remote_has_memory: false,
  ...over,
});

let emit: (e: LoginEvent) => void = () => {};

beforeEach(() => {
  vi.clearAllMocks();
  (onLoginEvent as Mock).mockImplementation((cb: (e: LoginEvent) => void) => {
    emit = cb;
    return Promise.resolve(() => {});
  });
  (loginStart as Mock).mockResolvedValue({ user_code: "ABCD-1234", verification_uri: "https://github.com/login/device", expires_in: 900 });
});

function setup(initial: SyncStatus | null = null) {
  const props = { onOpenChange: vi.fn(), onStatus: vi.fn(), notify: vi.fn() };
  renderApp(<LoginDialog open status={initial} {...props} />);
  return props;
}

async function startLogin() {
  await userEvent.click(await screen.findByRole("button", { name: "开始登录" }));
  await screen.findByText("ABCD-1234");
  await waitFor(() => expect(onLoginEvent).toHaveBeenCalled());
}

describe("LoginDialog", () => {
  it("shows_code_and_opens_github", async () => {
    setup();
    expect(await screen.findByText(/私有仓库 MemDockHub/)).toBeInTheDocument();
    await startLogin();
    await userEvent.click(screen.getByRole("button", { name: "打开 GitHub" }));
    expect(openVerificationUrl).toHaveBeenCalled();
    expect(screen.getByText("等待你在 GitHub 上授权…")).toBeInTheDocument();
  });

  it("ok_event_closes_and_toasts", async () => {
    (syncStatus as Mock).mockResolvedValue(status());
    const p = setup();
    await startLogin();
    await act(async () => emit({ status: "ok" }));
    await waitFor(() => expect(p.onOpenChange).toHaveBeenCalledWith(false));
    expect(p.notify).toHaveBeenCalledWith("success", "已登录 GitHub：octo");
    expect(p.onStatus).toHaveBeenCalled();
  });

  it("expired_offers_retry", async () => {
    setup();
    await startLogin();
    await act(async () => emit({ status: "expired" }));
    expect(await screen.findByText("验证码已过期")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重新获取验证码" }));
    expect(loginStart).toHaveBeenCalledTimes(2);
    await act(async () => emit({ status: "denied" }));
    expect(await screen.findByText("已在 GitHub 拒绝授权")).toBeInTheDocument();
  });

  it("not_ours_asks_new_name", async () => {
    (syncStatus as Mock).mockResolvedValue(status({ needs: "not_ours" }));
    (setRepoName as Mock).mockResolvedValue(status());
    const p = setup();
    await startLogin();
    await act(async () => emit({ status: "ok", needs: "not_ours" }));
    expect(await screen.findByText(/已有同名仓库 MemDockHub 但不是 MemDock 创建的/)).toBeInTheDocument();
    await userEvent.type(screen.getByRole("textbox"), "MyMemories");
    await userEvent.click(screen.getByRole("button", { name: "使用这个名字" }));
    expect(setRepoName).toHaveBeenCalledWith("MyMemories");
    await waitFor(() => expect(p.onOpenChange).toHaveBeenCalledWith(false));
  });

  it("public_requires_ack", async () => {
    (ackPublicRepo as Mock).mockResolvedValue(status({ repo: { owner: "octo", name: "MemDockHub", url: "u", private: false } }));
    const p = setup(status({ needs: "public_ack", repo: { owner: "octo", name: "MemDockHub", url: "u", private: false } }));
    expect(await screen.findByText(/仓库 octo\/MemDockHub 是公开的，记忆会被所有人看到/)).toBeInTheDocument();
    expect(p.onOpenChange).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "我了解，继续使用" }));
    expect(ackPublicRepo).toHaveBeenCalled();
    await waitFor(() => expect(p.onOpenChange).toHaveBeenCalledWith(false));
  });
});
