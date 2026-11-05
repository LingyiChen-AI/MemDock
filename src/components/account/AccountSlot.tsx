import { CircleUserRound, LogIn } from "lucide-react";
import type { SyncStatus } from "@/api/types";
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";

export interface AccountSlotProps {
  status: SyncStatus | null;
  /** Logged out: opens the login dialog. */
  onLogin: () => void;
  /** Logged in: opens the sync page. */
  onOpenSync: () => void;
}

const BTN =
  "cursor-pointer gap-2.5 px-2.5 text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground motion-reduce:transition-none";

/** Sidebar footer entry: "sign in" when logged out, the GitHub login and pending count otherwise. */
export function AccountSlot({ status, onLogin, onOpenSync }: AccountSlotProps) {
  const { t } = useI18n();
  const a = t.account;
  const { state } = useSidebar();
  const collapsed = state === "collapsed";

  if (!status?.logged_in) {
    return (
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton type="button" aria-label={a.login} tooltip={a.login} onClick={onLogin} className={`h-8 ${BTN}`}>
            <LogIn aria-hidden="true" />
            {collapsed ? null : <span>{a.login}</span>}
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    );
  }

  const login = status.login ?? "GitHub";
  const pendingText = status.pending > 0 ? fmt(a.pending, { n: status.pending }) : null;
  const label = pendingText ? `${login} · ${pendingText}` : login;
  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          type="button"
          aria-label={`${a.openSync}: ${label}`}
          tooltip={label}
          onClick={onOpenSync}
          className={`relative h-auto min-h-8 py-1.5 ${BTN}`}
        >
          <CircleUserRound aria-hidden="true" />
          {collapsed ? (
            pendingText ? <span aria-hidden="true" className="absolute top-1 right-1 size-2 rounded-full bg-primary" /> : null
          ) : (
            <span className="flex min-w-0 flex-col leading-tight">
              <span className="truncate text-sm text-foreground">{login}</span>
              {pendingText ? <span className="truncate text-xs text-accent-text">{pendingText}</span> : null}
            </span>
          )}
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
