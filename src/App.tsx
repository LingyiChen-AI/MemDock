import { useState } from "react";
import { AppProviders } from "@/providers";
import { AppShell, type NavKey } from "@/layout/AppShell";

export default function App() {
  const [current, setCurrent] = useState<NavKey>("memory");
  return (
    <AppProviders>
      <AppShell current={current} onNavigate={setCurrent}>
        {/* Memory page lands in tasks 11-12. */}
        <main className="p-5" />
      </AppShell>
    </AppProviders>
  );
}
