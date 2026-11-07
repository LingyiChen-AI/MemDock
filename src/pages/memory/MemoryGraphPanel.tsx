import * as React from "react";
import {
  ArrowDownRight, ArrowUpRight, Box, FileText, GitFork, Lightbulb, ListChecks,
  Loader2, Network, RefreshCw, Sparkles, Wrench, X,
} from "lucide-react";
import {
  extractMemoryGraph, getMemoryGraph, type GraphKind, type GraphTool, type MemoryGraph, type SavedMemoryGraph,
} from "@/api/memoryGraph";
import type { MemoryFile, ProjectMemories } from "@/api/types";
import { Button } from "@/components/ui/button";
import { SimpleSelect } from "@/components/ui/select";
import { useI18n } from "@/i18n/I18nProvider";
import { graphColors, projectGraphFiles } from "./memoryGraph";
import { memoryGraphCopy } from "./memoryGraphCopy";
const MemoryForceCanvas = React.lazy(() => import("./MemoryForceCanvas"));

const toolNames = { claude: "Claude Code", codex: "Codex" };
const nodeStyles = {
  entity: { color: graphColors.entity, icon: Box },
  concept: { color: graphColors.concept, icon: Lightbulb },
  decision: { color: graphColors.decision, icon: GitFork },
  tool: { color: graphColors.tool, icon: Wrench },
  process: { color: graphColors.process, icon: ListChecks },
} satisfies Record<GraphKind, { color: string; icon: typeof Box }>;
const canvas = "bg-background bg-[radial-gradient(ellipse_at_40%_35%,color-mix(in_oklch,var(--tool-codex)_11%,var(--background)),transparent_82%)]";

export type CachedMemoryGraph = SavedMemoryGraph;

type Copy = (typeof memoryGraphCopy)["en" | "zh"];
type Selection = { nodeId: string } | { edgeIndex: number };
type OpenSource = (file: MemoryFile, line: number) => boolean;

export function MemoryGraphPanel({ entry, cache, onOpenSource }: {
  entry: ProjectMemories;
  cache: Map<string, CachedMemoryGraph>;
  onOpenSource: OpenSource;
}) {
  const { language } = useI18n();
  const copy = memoryGraphCopy[language];
  const { id, name } = entry.project;
  const [result, setResult] = React.useState(() => cache.get(id));
  const [tool, setTool] = React.useState<GraphTool>(() => cache.get(id)?.tool ?? "claude");
  const [busy, setBusy] = React.useState(false);
  const [restoring, setRestoring] = React.useState(() => !cache.has(id));
  const [error, setError] = React.useState<{ kind: "restore" | "extract"; message: string } | null>(null);
  const [version, setVersion] = React.useState(0);
  const generationStarted = React.useRef(false);
  const generationSucceeded = React.useRef(false);
  const toolChanged = React.useRef(false);
  const files = projectGraphFiles(entry.files, id);
  const hasGraph = !!result?.graph.nodes.length;

  React.useEffect(() => {
    if (cache.has(id)) return;
    let active = true;
    void getMemoryGraph(id).then((saved) => {
      if (!active || generationSucceeded.current) return;
      const latest = cache.get(id) ?? saved;
      if (!latest) return;
      cache.set(id, latest);
      setResult(latest);
      if (!generationStarted.current && !toolChanged.current) setTool(latest.tool);
    }).catch((cause: unknown) => {
      if (active && !generationStarted.current) setError({ kind: "restore", message: errorMessage(cause) });
    }).finally(() => {
      if (active) setRestoring(false);
    });
    return () => { active = false; };
  }, [id, cache]);

  async function generate() {
    generationStarted.current = true;
    setRestoring(false);
    setBusy(true);
    setError(null);
    try {
      const graph = await extractMemoryGraph(id, tool, language);
      generationSucceeded.current = true;
      const next = { graph, tool };
      cache.set(id, next);
      setResult(next);
      setVersion((value) => value + 1);
    } catch (cause) {
      setError({ kind: "extract", message: errorMessage(cause) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-label={copy.title} className="@container flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-card">
      <header className="flex shrink-0 flex-wrap items-center gap-x-5 gap-y-3 border-b border-border px-4 py-3">
        <span className="grid size-10 shrink-0 place-items-center rounded-xl bg-primary/10 text-primary">
          <Network className="size-4" aria-hidden="true" />
        </span>
        <div className="-ml-2 min-w-0 flex-1">
          <p className="truncate text-xs text-muted-foreground" title={name}>{name} / {copy.scope}</p>
          <h2 className="mt-0.5 text-base font-semibold tracking-tight">{copy.title}</h2>
        </div>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <SimpleSelect aria-label={copy.tool} value={tool} disabled={busy}
              onValueChange={(value) => {
                toolChanged.current = true;
                setTool(value === "codex" ? "codex" : "claude");
              }}
              options={[{ value: "claude", label: "Claude Code" }, { value: "codex", label: "Codex" }]}
          />
          <Button type="button" size="lg" disabled={busy || !files.length} onClick={() => void generate()}>
            {busy ? <Loader2 className="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              : result ? <RefreshCw aria-hidden="true" /> : <Sparkles aria-hidden="true" />}
            {busy ? copy.refreshing : result ? copy.refresh : copy.generate}
          </Button>
        </div>
      </header>
      {busy && hasGraph && <p role="status" className="sr-only">{copy.loading}</p>}
      {error !== null && (
        <p role="alert" className="shrink-0 border-b border-destructive/20 bg-destructive/5 break-words px-4 py-2 text-sm text-destructive">
          {error.kind === "restore" ? copy.restoreFailed : copy.failed}{error.message && <> · {error.message}</>}
        </p>
      )}
      {result && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-border px-4 py-2 text-xs text-muted-foreground">
          <span className="font-medium text-foreground">{toolNames[result.tool]}</span>
          <span className="tabular-nums"><b className="font-semibold text-foreground">{result.graph.nodes.length}</b> {copy.nodes}</span>
          <span className="tabular-nums"><b className="font-semibold text-foreground">{result.graph.relationships.length}</b> {copy.edges}</span>
          {result.graph.unread > 0 && <span>{copy.partial} {result.graph.unread}</span>}
        </div>
      )}
      {result && hasGraph ? (
        <GraphView key={version} graph={result.graph} files={files} copy={copy} onOpenSource={onOpenSource} />
      ) : (
        <div className={`grid min-h-64 flex-1 place-items-center p-6 ${canvas}`}>
          <div role={busy || restoring ? "status" : undefined} className="flex max-w-sm flex-col items-center px-8 py-7 text-center">
            <span className="mb-4 grid size-14 place-items-center rounded-2xl bg-primary/10 text-primary">
              {busy || restoring ? <Loader2 className="size-5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
                : <Network className="size-5" aria-hidden="true" />}
            </span>
            <p className="text-base font-semibold tracking-tight">{busy ? copy.loading : restoring ? copy.restoring : copy.title}</p>
            {!restoring && <p className="mt-1 text-sm text-muted-foreground">
              {busy ? copy.extracting : result ? copy.empty : files.length ? copy.ready : copy.noFiles}
            </p>}
            {busy && <span className="mt-4 rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">{toolNames[tool]}</span>}
          </div>
        </div>
      )}
    </section>
  );
}

function errorMessage(cause: unknown) {
  const message = cause && typeof cause === "object" && "message" in cause ? cause.message : cause;
  return typeof message === "string" ? message : "";
}

function GraphView({ graph, files, copy, onOpenSource }: {
  graph: MemoryGraph; files: MemoryFile[]; copy: Copy; onOpenSource: OpenSource;
}) {
  const [selection, setSelection] = React.useState<Selection | null>(null);
  const [unavailable, setUnavailable] = React.useState(false);
  const { nodes, relationships } = graph;
  const detailHeading = React.useRef<HTMLHeadingElement>(null);
  const focusDetails = React.useRef(false);
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const filesById = new Map(files.map((file) => [file.id, file]));
  const node = selection && "nodeId" in selection ? byId.get(selection.nodeId) : undefined;
  const edge = selection && "edgeIndex" in selection ? relationships[selection.edgeIndex] : undefined;
  const selected = node ?? edge;
  const relatedEdges = relationships.flatMap((relation, index) => {
    if (relation.fromNodeId === node?.id) return [{ index, other: relation.toNodeId, outgoing: true }];
    if (relation.toNodeId === node?.id) return [{ index, other: relation.fromNodeId, outgoing: false }];
    return [];
  });
  const sourceIds = new Set(selected?.sourceFileIds);
  const sources = graph.sources.filter((source) => sourceIds.has(source.fileId));

  React.useLayoutEffect(() => {
    if (focusDetails.current) {
      detailHeading.current?.focus();
      focusDetails.current = false;
    }
  }, [selection]);

  function select(next: Selection) {
    setUnavailable(false);
    setSelection(next);
  }

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      onKeyDown={(event) => { if (event.key === "Escape" && selection) setSelection(null); }}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <React.Suspense fallback={<div className="min-h-0 flex-1 bg-muted/25" />}>
          <MemoryForceCanvas graph={graph} selection={selection}
            onSelect={(next) => next ? select(next) : setSelection(null)} copy={copy} />
        </React.Suspense>
        <div className="sr-only focus-within:not-sr-only focus-within:flex focus-within:max-h-36 focus-within:flex-wrap focus-within:gap-1 focus-within:overflow-auto focus-within:p-2">
          {nodes.map((item) => (
            <Button key={item.id} size="sm" variant="ghost" aria-pressed={node?.id === item.id}
              onClick={() => select({ nodeId: item.id })}>{item.label}</Button>
          ))}
          {relationships.map((relation, index) => (
            <Button key={index} size="sm" variant="ghost" aria-pressed={edge === relation}
              onClick={() => select({ edgeIndex: index })}>
              {byId.get(relation.fromNodeId)?.label} → {relation.label} → {byId.get(relation.toNodeId)?.label}
            </Button>
          ))}
        </div>
        <div className="flex shrink-0 flex-wrap gap-x-3 gap-y-1 border-t border-border px-4 py-2 text-xs text-muted-foreground">
          {(Object.keys(nodeStyles) as GraphKind[]).filter((kind) => nodes.some((item) => item.kind === kind)).map((kind) => {
            const { color, icon: Icon } = nodeStyles[kind];
            return <span key={kind} className="inline-flex items-center gap-1.5"><Icon size={13} color={color} aria-hidden="true" />{copy.kinds[kind]}</span>;
          })}
        </div>
      </div>
      {selected && (
        <aside aria-label={copy.inspector}
          className="absolute left-3 top-3 z-10 max-h-[55%] w-72 max-w-[calc(100%_-_1.5rem)] overflow-auto rounded-lg border border-border bg-card shadow-md">
          <div className="sticky top-0 flex items-center justify-between border-b border-border bg-card px-3 py-1.5">
            <span className="text-sm font-semibold">{copy.inspector}</span>
            <Button type="button" variant="ghost" size="icon-sm" aria-label={copy.closeInspector}
              onClick={() => setSelection(null)}><X aria-hidden="true" /></Button>
          </div>
          <div className="space-y-4 p-4">
            {node && (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{copy.kinds[node.kind]}</p>
                <h3 className="mt-1 break-words text-base font-semibold">{node.label}</h3>
              </div>
            )}
            {edge && (
              <div>
                <p className="text-xs font-medium text-muted-foreground">{copy.relationship}</p>
                <h3 ref={detailHeading} tabIndex={-1}
                  className="mt-1 break-words rounded-sm text-base font-semibold focus-visible:outline-2 focus-visible:outline-ring">{edge.label}</h3>
                <p className="mt-1 break-words text-sm text-muted-foreground">
                  {byId.get(edge.fromNodeId)?.label} → {byId.get(edge.toNodeId)?.label}
                </p>
              </div>
            )}
            <div>
              <h4 className="mb-2 text-xs font-semibold">{copy.sources}</h4>
              {sources.length ? (
                <ul aria-label={copy.sources} className="space-y-1.5">
                  {sources.map((source) => {
                    const file = filesById.get(source.fileId);
                    return (
                      <li key={source.fileId} className="flex min-w-0 items-center gap-2 rounded-md border border-border bg-background p-2">
                        <FileText className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                        <div title={file?.path ?? source.title} className="min-w-0 flex-1">
                          <p className="truncate text-xs font-medium">{source.title}</p>
                          <p className="mt-0.5 text-xs text-muted-foreground">{copy.line} {source.line}</p>
                        </div>
                        <Button type="button" variant="ghost" size="icon-sm" title={copy.source} aria-label={copy.source} disabled={!file}
                          onClick={() => { if (file) setUnavailable(!onOpenSource(file, source.line)); }}>
                          <ArrowUpRight aria-hidden="true" />
                        </Button>
                      </li>
                    );
                  })}
                </ul>
              ) : <p className="text-xs text-muted-foreground">{copy.noSources}</p>}
              {unavailable && <p role="status" className="mt-2 text-xs text-muted-foreground">{copy.unavailable}</p>}
            </div>
            {node && (
              <div className="border-t border-border pt-3">
                <h4 className="mb-1.5 text-xs font-semibold">{copy.relationships} <span className="text-muted-foreground">{relatedEdges.length}</span></h4>
                {relatedEdges.length ? (
                  <ul>
                    {relatedEdges.map(({ index, other, outgoing }) => {
                      const Icon = outgoing ? ArrowUpRight : ArrowDownRight;
                      return (
                        <li key={index}>
                          <button type="button" className="flex w-full cursor-pointer items-start gap-2 rounded-md px-2 py-1.5 text-left hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
                            onClick={() => { focusDetails.current = true; select({ edgeIndex: index }); }}>
                            <Icon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                            <span className="min-w-0 break-words text-xs">
                              <span className="block font-medium">{relationships[index].label}</span>
                              <span className="block text-muted-foreground">{byId.get(other)?.label}</span>
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                ) : <p className="text-xs text-muted-foreground">{copy.noRelationships}</p>}
              </div>
            )}
          </div>
        </aside>
      )}
    </div>
  );
}
