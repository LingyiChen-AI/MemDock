import * as React from "react";

const STORE_KEY = "__memdock_contexts__";

/**
 * A React context that survives Vite hot updates. Re-evaluating a provider
 * module would otherwise create a second context object; components that were
 * not re-rendered keep reading the old one, throw, and unmount the whole app.
 */
export function stableContext<T>(key: string, initial: T): React.Context<T> {
  const g = globalThis as unknown as Record<string, Record<string, React.Context<unknown>>>;
  const store = (g[STORE_KEY] ??= {});
  if (!store[key]) store[key] = React.createContext<unknown>(initial);
  return store[key] as React.Context<T>;
}
