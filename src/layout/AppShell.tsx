import * as React from "react";
import {
  BarChart3,
  BookOpen,
  ListChecks,
  Package,
  Palette,
  PanelLeftClose,
  PanelLeftOpen,
  Settings,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarInset,
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
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { Placeholder } from "@/pages/Placeholder";
import logoUrl from "@/assets/memdock-logo.svg";

export type NavKey = "memory" | "review" | "extract" | "skills" | "usage" | "settings";

const NAV: { key: NavKey; icon: LucideIcon }[] = [
  { key: "memory", icon: BookOpen },
  { key: "review", icon: ListChecks },
  { key: "extract", icon: Sparkles },
  { key: "skills", icon: Package },
  { key: "usage", icon: BarChart3 },
  { key: "settings", icon: Settings },
];

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

export function AppShell({ current, onNavigate, badges, status, children, pages }: AppShellProps) {
  const { t } = useI18n();
  const page = NAV.find((n) => n.key === current)!;

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
          <SidebarMenu>
            {NAV.map(({ key, icon: Icon }) => {
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
                      "data-active:bg-muted data-active:font-medium data-active:text-foreground data-active:shadow-[inset_2px_0_0_var(--primary)]",
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
        </SidebarContent>
        <SidebarFooter className="gap-1 border-t border-sidebar-border px-2 pt-2.5">
          {status ? <div data-slot="sidebar-status">{status}</div> : null}
          <div className="flex items-center gap-1 group-data-[collapsible=icon]:flex-col-reverse group-data-[collapsible=icon]:items-center">
            <div className="min-w-0 flex-1 group-data-[collapsible=icon]:flex-none">
              <CollapseToggle />
            </div>
            <AppearanceButton />
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-w-0 overflow-auto">
        {current === "memory" ? children : (pages?.[current] ?? <Placeholder icon={page.icon} />)}
      </SidebarInset>
    </SidebarProvider>
  );
}
