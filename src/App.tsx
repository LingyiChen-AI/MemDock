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
import { usePendingSummary } from "@/pages/review/usePendingSummary";

function Shell() {
  const { t } = useI18n();
  const [current, setCurrent] = useState<NavKey>("memory");
  // The open page reports unsaved edits; leaving it (pages unmount on navigation) asks first.
  const [dirty, setDirty] = useState(false);
  const [pendingNav, setPendingNav] = useState<NavKey | null>(null);
  const pending = usePendingSummary();
  const onDirty = useCallback((d: boolean) => setDirty(d), []);

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
        pages={{
          review: <ReviewPage onNavigate={navigate} />,
          extract: <ExtractPage onNavigate={navigate} onDirtyChange={onDirty} />,
          skills: <SkillsPage onDirtyChange={onDirty} />,
          usage: <UsagePage />,
        }}
      >
        <MemoryPage onDirtyChange={onDirty} pendingByProject={pending?.by_project} />
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
