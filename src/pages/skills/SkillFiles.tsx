import * as React from "react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { EditorView } from "@codemirror/view";
import { ChevronDown, ChevronRight, File, Folder, Info, Save, TriangleAlert } from "lucide-react";
import { listSkillFiles, onSkillsChanged, readSkillFile, writeSkillFile } from "@/api/skills";
import type { SkillFileDto, SkillFileInfo } from "@/api/types";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/I18nProvider";
import { ConfirmDialog } from "@/pages/memory/ConfirmDialog";
import { memdockCmTheme } from "@/pages/memory/cmTheme";
import { fmt } from "@/pages/memory/paths";
import { asCommandError, errorMessage } from "./skillUtils";

export interface SkillFilesProps {
  skillId: string;
  readonly: boolean;
  /** Reports unsaved edits so the page can confirm before leaving. */
  onDirtyChange?: (dirty: boolean) => void;
}

interface TreeDir {
  kind: "dir";
  path: string;
  name: string;
  children: TreeNode[];
}
interface TreeFile {
  kind: "file";
  path: string;
  name: string;
}
type TreeNode = TreeDir | TreeFile;

function buildTree(files: SkillFileInfo[]): TreeNode[] {
  const root: TreeDir = { kind: "dir", path: "", name: "", children: [] };
  for (const f of files) {
    const parts = f.rel_path.split("/");
    let cur = root;
    parts.forEach((part, i) => {
      const path = parts.slice(0, i + 1).join("/");
      if (i === parts.length - 1) {
        cur.children.push({ kind: "file", path, name: part });
        return;
      }
      let dir = cur.children.find((c): c is TreeDir => c.kind === "dir" && c.name === part);
      if (!dir) {
        dir = { kind: "dir", path, name: part, children: [] };
        cur.children.push(dir);
      }
      cur = dir;
    });
  }
  const sort = (nodes: TreeNode[]) => {
    nodes.sort((a, b) => {
      // SKILL.md first, then folders, then files.
      if (a.path === "SKILL.md") return -1;
      if (b.path === "SKILL.md") return 1;
      if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    nodes.forEach((n) => n.kind === "dir" && sort(n.children));
  };
  sort(root.children);
  return root.children;
}

export function SkillFiles({ skillId, readonly, onDirtyChange }: SkillFilesProps) {
  const { t } = useI18n();
  const s = t.skills;
  const [list, setList] = React.useState<{ files: SkillFileInfo[]; truncated: boolean } | null>(null);
  const [listError, setListError] = React.useState<string | null>(null);
  const [relPath, setRelPath] = React.useState<string | undefined>();
  const [collapsed, setCollapsed] = React.useState<Set<string>>(new Set());
  const [base, setBase] = React.useState<SkillFileDto | null>(null);
  const [text, setText] = React.useState("");
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [external, setExternal] = React.useState(false);
  const [conflict, setConflict] = React.useState(false);
  const [saving, setSaving] = React.useState(false);
  const [pendingPath, setPendingPath] = React.useState<string | null>(null);

  const noText = base !== null && (base.binary || base.too_large);
  const dirty = base !== null && !noText && text !== base.text;
  const live = React.useRef({ relPath, dirty, hash: base?.hash ?? "" });
  live.current = { relPath, dirty, hash: base?.hash ?? "" };
  const loadSeq = React.useRef(0);

  const apply = React.useCallback((dto: SkillFileDto) => {
    setBase(dto);
    setText(dto.text);
    setExternal(false);
    setConflict(false);
    setSaveError(null);
  }, []);

  const loadFile = React.useCallback(
    async (path: string) => {
      const seq = ++loadSeq.current;
      setLoadError(null);
      try {
        const dto = await readSkillFile(skillId, path);
        if (seq === loadSeq.current) apply(dto);
      } catch (e) {
        if (seq === loadSeq.current) setLoadError(errorMessage(e));
      }
    },
    [skillId, apply],
  );

  const refreshList = React.useCallback(async () => {
    try {
      const l = await listSkillFiles(skillId);
      setList({ files: l.files, truncated: l.truncated });
      setListError(null);
      return l;
    } catch (e) {
      setListError(errorMessage(e));
      return null;
    }
  }, [skillId]);

  // Initial listing; SKILL.md (or the first file) opens by default.
  React.useEffect(() => {
    let alive = true;
    setList(null);
    setBase(null);
    setRelPath(undefined);
    void refreshList().then((l) => {
      if (!alive || !l) return;
      const first = l.files.find((f) => f.rel_path === "SKILL.md") ?? l.files[0];
      if (first) setRelPath(first.rel_path);
    });
    return () => {
      alive = false;
      loadSeq.current++;
    };
  }, [refreshList]);

  React.useEffect(() => {
    setBase(null);
    setText("");
    if (relPath) void loadFile(relPath);
  }, [relPath, loadFile]);

  React.useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  React.useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  // Watcher: refresh the listing and pick up external edits of the open file.
  React.useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    onSkillsChanged(() => {
      void refreshList();
      const cur = live.current;
      if (!cur.relPath) return;
      const seq = loadSeq.current;
      readSkillFile(skillId, cur.relPath).then(
        (dto) => {
          if (seq !== loadSeq.current || dto.hash === live.current.hash) return;
          if (live.current.dirty) setExternal(true);
          else apply(dto);
        },
        () => {
          // Deleted or unreadable: keep what the editor shows.
        },
      );
    }).then((u) => {
      if (alive) unlisten = u;
      else u();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [skillId, refreshList, apply]);

  const save = React.useCallback(async () => {
    if (!base || !relPath || saving || readonly) return;
    const sent = text;
    setSaving(true);
    setSaveError(null);
    try {
      const dto = await writeSkillFile(skillId, relPath, sent, base.hash);
      setBase(dto);
      setText((cur) => (cur === sent ? dto.text : cur));
      setExternal(false);
    } catch (e) {
      if (asCommandError(e)?.code === "disk_changed") {
        setConflict(true);
        setExternal(true);
      } else {
        setSaveError(fmt(s.saveFailed, { message: errorMessage(e) }));
      }
    } finally {
      setSaving(false);
    }
  }, [base, relPath, saving, readonly, text, skillId, s]);

  const reload = () => {
    setConflict(false);
    if (relPath) void loadFile(relPath);
  };

  const open = (path: string) => {
    if (path === relPath) return;
    if (dirty) setPendingPath(path);
    else setRelPath(path);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      if (dirty && !readonly) void save();
    }
  };

  const tree = React.useMemo(() => buildTree(list?.files ?? []), [list]);
  const extensions = React.useMemo(
    () => [markdown(), ...memdockCmTheme, EditorView.lineWrapping],
    [],
  );
  const editable = !readonly;

  const renderNodes = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((n) => {
      const pad = { paddingLeft: 8 + depth * 14 };
      if (n.kind === "dir") {
        const expanded = !collapsed.has(n.path);
        const Chevron = expanded ? ChevronDown : ChevronRight;
        return (
          <React.Fragment key={n.path}>
            <div
              role="treeitem"
              aria-expanded={expanded}
              aria-level={depth + 1}
              tabIndex={0}
              style={pad}
              onClick={() =>
                setCollapsed((prev) => {
                  const next = new Set(prev);
                  if (next.has(n.path)) next.delete(n.path);
                  else next.add(n.path);
                  return next;
                })
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  e.currentTarget.click();
                }
              }}
              className={rowClass}
            >
              <Chevron className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
              <Folder className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
              <span className="min-w-0 truncate">{n.name}</span>
            </div>
            {expanded ? <div role="group">{renderNodes(n.children, depth + 1)}</div> : null}
          </React.Fragment>
        );
      }
      const selected = n.path === relPath;
      return (
        <div
          key={n.path}
          role="treeitem"
          aria-selected={selected}
          aria-level={depth + 1}
          tabIndex={0}
          title={n.path}
          style={{ paddingLeft: 8 + depth * 14 + 18 }}
          onClick={() => open(n.path)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              open(n.path);
            }
          }}
          className={cn(rowClass, selected && "bg-muted shadow-[inset_2px_0_0_var(--primary)]")}
        >
          <File className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 truncate font-mono text-[12.5px]">{n.name}</span>
        </div>
      );
    });

  return (
    <div className="flex min-h-0 min-w-0 flex-1" onKeyDown={onKeyDown}>
      <div className="flex w-[220px] shrink-0 flex-col border-r border-border">
        <div role="tree" aria-label={s.filesTree} className="min-h-0 flex-1 overflow-y-auto p-2">
          {list === null && !listError ? (
            <p className="px-2 py-2 text-sm text-muted-foreground" aria-busy="true">
              {s.loading}
            </p>
          ) : null}
          {listError ? (
            <p role="alert" className="px-2 py-2 text-sm text-destructive">
              {fmt(s.loadFailed, { message: listError })}
            </p>
          ) : null}
          {list && list.files.length === 0 ? <p className="px-2 py-2 text-sm text-muted-foreground">{s.noFiles}</p> : null}
          {renderNodes(tree, 0)}
        </div>
        {list?.truncated ? (
          <p className="flex items-center gap-1.5 border-t border-border px-3 py-2 text-xs text-muted-foreground">
            <Info className="size-3.5 shrink-0" aria-hidden="true" />
            {s.filesTruncated}
          </p>
        ) : null}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-10 shrink-0 items-center gap-2.5 border-b border-border px-4">
          {dirty ? (
            <span role="img" aria-label={s.unsaved} title={s.unsaved} className="size-[7px] shrink-0 rounded-full bg-foreground" />
          ) : null}
          <span className="min-w-0 flex-1 truncate font-mono text-[12.5px] font-medium" title={relPath}>
            {relPath ?? s.selectFile}
          </span>
          {readonly || noText ? (
            <span className="shrink-0 rounded-full border border-border px-2 text-xs leading-5 text-muted-foreground">
              {s.readOnlyFile}
            </span>
          ) : (
            <Button
              type="button"
              disabled={!base || saving || !dirty}
              onClick={() => void save()}
              title={s.save}
              className="h-7 shrink-0 cursor-pointer gap-1.5 px-3 text-xs font-semibold"
            >
              <Save className="size-4" aria-hidden="true" />
              {s.save}
            </Button>
          )}
        </div>

        {external || saveError || loadError ? (
          <div className="flex shrink-0 flex-col gap-2 px-4 py-3">
            {external ? (
              <div className="flex items-center gap-2.5 rounded-md bg-warning/15 px-3 py-2 text-warning">
                <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
                <span className="text-foreground">{s.externalChanged}</span>
                <span className="flex-1" />
                <Button
                  type="button"
                  variant="outline"
                  onClick={reload}
                  className="h-7 cursor-pointer bg-card px-3 text-xs text-foreground hover:bg-muted"
                >
                  {dirty ? s.reloadDiscard : s.reload}
                </Button>
              </div>
            ) : null}
            {saveError ? (
              <div role="alert" className="flex items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-destructive">
                <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
                <span className="text-foreground">{saveError}</span>
              </div>
            ) : null}
            {loadError ? (
              <div role="alert" className="flex items-center gap-2.5 rounded-md bg-destructive/15 px-3 py-2 text-destructive">
                <TriangleAlert className="size-4 shrink-0" aria-hidden="true" />
                <span className="text-foreground">{fmt(s.fileLoadFailed, { message: loadError })}</span>
              </div>
            ) : null}
          </div>
        ) : null}

        <div className="min-h-0 min-w-0 flex-1 overflow-hidden bg-card">
          {base && noText ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
              <Info className="size-8 text-muted-foreground" aria-hidden="true" />
              <p className="text-sm text-muted-foreground">{base.binary ? s.binaryNotice : s.tooLargeNotice}</p>
            </div>
          ) : null}
          {base && !noText ? (
            <CodeMirror
              key={relPath}
              value={text}
              onChange={setText}
              extensions={extensions}
              theme="none"
              editable={editable}
              readOnly={!editable}
              height="100%"
              style={{ height: "100%" }}
              basicSetup={{ foldGutter: false, highlightActiveLineGutter: true, searchKeymap: true }}
            />
          ) : null}
        </div>
      </div>

      <ConfirmDialog
        open={conflict}
        title={s.externalChanged}
        body={s.conflictBody}
        confirmLabel={s.reloadDiscard}
        cancelLabel={s.cancel}
        destructive
        onConfirm={reload}
        onCancel={() => setConflict(false)}
      />
      <ConfirmDialog
        open={pendingPath !== null}
        title={s.discardTitle}
        body={s.discardBody}
        confirmLabel={s.discardConfirm}
        cancelLabel={s.keepEditing}
        destructive
        onConfirm={() => {
          if (pendingPath) setRelPath(pendingPath);
          setPendingPath(null);
        }}
        onCancel={() => setPendingPath(null)}
      />
    </div>
  );
}

const rowClass =
  "flex h-7 cursor-pointer items-center gap-1.5 rounded-md pr-2 text-sm text-foreground transition-colors duration-150 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none";
