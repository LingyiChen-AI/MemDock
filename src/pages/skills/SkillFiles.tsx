import * as React from "react";
import { listSkillFiles, onSkillsChanged, readSkillFile, writeSkillFile } from "@/api/skills";
import type { SkillFileList } from "@/api/types";
import type { CodeEntry } from "@/api/workspaceCode";
import { useI18n } from "@/i18n/I18nProvider";
import { WorkspaceCodePanel, type CodePanelSource } from "@/pages/chat/WorkspaceCodePanel";
import type { CodeLocation } from "@/pages/chat/workspaceCodeLinks";

interface SkillFilesProps {
  skillId: string;
  cwd?: string;
  readonly: boolean;
  shown?: boolean;
  onDirtyChange?: (dirty: boolean) => void;
}

export function SkillFiles({ skillId, cwd = "", readonly, shown = true, onDirtyChange }: SkillFilesProps) {
  const { t: { skills } } = useI18n();
  const [request, setRequest] = React.useState<CodeLocation | null>(null);
  const [truncated, setTruncated] = React.useState(false);
  const listing = React.useMemo(() => {
    let pending: Promise<SkillFileList> | undefined;
    return {
      load() {
        if (!pending) {
          const request = listSkillFiles(skillId).catch((error) => {
            if (pending === request) pending = undefined;
            throw error;
          });
          pending = request;
        }
        return pending;
      },
      invalidate() { pending = undefined; },
    };
  }, [skillId]);

  React.useEffect(() => {
    let current = true;
    void listing.load().then(({ files }) => {
      const first = files.find((file) => file.rel_path === "SKILL.md") ?? files[0];
      if (current && first) setRequest({ path: first.rel_path });
    }, () => { /* The tree displays the listing error. */ });
    return () => { current = false; };
  }, [listing]);

  const source = React.useMemo<CodePanelSource>(() => ({
    async entries(path) {
      const { files, truncated } = await listing.load();
      setTruncated(truncated);
      const prefix = path ? path + "/" : "";
      const entries = new Map<string, CodeEntry>();
      for (const { rel_path } of files) {
        if (!rel_path.startsWith(prefix)) continue;
        const [name, ...rest] = rel_path.slice(prefix.length).split("/");
        if (name) entries.set(name, { name, path: prefix + name, directory: rest.length > 0 });
      }
      return [...entries.values()].sort((a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name));
    },
    async read(path) {
      const file = await readSkillFile(skillId, path);
      if (file.too_large || file.binary) throw new Error(file.too_large ? skills.tooLargeNotice : skills.binaryNotice);
      return { kind: "text", text: file.text, revision: file.hash, baseline: null };
    },
    save: readonly ? undefined : async (path, revision, text) => (await writeSkillFile(skillId, path, text, revision)).hash,
    async search(query) {
      const { files } = await listing.load();
      const needle = query.trim().toLowerCase();
      return files.filter((file) => file.rel_path.toLowerCase().includes(needle)).slice(0, 100)
        .map((file) => ({ path: file.rel_path, line: null, text: null }));
    },
    invalidate: listing.invalidate,
    watch: (refresh) => onSkillsChanged(refresh),
  }), [listing, readonly, skillId, skills.binaryNotice, skills.tooLargeNotice]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {truncated && <p className="border-b border-border px-3 py-1 text-xs text-muted-foreground">{skills.filesTruncated}</p>}
      <div className="flex min-h-0 min-w-0 flex-1">
        <WorkspaceCodePanel cwd={cwd} base={null} shown={shown} request={request} source={source} onDirtyChange={onDirtyChange} />
      </div>
    </div>
  );
}
