import { useState } from "react";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "@/layout/AppShell";
import { MemoryPage } from "@/pages/memory/MemoryPage";
import { SkillsPage } from "@/pages/skills/SkillsPage";

export default function App() {
  const [current, setCurrent] = useState<NavKey>("memory");
  return (
    <AppProviders>
      <AppShell current={current} onNavigate={setCurrent} pages={{ skills: <SkillsPage /> }}>
        <MemoryPage />
      </AppShell>
    </AppProviders>
  );
}
