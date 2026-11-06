import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ImageOff } from "lucide-react";
import { cn } from "@/lib/utils";

/**
 * Markdown for chat bubbles: tighter than the memory preview. Links never
 * navigate the app webview and images are never loaded (no network egress).
 */
export function ChatMarkdown({ text }: { text: string }) {
  return (
    <div className="min-w-0 text-sm leading-relaxed [overflow-wrap:anywhere] [&>*:first-child]:mt-0 [&>*:last-child]:mb-0">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
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
          a: ({ children, href }) => (
            <span className="text-primary underline-offset-2 hover:underline" title={href}>
              {children}
            </span>
          ),
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
          code: ({ className, children }) => (
            <code className={cn("rounded border border-border/60 bg-muted px-1 py-px font-mono text-[12px] break-all", className)}>{children}</code>
          ),
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
