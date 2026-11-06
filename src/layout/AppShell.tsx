import * as React from "react";
import {
  BarChart3,
  BookOpen,
  ChevronDown,
  ListChecks,
  MessagesSquare,
  Package,
  Palette,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  Settings,
  Sparkles,
  SquarePen,
  type LucideIcon,
} from "lucide-react";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarInset,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  useSidebar,
} from "@/components/ui/sidebar";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { AppearanceControls } from "@/components/appearance/AppearanceControls";
import { openUrl } from "@tauri-apps/plugin-opener";
import { GithubMark } from "@/components/brand/GithubMark";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { Placeholder } from "@/pages/Placeholder";
import logoUrl from "@/assets/memdock-logo.svg";

export const PROJECT_URL = "https://github.com/LingyiChen-AI/MemDock";

export type NavKey = "chat" | "memory" | "review" | "extract" | "skills" | "sync" | "usage" | "settings";

type NavGroupKey = "work" | "memory" | "tools" | "data" | "system";

const NAV_GROUPS: { key: NavGroupKey; items: { key: NavKey; icon: LucideIcon }[] }[] = [
  {
    key: "work",
    items: [
      { key: "chat", icon: MessagesSquare },
    ],
  },
  {
    key: "memory",
    items: [
      { key: "memory", icon: BookOpen },
      { key: "extract", icon: Sparkles },
      { key: "review", icon: ListChecks },
    ],
  },
  {
    key: "tools",
    items: [{ key: "skills", icon: Package }],
  },
  {
    key: "data",
    items: [
      { key: "sync", icon: RefreshCw },
      { key: "usage", icon: BarChart3 },
    ],
  },
  { key: "system", items: [{ key: "settings", icon: Settings }] },
];

const NAV = NAV_GROUPS.flatMap((g) => g.items);

const COLLAPSED_KEY = "memdock.nav.collapsed";

function loadCollapsed(): NavGroupKey[] {
  try {
    const v = JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export interface AppShellProps {
  current: NavKey;
  onNavigate: (key: NavKey) => void;
  /** Count badges next to nav items (e.g. pending reviews). */
  badges?: Partial<Record<NavKey, number>>;
  /** Footer status area above the collapse toggle (plan 4 puts "pending sync" here). */
  status?: React.ReactNode;
  /** Content of the `memory` page. */
  children?: React.ReactNode;
  /** Content of the other pages that already exist; the rest render the placeholder. */
  pages?: Partial<Record<NavKey, React.ReactNode>>;
  /** The "New chat" button at the top of the sidebar; hidden when not set. */
  onNewChat?: () => void;
}

/** Prominent "New chat" entry above the nav groups; icon-only when the sidebar is collapsed. */
function NewChatButton({ onClick }: { onClick: () => void }) {
  const { t } = useI18n();
  const label = t.shell.newChat;
  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          type="button"
          tooltip={label}
          onClick={onClick}
          className="h-9 cursor-pointer justify-center gap-2 rounded-md border border-primary/30 bg-primary/10 px-2.5 font-semibold text-primary transition-colors duration-150 hover:bg-primary/16 hover:text-primary motion-reduce:transition-none group-data-[collapsible=icon]:justify-center"
        >
          <SquarePen aria-hidden="true" />
          <span>{label}</span>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function CollapseToggle() {
  const { t } = useI18n();
  const { state, toggleSidebar } = useSidebar();
  const collapsed = state === "collapsed";
  const label = collapsed ? t.shell.expand : t.shell.collapse;
  const Icon = collapsed ? PanelLeftOpen : PanelLeftClose;
  return (
    <SidebarMenuButton
      type="button"
      aria-label={label}
      tooltip={label}
      onClick={toggleSidebar}
      className="h-8 cursor-pointer gap-2.5 px-2.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
    >
      <Icon aria-hidden="true" />
      <span>{label}</span>
    </SidebarMenuButton>
  );
}

/** Opens the MemDock repository on GitHub in the default browser. */
function GithubButton() {
  const { t } = useI18n();
  const label = t.shell.github;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={() => void openUrl(PROJECT_URL).catch(() => {})}
          className="grid size-8 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground outline-none transition-colors duration-150 hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
        >
          <GithubMark className="size-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

function AppearanceButton() {
  const { t } = useI18n();
  const label = t.appearance.button;
  return (
    <Popover>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              aria-label={label}
              className="grid size-8 shrink-0 cursor-pointer place-items-center rounded-md text-muted-foreground outline-none transition-colors duration-150 hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
            >
              <Palette className="size-4" aria-hidden="true" />
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="right">{label}</TooltipContent>
      </Tooltip>
      <PopoverContent side="right" align="end" className="w-80">
        <p className="mb-3 text-sm font-semibold">{t.appearance.title}</p>
        <AppearanceControls />
      </PopoverContent>
    </Popover>
  );
}

export function AppShell({ current, onNavigate, badges, status, children, pages, onNewChat }: AppShellProps) {
  const { t } = useI18n();
  const page = NAV.find((n) => n.key === current)!;
  const [collapsed, setCollapsed] = React.useState<NavGroupKey[]>(loadCollapsed);
  const toggleGroup = (g: NavGroupKey) =>
    setCollapsed((prev) => {
      const next = prev.includes(g) ? prev.filter((x) => x !== g) : [...prev, g];
      try {
        localStorage.setItem(COLLAPSED_KEY, JSON.stringify(next));
      } catch {
        // storage unavailable: the fold just is not remembered
      }
      return next;
    });

  return (
    <SidebarProvider className="h-screen min-h-0">
      <Sidebar collapsible="icon" aria-label={t.shell.mainNav}>
        <SidebarHeader className="px-2 pt-3 pb-3.5">
          <div className="flex items-center gap-2 px-2 py-1 text-[15px] font-semibold group-data-[collapsible=icon]:px-0">
            <img src={logoUrl} alt="MemDock" width={24} height={24} className="size-6 shrink-0 group-data-[collapsible=icon]:mx-auto" />
            <span aria-hidden="true" className="group-data-[collapsible=icon]:hidden">MemDock</span>
          </div>
        </SidebarHeader>
        <SidebarContent className="px-2">
          {onNewChat ? (
            <div className="pb-1">
              <NewChatButton onClick={onNewChat} />
            </div>
          ) : null}
          {NAV_GROUPS.map((group) => {
            const folded = collapsed.includes(group.key) && !group.items.some((i) => i.key === current);
            return (
              <SidebarGroup key={group.key} className="px-0 py-1">
                <SidebarGroupLabel asChild className="h-7 px-2.5 text-[11px] font-medium tracking-wide text-muted-foreground/80 uppercase">
                  <button
                    type="button"
                    aria-expanded={!folded}
                    onClick={() => toggleGroup(group.key)}
                    className="flex w-full cursor-pointer items-center justify-between rounded-md outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    {t.navGroups[group.key]}
                    <ChevronDown className={cn("size-3.5 transition-transform duration-150 motion-reduce:transition-none", folded && "-rotate-90")} aria-hidden="true" />
                  </button>
                </SidebarGroupLabel>
                {folded ? null : (
                  <SidebarMenu>
                    {group.items.map(({ key, icon: Icon }) => {
                    const active = key === current;
                    const label = t.nav[key];
                    const count = badges?.[key];
                    return (
                      <SidebarMenuItem key={key}>
                        <SidebarMenuButton
                          type="button"
                          isActive={active}
                          aria-current={active ? "page" : undefined}
                          tooltip={label}
                          onClick={() => onNavigate(key)}
                          className={cn(
                            "h-8 cursor-pointer gap-2.5 px-2.5 text-muted-foreground transition-colors duration-150 hover:bg-muted hover:text-foreground motion-reduce:transition-none",
                            "data-active:bg-primary/12 data-active:font-semibold data-active:text-primary data-active:shadow-[inset_2px_0_0_var(--primary)] data-active:hover:bg-primary/16 data-active:hover:text-primary",
                          )}
                        >
                          <Icon aria-hidden="true" />
                          <span>{label}</span>
                        </SidebarMenuButton>
                        {count ? (
                          <SidebarMenuBadge className="top-2! h-4 min-w-4 rounded-full bg-primary px-1.5 text-xs font-semibold text-primary-foreground peer-data-active/menu-button:text-primary-foreground">
                            {count}
                          </SidebarMenuBadge>
                        ) : null}
                      </SidebarMenuItem>
                    );
                    })}
                  </SidebarMenu>
                )}
              </SidebarGroup>
            );
          })}
        </SidebarContent>
        <SidebarFooter className="gap-1 border-t border-sidebar-border px-2 pt-2.5">
          {status ? <div data-slot="sidebar-status">{status}</div> : null}
          <div className="flex items-center gap-1 group-data-[collapsible=icon]:flex-col-reverse group-data-[collapsible=icon]:items-center">
            <div className="min-w-0 flex-1 group-data-[collapsible=icon]:flex-none">
              <CollapseToggle />
            </div>
            <GithubButton />
            <AppearanceButton />
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-h-0 min-w-0 overflow-hidden">
        {current === "memory" ? children : (pages?.[current] ?? <Placeholder icon={page.icon} />)}
      </SidebarInset>
    </SidebarProvider>
  );
}
