import { useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide-react";

export interface QuickReplyPayload {
  liveId: string;
  title: string;
  body: string;
  status: string;
  placeholder: string;
  replyLabel: string;
  failedLabel: string;
  closeLabel: string;
}

export function QuickReply({ notice }: { notice: QuickReplyPayload }) {
  const [text, setText] = useState("");
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [sending, setSending] = useState(false);
  const [sent, setSent] = useState(false);
  const [error, setError] = useState("");
  const close = () => getCurrentWindow().close();
  async function openConversation() {
    try {
      await invoke("open_reply_conversation");
      await close();
    } catch (err) {
      setError(String(err));
    }
  }

  useEffect(() => {
    if (hovered || focused || text || sending || error) return;
    const timer = window.setTimeout(() => void getCurrentWindow().close(), 20_000);
    return () => window.clearTimeout(timer);
  }, [hovered, focused, text, sending, error]);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const message = text.trim();
    if (!message || sending || sent) return;
    setSending(true);
    setError("");
    try {
      await invoke("live_send", { liveId: notice.liveId, text: message });
      setSent(true);
    } catch (err) {
      setError(String(err));
      return;
    } finally {
      setSending(false);
    }
    await close();
  }

  return (
    <main className="flex h-screen flex-col gap-2 overflow-hidden rounded-xl border border-border bg-card p-4 text-card-foreground"
      onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)}
      onKeyDown={(event) => { if (event.key === "Escape") void close(); }}>
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <img src="/memdock-logo.svg" alt="" className="size-5 rounded" />
        <span className="font-medium text-card-foreground">MemDock</span>
        <span className="truncate">{notice.status}</span>
        <button type="button" onClick={() => void close()} aria-label={notice.closeLabel}
          className="ml-auto grid size-6 shrink-0 place-items-center rounded hover:bg-muted focus-visible:outline-2">
          <X size={14} />
        </button>
      </div>
      <button type="button" onClick={() => void openConversation()}
        className="flex min-h-0 flex-1 cursor-pointer flex-col rounded text-left hover:bg-muted focus-visible:outline-2">
        <span className="w-full truncate text-sm font-semibold" title={notice.title}>{notice.title}</span>
        <span className="mt-1 line-clamp-4 break-words text-xs leading-4 text-muted-foreground">{notice.body}</span>
      </button>
      {error && <p role="alert" title={error} className="truncate text-xs text-destructive">{notice.failedLabel}: {error}</p>}
      <form className="flex gap-2" onSubmit={(event) => void submit(event)}>
        <input value={text} onChange={(event) => setText(event.target.value)} onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)} aria-label={notice.placeholder} placeholder={notice.placeholder}
          readOnly={sending || sent}
          className="h-8 min-w-0 flex-1 rounded-md border border-input bg-background px-2.5 text-sm outline-none focus:border-ring" />
        <button type="submit" disabled={!text.trim() || sending || sent}
          className="h-8 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground disabled:opacity-50">
          {notice.replyLabel}
        </button>
      </form>
    </main>
  );
}
