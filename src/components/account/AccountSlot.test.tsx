import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import type { SyncStatus } from "@/api/types";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
}));

import { SidebarProvider } from "@/components/ui/sidebar";
import { renderApp } from "@/test/fixtures";
import { AccountSlot } from "./AccountSlot";

const loggedIn = (pending: number): SyncStatus => ({
  logged_in: true,
  login: "octo",
  repo: { owner: "octo", name: "MemDockHub", url: "https://github.com/octo/MemDockHub", private: true },
  last_sync_at: null,
  pending,
  needs: null,
  remote_has_memory: false,
});

function setup(status: SyncStatus | null, open = true) {
  const onLogin = vi.fn();
  const onOpenSync = vi.fn();
  renderApp(
    <SidebarProvider defaultOpen={open}>
      <AccountSlot status={status} onLogin={onLogin} onOpenSync={onOpenSync} />
    </SidebarProvider>,
  );
  return { onLogin, onOpenSync };
}

describe("AccountSlot", () => {
  it("logged out offers sign in", async () => {
    const { onLogin } = setup(null);
    await userEvent.click(await screen.findByRole("button", { name: "登录 GitHub" }));
    expect(onLogin).toHaveBeenCalled();
  });

  it("account_slot_shows_pending_count", async () => {
    const { onOpenSync } = setup(loggedIn(3));
    expect(await screen.findByText("octo")).toBeInTheDocument();
    expect(screen.getByText("3 份待同步")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /octo/ }));
    expect(onOpenSync).toHaveBeenCalled();
  });

  it("hides the pending line when nothing is pending", async () => {
    setup(loggedIn(0));
    expect(await screen.findByText("octo")).toBeInTheDocument();
    expect(screen.queryByText(/份待同步/)).toBeNull();
  });

  it("collapsed_sidebar_icon_only", async () => {
    setup(loggedIn(2), false);
    const btn = await screen.findByRole("button", { name: /octo/ });
    expect(btn).toHaveAccessibleName("打开同步页: octo · 2 份待同步");
    expect(screen.queryByText("octo")).toBeNull();
    expect(screen.queryByText("2 份待同步")).toBeNull();
  });
});
