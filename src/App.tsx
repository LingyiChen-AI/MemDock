import { useCallback, useEffect, useRef, useState } from "react";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "@/layout/AppShell";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { MemoryPage } from "@/pages/memory/MemoryPage";
import type { RunRecord } from "@/api/types";
import { ExtractPage } from "@/pages/extract/ExtractPage";
import { SkillsPage } from "@/pages/skills/SkillsPage";
import { ReviewPage } from "@/pages/review/ReviewPage";
import { UsagePage } from "@/pages/usage/UsagePage";
import { SessionsPage, type NewRequest } from "@/pages/sessions/SessionsPage";
import { FeishuPage } from "@/pages/feishu/FeishuPage";
import { SettingsPage } from "@/pages/settings/SettingsPage";
import { useAppUpdate } from "@/pages/settings/useAppUpdate";
import { fmt } from "@/pages/memory/paths";
import { usePendingSummary } from "@/pages/review/usePendingSummary";
import { ToastRegion, useToasts } from "@/pages/skills/Toasts";
import { AccountSlot } from "@/components/account/AccountSlot";
import { LoginDialog } from "@/components/account/LoginDialog";
import { useSyncStatus } from "@/components/account/useSyncStatus";
import { SyncPage } from "@/pages/sync/SyncPage";
import { initLiveStore, selectActiveCount, useLiveStore } from "@/pages/chat/liveStore";

function Shell() {
  const { t } = useI18n();
  const [current, setCurrent] = useState<NavKey>("chat");
  // The open page reports unsaved edits; leaving it (pages unmount on navigation) asks first.
  const [dirty, setDirty] = useState(false);
  const [pendingNav, setPendingNav] = useState<NavKey | null>(null);
  const pending = usePendingSummary();
  const onDirty = useCallback((d: boolean) => setDirty(d), []);
  const [syncStatus, setSyncStatus] = useSyncStatus();
  const [loginOpen, setLoginOpen] = useState(false);
  // The extraction run whose suggestions the review page is limited to.
  const [reviewRun, setReviewRun] = useState<RunRecord | null>(null);
  const [openRequest, setOpenRequest] = useState<{ fileId: string; nonce: number } | null>(null);
  const toasts = useToasts();
  // Live chats keep streaming (and the badge stays right) whichever page is open.
  useEffect(() => initLiveStore(), []);
  // Counts only: streaming tokens do not re-render the shell.
  const activeChats = useLiveStore(selectActiveCount);
  const appUpdate = useAppUpdate(dirty || activeChats > 0);
  const updateVersion = appUpdate.update?.version;
  const notifiedVersion = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (updateVersion && notifiedVersion.current !== updateVersion) {
      notifiedVersion.current = updateVersion;
      toasts.push("info", fmt(t.appUpdate.available, { version: updateVersion }));
    }
  }, [updateVersion, t.appUpdate.available, toasts.push]);
  // "New chat" / "new terminal" from the sidebar, handed to the chat page once it is open.
  const [newRequest, setNewRequest] = useState<NewRequest | null>(null);
  const requestHandled = useCallback(() => setNewRequest(null), []);

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
        badges={{ review: pending?.total, chat: activeChats }}
        onNewChat={() => {
          setNewRequest("chat");
          navigate("chat");
        }}
        onNewTerminal={() => {
          setNewRequest("terminal");
          navigate("chat");
        }}
        status={<AccountSlot status={syncStatus} onLogin={() => setLoginOpen(true)} onOpenSync={() => navigate("sync")} />}
        pages={{
          review: <ReviewPage onNavigate={navigate} runFilter={reviewRun} onClearRunFilter={() => setReviewRun(null)} />,
          extract: (
            <ExtractPage
              onNavigate={navigate}
              onDirtyChange={onDirty}
              onOpenRunReview={(run) => {
                setReviewRun(run);
                navigate("review");
              }}
            />
          ),
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
          chat: <SessionsPage request={newRequest} onRequestHandled={requestHandled} />,
          usage: <UsagePage />,
          feishu: <FeishuPage />,
          settings: <SettingsPage onNavigate={navigate} appUpdate={appUpdate} />,
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
        onCancel={() => {
          setPendingNav(null);
          setNewRequest(null);
        }}
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
