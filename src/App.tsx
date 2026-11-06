import { useCallback, useState } from "react";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "@/layout/AppShell";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { MemoryPage } from "@/pages/memory/MemoryPage";
import { ExtractPage } from "@/pages/extract/ExtractPage";
import { SkillsPage } from "@/pages/skills/SkillsPage";
import { ReviewPage } from "@/pages/review/ReviewPage";
import { UsagePage } from "@/pages/usage/UsagePage";
import { SessionsPage } from "@/pages/sessions/SessionsPage";
import { SettingsPage } from "@/pages/settings/SettingsPage";
import { usePendingSummary } from "@/pages/review/usePendingSummary";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { AccountSlot } from "@/components/account/AccountSlot";
import { LoginDialog } from "@/components/account/LoginDialog";
import { useSyncStatus } from "@/components/account/useSyncStatus";
import { SyncPage } from "@/pages/sync/SyncPage";

function Shell() {
  const { t } = useI18n();
  const [current, setCurrent] = useState<NavKey>("memory");
  // The open page reports unsaved edits; leaving it (pages unmount on navigation) asks first.
  const [dirty, setDirty] = useState(false);
  const [pendingNav, setPendingNav] = useState<NavKey | null>(null);
  const pending = usePendingSummary();
  const onDirty = useCallback((d: boolean) => setDirty(d), []);
  const [syncStatus, setSyncStatus] = useSyncStatus();
  const [loginOpen, setLoginOpen] = useState(false);
  const [openRequest, setOpenRequest] = useState<{ fileId: string; nonce: number } | null>(null);
  const toasts = useToasts();

  const navigate = (key: NavKey) => {
    if (key === current) return;
    if (dirty) setPendingNav(key);
    else setCurrent(key);
  };

  return (
    <>
      <AppShell
        current={current}
        onNavigate={navigate}
        badges={{ review: pending?.total }}
        status={<AccountSlot status={syncStatus} onLogin={() => setLoginOpen(true)} onOpenSync={() => navigate("sync")} />}
        pages={{
          review: <ReviewPage onNavigate={navigate} />,
          extract: <ExtractPage onNavigate={navigate} onDirtyChange={onDirty} />,
          skills: <SkillsPage onDirtyChange={onDirty} onNavigate={navigate} />,
          sync: (
            <SyncPage
              status={syncStatus}
              onStatus={setSyncStatus}
              onOpenLogin={() => setLoginOpen(true)}
              onDirtyChange={onDirty}
              onOpenMemoryFile={(fileId) => {
                setOpenRequest((prev) => ({ fileId, nonce: (prev?.nonce ?? 0) + 1 }));
                navigate("memory");
              }}
            />
          ),
          sessions: <SessionsPage />,
          usage: <UsagePage />,
          settings: <SettingsPage onNavigate={navigate} />,
        }}
      >
        <MemoryPage onDirtyChange={onDirty} pendingByProject={pending?.by_project} openRequest={openRequest} />
      </AppShell>
      <ConfirmDialog
        open={pendingNav !== null}
        title={t.shell.leaveTitle}
        body={t.shell.leaveBody}
        confirmLabel={t.shell.leaveConfirm}
        cancelLabel={t.shell.leaveKeep}
        destructive
        onConfirm={() => {
          if (pendingNav) {
            setDirty(false);
            setCurrent(pendingNav);
          }
          setPendingNav(null);
        }}
        onCancel={() => setPendingNav(null)}
      />
      <LoginDialog open={loginOpen} onOpenChange={setLoginOpen} status={syncStatus} onStatus={setSyncStatus} notify={toasts.push} />
      <ToastRegion toasts={toasts.toasts} onDismiss={toasts.dismiss} />
    </>
  );
}

export default function App() {
  return (
    <AppProviders>
      <Shell />
    </AppProviders>
  );
}
