import { useState } from "react";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "@/layout/AppShell";
import { MemoryPage } from "@/pages/memory/MemoryPage";

export default function App() {
  const [current, setCurrent] = useState<NavKey>("memory");
  return (
    <AppProviders>
      <AppShell current={current} onNavigate={setCurrent}>
        <MemoryPage />
      </AppShell>
    </AppProviders>
  );
}
