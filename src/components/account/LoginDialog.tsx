import * as React from "react";
import { Check, Copy, ExternalLink, Loader2, RotateCcw, TriangleAlert } from "lucide-react";
import {
  ackPublicRepo,
  loginCancel,
  loginStart,
  onLoginEvent,
  openVerificationUrl,
  setRepoName,
  syncStatus,
} from "@/api/sync";
import type { DeviceCodeInfo, LoginEvent, SyncStatus } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { FlowDialog, InlineError } from "@/pages/skills/flowParts";
import type { ToastKind } from "@/pages/skills/Toasts";
import { syncErrorText } from "./syncErrors";

export interface LoginDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: SyncStatus | null;
  onStatus: (s: SyncStatus) => void;
  notify: (kind: ToastKind, message: string) => void;
}

type Step =
  | { k: "intro" }
  | { k: "starting" }
  | { k: "code"; info: DeviceCodeInfo }
  | { k: "expired" }
  | { k: "denied" }
  | { k: "error"; message: string }
  | { k: "not_ours" }
  | { k: "public_ack" };

const REPO_NAME = /^[A-Za-z0-9._-]{1,100}$/;

function initialStep(status: SyncStatus | null): Step {
  if (status?.logged_in && status.needs === "not_ours") return { k: "not_ours" };
  if (status?.logged_in && status.needs === "public_ack") return { k: "public_ack" };
  return { k: "intro" };
}

const SMALL = "h-8 cursor-pointer px-3 text-sm";

/** GitHub device-flow sign-in plus the follow-up questions (name taken, public repository). */
export function LoginDialog({ open, onOpenChange, status, onStatus, notify }: LoginDialogProps) {
  const { t } = useI18n();
  const a = t.account;
  const [step, setStep] = React.useState<Step>(() => initialStep(status));
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [name, setName] = React.useState("");
  const [copied, setCopied] = React.useState(false);
  const stepRef = React.useRef(step);
  stepRef.current = step;
  const ctx = React.useRef({ t, onStatus, notify, onOpenChange });
  ctx.current = { t, onStatus, notify, onOpenChange };

  // Reset each time the dialog opens (status decides whether a follow-up question is pending).
  React.useEffect(() => {
    if (!open) return;
    setStep(initialStep(status));
    setError(null);
    setName("");
    setCopied(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const finish = React.useCallback((s: SyncStatus | null) => {
    const c = ctx.current;
    if (s) c.onStatus(s);
    c.notify("success", fmt(c.t.account.loggedIn, { login: s?.login ?? "" }));
    c.onOpenChange(false);
  }, []);

  /** After login or a follow-up answer: either ask the next question or close. */
  const proceed = React.useCallback(
    (s: SyncStatus | null, needs?: string | null) => {
      const n = needs ?? s?.needs;
      if (n === "not_ours") setStep({ k: "not_ours" });
      else if (n === "public_ack") setStep({ k: "public_ack" });
      else finish(s);
    },
    [finish],
  );

  React.useEffect(() => {
    if (!open) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    const handle = async (e: LoginEvent) => {
      if (!alive) return;
      switch (e.status) {
        case "ok": {
          const s = await syncStatus().catch(() => null);
          if (!alive) return;
          if (s) ctx.current.onStatus(s);
          proceed(s, e.needs);
          break;
        }
        case "expired":
          setStep({ k: "expired" });
          break;
        case "denied":
          setStep({ k: "denied" });
          break;
        case "cancelled":
          setStep({ k: "intro" });
          break;
        case "relogin":
          setStep({ k: "error", message: ctx.current.t.account.relogin });
          break;
        default:
          setStep({ k: "error", message: fmt(ctx.current.t.account.failed, { msg: e.message ?? "" }) });
      }
    };
    onLoginEvent((e) => void handle(e))
      .then((u) => (alive ? (unlisten = u) : u()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [open, proceed]);

  const start = async () => {
    setError(null);
    setStep({ k: "starting" });
    try {
      const info = await loginStart();
      setStep({ k: "code", info });
    } catch (e) {
      setStep({ k: "error", message: syncErrorText(t, e) });
    }
  };

  const close = () => {
    if (stepRef.current.k === "code" || stepRef.current.k === "starting") void loginCancel().catch(() => {});
    onOpenChange(false);
  };

  const copy = async (code: string) => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // clipboard unavailable: the code stays visible and selectable
    }
  };

  const submitName = async () => {
    const n = name.trim();
    if (!REPO_NAME.test(n)) {
      setError(a.nameInvalid);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const s = await setRepoName(n);
      onStatus(s);
      proceed(s);
    } catch (e) {
      setError(syncErrorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const ack = async () => {
    setBusy(true);
    setError(null);
    try {
      const s = await ackPublicRepo();
      onStatus(s);
      proceed(s);
    } catch (e) {
      setError(syncErrorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const repoName = status?.repo?.name ?? "MemDockHub";
  const repoFull = status?.repo ? `${status.repo.owner}/${status.repo.name}` : repoName;
  const relogin = status?.needs === "relogin";

  let body: React.ReactNode;
  switch (step.k) {
    case "intro":
    case "starting":
      body = (
        <>
          {relogin ? <InlineError message={a.relogin} /> : null}
          <p className="text-sm leading-relaxed">{a.loginIntro}</p>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={close} className={SMALL}>
              {a.cancel}
            </Button>
            <Button type="button" disabled={step.k === "starting"} onClick={() => void start()} className={`${SMALL} font-semibold`}>
              {step.k === "starting" ? <Loader2 className="size-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : null}
              {step.k === "starting" ? a.starting : a.start}
            </Button>
          </div>
        </>
      );
      break;
    case "code":
      body = (
        <>
          <p className="text-sm text-muted-foreground">{a.codeHint}</p>
          <div className="flex items-center justify-center gap-2 rounded-md border border-border bg-muted px-4 py-4">
            <span data-testid="user-code" className="font-mono text-3xl font-semibold tracking-[0.2em] select-all">
              {step.info.user_code}
            </span>
            <button
              type="button"
              aria-label={a.copy}
              title={copied ? a.copied : a.copy}
              onClick={() => void copy(step.info.user_code)}
              className="grid size-8 cursor-pointer place-items-center rounded-md text-muted-foreground transition-colors duration-150 outline-none hover:bg-background hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
            >
              {copied ? <Check className="size-4 text-accent-text" aria-hidden="true" /> : <Copy className="size-4" aria-hidden="true" />}
            </button>
          </div>
          <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin text-accent-text motion-reduce:animate-none" aria-hidden="true" />
            {a.waiting}
          </p>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={close} className={SMALL}>
              {a.cancel}
            </Button>
            <Button type="button" onClick={() => void openVerificationUrl().catch(() => {})} className={`${SMALL} font-semibold`}>
              <ExternalLink className="size-4" aria-hidden="true" />
              {a.openGithub}
            </Button>
          </div>
        </>
      );
      break;
    case "expired":
    case "denied":
    case "error":
      body = (
        <>
          <InlineError message={step.k === "expired" ? a.expired : step.k === "denied" ? a.denied : step.message} />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={close} className={SMALL}>
              {a.close}
            </Button>
            <Button type="button" onClick={() => void start()} className={`${SMALL} font-semibold`}>
              <RotateCcw className="size-4" aria-hidden="true" />
              {a.retry}
            </Button>
          </div>
        </>
      );
      break;
    case "not_ours":
      body = (
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            void submitName();
          }}
        >
          <p className="text-sm">{fmt(a.notOurs, { name: repoName })}</p>
          <label className="flex flex-col gap-1.5 text-sm">
            <span className="text-muted-foreground">{a.newName}</span>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="MemDockHub-2" className="font-mono" autoFocus />
          </label>
          {error ? <InlineError message={error} /> : null}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={close} className={SMALL}>
              {a.cancel}
            </Button>
            <Button type="submit" disabled={busy || !name.trim()} className={`${SMALL} font-semibold`}>
              {a.useName}
            </Button>
          </div>
        </form>
      );
      break;
    case "public_ack":
      body = (
        <>
          <div role="alert" className="flex items-start gap-2.5 rounded-md border border-destructive/40 bg-destructive/15 px-3 py-3 text-sm">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
            <div>
              <p className="font-semibold text-destructive">{a.publicTitle}</p>
              <p className="mt-1 text-foreground">{fmt(a.publicWarn, { name: repoFull })}</p>
            </div>
          </div>
          {error ? <InlineError message={error} /> : null}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={close} className={SMALL}>
              {a.cancel}
            </Button>
            <Button type="button" variant="destructive" disabled={busy} onClick={() => void ack()} className={`${SMALL} font-semibold`}>
              {a.publicAck}
            </Button>
          </div>
        </>
      );
      break;
  }

  return (
    <FlowDialog
      open={open}
      onOpenChange={(o) => (o ? onOpenChange(true) : close())}
      title={step.k === "not_ours" ? a.notOursTitle : a.loginTitle}
      description=""
      width={460}
    >
      <div className="mt-3 flex flex-col gap-4">{body}</div>
    </FlowDialog>
  );
}
