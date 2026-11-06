import * as React from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import { FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useI18n } from "@/i18n/I18nProvider";
import { fmt } from "@/pages/memory/paths";
import { loadNewChatDir, saveNewChatDir, type NewChatDirMode, type NewChatDirSetting } from "@/pages/chat/newChatDir";

/** Settings: which folder a new chat (sidebar / header / Ctrl+N) starts in. */
export function NewChatDirSettings() {
  const { t } = useI18n();
  const s = t.settings.newChatDir;
  const [value, setValue] = React.useState<NewChatDirSetting>(loadNewChatDir);
  const [home, setHome] = React.useState("~");
  React.useEffect(() => {
    void homeDir()
      .then((h) => setHome(h.replace(/[\\/]+$/, "")))
      .catch(() => {});
  }, []);
  const sep = home.includes("\\") ? "\\" : "/";
  const update = (v: NewChatDirSetting) => {
    setValue(v);
    saveNewChatDir(v);
  };
  const pick = async () => {
    const dir = await openDialog({ directory: true, multiple: false, defaultPath: value.fixed || undefined }).catch(() => null);
    if (typeof dir === "string" && dir) update({ mode: "fixed", fixed: dir });
  };
  const options: { mode: NewChatDirMode; title: string; hint: string }[] = [
    { mode: "follow", title: s.follow, hint: s.followHint },
    { mode: "memdock", title: s.memdock, hint: fmt(s.memdockHint, { path: `${home}${sep}memdock` }) },
    { mode: "fixed", title: s.fixed, hint: value.fixed || s.fixedNone },
  ];
  return (
    <RadioGroup value={value.mode} onValueChange={(m) => update({ ...value, mode: m as NewChatDirMode })} aria-label={s.title} className="gap-1">
      {options.map((o) => (
        <label
          key={o.mode}
          className="flex cursor-pointer items-start gap-3 rounded-md px-2 py-2 transition-colors hover:bg-muted/60 has-[[data-state=checked]]:bg-primary/6"
        >
          <RadioGroupItem value={o.mode} className="mt-0.5" />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-sm font-medium">{o.title}</span>
            <span className={o.mode === "follow" ? "text-xs text-muted-foreground" : "truncate font-mono text-[11.5px] text-muted-foreground"} title={o.hint}>
              {o.hint}
            </span>
          </span>
          {o.mode === "fixed" ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="shrink-0 cursor-pointer"
              onClick={(e) => {
                e.preventDefault();
                void pick();
              }}
            >
              <FolderOpen aria-hidden="true" />
              {value.fixed ? s.change : s.choose}
            </Button>
          ) : null}
        </label>
      ))}
    </RadioGroup>
  );
}
