import * as React from "react";
import { messageCodeLocation, WorkspaceCodeContext } from "@/pages/chat/workspaceCodeLinks";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import remarkGfm from "remark-gfm";
import { ImageOff } from "lucide-react";
import { cn } from "@/lib/utils";

/** Only web and mail links are opened outside the app. */
export function externalHref(href: string | undefined): string | null {
  if (!href) return null;
  try {
    const u = new URL(href);
    return ["http:", "https:", "mailto:"].includes(u.protocol) ? u.href : null;
  } catch {
    return null;
  }
}

/** A link that opens in the system's default browser on click; never inside the app window. */
export function ExternalLink({ href, children }: { href?: string; children: React.ReactNode }) {
  const { t } = useI18n();
  const url = externalHref(href);
  if (!url) return <span className="text-primary">{children}</span>;
  const open = (e: React.SyntheticEvent) => {
    e.preventDefault();
    void openUrl(url).catch((err) => console.error("memdock: open link failed", err));
  };
  return (
    <a
      href={url}
      title={fmt(t.shell.openLinkHint, { url })}
      onClick={open}
      onKeyDown={(e) => {
        if (e.key === "Enter") open(e);
      }}
      // No webview menu ("open in new window") and no middle-click navigation.
      onContextMenu={(e) => e.preventDefault()}
      onAuxClick={(e) => e.preventDefault()}
      className="cursor-pointer text-primary underline decoration-primary/40 underline-offset-2 hover:decoration-primary"
    >
      {children}
    </a>
  );
}

/**
 * Markdown for chat bubbles: tighter than the memory preview. Links never
 * navigate the app webview (Ctrl+click opens them in the browser) and images are never loaded (no network egress).
 */
/** Claude Code occasionally writes a broken character (U+FFFD) at the very start of a reply; it carries nothing. */
export function cleanReplyText(text: string): string {
  return text.replace(/^[\uFFFD\s]*\uFFFD\s*/, "");
}

export function ChatMarkdown({ text: raw }: { text: string }) {
  const openCode = React.useContext(WorkspaceCodeContext);
  const text = cleanReplyText(raw);
  if (!text.trim()) return null;
  return (
    <div className="min-w-0 text-sm leading-relaxed [overflow-wrap:anywhere] [&>*:first-child]:mt-0 [&>*:last-child]:mb-0">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={(url, key) => openCode && key === "href" && /^[a-z]:[\\/]/i.test(url) ? url : defaultUrlTransform(url)}
        components={{
          h1: ({ children }) => <h3 className="mt-3 mb-1.5 text-[15px] font-semibold">{children}</h3>,
          h2: ({ children }) => <h3 className="mt-3 mb-1.5 text-[15px] font-semibold">{children}</h3>,
          h3: ({ children }) => <h4 className="mt-3 mb-1 text-sm font-semibold">{children}</h4>,
          h4: ({ children }) => <h4 className="mt-2.5 mb-1 text-sm font-semibold">{children}</h4>,
          p: ({ children }) => <p className="my-2">{children}</p>,
          ul: ({ children }) => <ul className="my-2 ml-5 list-disc space-y-0.5">{children}</ul>,
          ol: ({ children }) => <ol className="my-2 ml-5 list-decimal space-y-0.5">{children}</ol>,
          li: ({ children }) => <li className="pl-0.5">{children}</li>,
          strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
          blockquote: ({ children }) => <blockquote className="my-2 border-l-[3px] border-border pl-3 text-muted-foreground">{children}</blockquote>,
          hr: () => <hr className="my-3 border-border" />,
          a: ({ children, href }) => {
            let path = href;
            if (href && !externalHref(href)) {
              try { path = decodeURI(href); } catch { /* Keep a literal percent in a file name. */ }
            }
            const location = openCode && path && !externalHref(href) && messageCodeLocation(path);
            return location
              ? <button type="button" className="text-primary underline" onClick={() => openCode?.(location)}>{children}</button>
              : <ExternalLink href={href}>{children}</ExternalLink>;
          },
          img: ({ alt, src }) => (
            <span className="inline-flex max-w-full items-center gap-1 align-middle text-xs text-muted-foreground">
              <ImageOff size={13} aria-hidden />
              {alt || (typeof src === "string" ? src : "")}
            </span>
          ),
          pre: ({ children }) => (
            <pre className="my-2 max-w-full overflow-x-auto rounded-md border border-border bg-muted/50 px-3 py-2 font-mono text-[12px] leading-5 [&_code]:border-0 [&_code]:bg-transparent [&_code]:p-0 [&_code]:break-normal">
              {children}
            </pre>
          ),
          code: ({ className, children }) => {
            const location = openCode && !className && typeof children === "string" && messageCodeLocation(children);
            const style = cn("rounded border border-border/60 bg-muted px-1 py-px font-mono text-[12px] break-all", className);
            return location
              ? <button type="button" className={cn(style, "text-primary underline")} onClick={() => openCode?.(location)}>{children}</button>
              : <code className={style}>{children}</code>;
          },
          table: ({ children }) => (
            <div className="my-2 max-w-full overflow-x-auto">
              <table className="border-collapse text-[13px]">{children}</table>
            </div>
          ),
          th: ({ children }) => <th className="border border-border bg-muted/50 px-2 py-1 text-left font-semibold">{children}</th>,
          td: ({ children }) => <td className="border border-border px-2 py-1">{children}</td>,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
