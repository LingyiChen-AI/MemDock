import * as React from "react";

interface State {
  error: Error | null;
}

/**
 * Last-resort fallback so a render error shows a message and a reload button
 * instead of an empty window. Plain markup: providers (i18n, theme) may be
 * the thing that failed, so this cannot depend on them.
 */
export class RootErrorBoundary extends React.Component<{ children: React.ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("MemDock render error", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    const zh = /^zh/i.test(navigator.language);
    return (
      <div role="alert" className="flex h-screen flex-col items-center justify-center gap-4 bg-background p-8 text-foreground">
        <h1 className="text-lg font-semibold">{zh ? "界面出错了" : "Something went wrong"}</h1>
        <pre className="max-h-64 max-w-3xl overflow-auto rounded-md border border-border bg-muted px-4 py-3 font-mono text-xs whitespace-pre-wrap">
          {error.message}
        </pre>
        <button
          type="button"
          onClick={() => window.location.reload()}
          className="h-9 cursor-pointer rounded-md bg-primary px-4 text-sm font-semibold text-primary-foreground"
        >
          {zh ? "重新加载" : "Reload"}
        </button>
      </div>
    );
  }
}
