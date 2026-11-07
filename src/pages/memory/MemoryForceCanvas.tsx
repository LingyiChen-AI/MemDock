import * as React from "react";
import ForceGraph2D, { type ForceGraphMethods } from "react-force-graph-2d";
import { Maximize2, Minus, Plus } from "lucide-react";
import type { MemoryGraph } from "@/api/memoryGraph";
import { Button } from "@/components/ui/button";
import { memoryGraphCopy } from "./memoryGraphCopy";
import { createForceGraphData, graphColors, type SceneLink, type SceneNode } from "./memoryGraph";

export type GraphSelection = { nodeId: string } | { edgeIndex: number };
type Copy = (typeof memoryGraphCopy)["en" | "zh"];
const nodeRadiusScale = 7;
export default function MemoryForceCanvas({ graph, selection, onSelect, copy }: {
  graph: MemoryGraph;
  selection: GraphSelection | null;
  onSelect: (selection: GraphSelection | null) => void;
  copy: Copy;
}) {
  const container = React.useRef<HTMLDivElement>(null);
  const instance = React.useRef<ForceGraphMethods<SceneNode, SceneLink> | undefined>(undefined);
  const autoFit = React.useRef({ ticks: 0, settled: false, manual: false, painted: false });
  const [size, setSize] = React.useState({ width: 0, height: 0 });
  const [painted, setPainted] = React.useState(false);
  const [dark, setDark] = React.useState(() => document.documentElement.classList.contains("dark"));
  const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  const data = React.useMemo(() => createForceGraphData(graph), [graph]);
  const visible = size.width > 0 && size.height > 0;
  const padding = Math.min(60, Math.min(size.width, size.height) * 0.11);

  React.useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    const measure = () => {
      const { clientWidth: width, clientHeight: height } = element;
      setSize((current) => current.width === width && current.height === height ? current : { width, height });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  React.useEffect(() => {
    const root = document.documentElement;
    const observer = new MutationObserver(() => setDark(root.classList.contains("dark")));
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);

  React.useLayoutEffect(() => {
    const view = instance.current;
    if (!visible || !view || reducedMotion) return;
    view.d3Force("charge")?.strength(-86);
    const link = view.d3Force("link");
    link?.distance(64);
    link?.strength(0.32);
    // The default center force only moves the centroid. A soft pull keeps
    // disconnected clusters from drifting away and shrinking the fitted view.
    const idleAnchors = new Map<string, { x: number; y: number }>();
    view.d3Force("compact", (alpha) => {
      const idle = alpha < 0.045;
      if (!idle) idleAnchors.clear();
      const time = performance.now() / 1800;
      for (const [index, node] of data.nodes.entries()) {
        const x = node.x ?? 0;
        const y = node.y ?? 0;
        const strength = alpha * 0.045;
        node.vx = (node.vx ?? 0) - x * strength;
        node.vy = (node.vy ?? 0) - y * strength;
        if (idle) {
          const anchor = idleAnchors.get(node.id) ?? { x, y };
          idleAnchors.set(node.id, anchor);
          const phase = time + index * 2.4;
          node.vx += (anchor.x + Math.sin(phase) * 5 - x) * 0.025;
          node.vy += (anchor.y + Math.cos(phase) * 4 - y) * 0.025;
        }
      }
    });
    view.d3ReheatSimulation();
  }, [visible, reducedMotion, data]);

  const nodeId = selection && "nodeId" in selection ? selection.nodeId : undefined;
  const edgeIndex = selection && "edgeIndex" in selection ? selection.edgeIndex : undefined;
  const isRelated = (link: SceneLink) => link.index === edgeIndex || link.from === nodeId || link.to === nodeId;
  const focusNodes = new Set(data.links.filter(isRelated).flatMap((link) => [link.from, link.to]));
  if (nodeId) focusNodes.add(nodeId);

  function fit() {
    autoFit.current.manual = true;
    instance.current?.zoomToFit(reducedMotion ? 0 : 260, padding);
  }

  function zoom(factor: number) {
    autoFit.current.manual = true;
    const view = instance.current;
    if (view) view.zoom(Math.max(0.2, Math.min(5, view.zoom() * factor)), reducedMotion ? 0 : 180);
  }

  function keepNodesInFrame() {
    const state = autoFit.current;
    const view = instance.current;
    if (!state.painted || state.manual || state.settled || !view) return;
    const tick = ++state.ticks;
    if (tick >= 220) {
      state.settled = true;
      return;
    }
    if (tick % 12 !== 0) return;
    const bounds = view.getGraphBbox();
    if (!bounds) return;
    const topLeft = view.graph2ScreenCoords(bounds.x[0], bounds.y[0]);
    const bottomRight = view.graph2ScreenCoords(bounds.x[1], bounds.y[1]);
    const margin = padding / 2;
    if (topLeft.x < margin || topLeft.y < margin || bottomRight.x > size.width - margin || bottomRight.y > size.height - margin) {
      const width = bounds.x[1] - bounds.x[0];
      const height = bounds.y[1] - bounds.y[0];
      const fitZoom = Math.min((size.width - padding * 2) / width, (size.height - padding * 2) / height);
      view.centerAt((bounds.x[0] + bounds.x[1]) / 2, (bounds.y[0] + bounds.y[1]) / 2);
      if (fitZoom < view.zoom()) view.zoom(fitZoom);
    }
  }

  function revealCenteredFrame(context: CanvasRenderingContext2D) {
    const state = autoFit.current;
    const view = instance.current;
    if (state.painted || !view) return;
    const bounds = view.getGraphBbox();
    if (!bounds || ![...bounds.x, ...bounds.y].every(Number.isFinite)) return;
    // Inspect the transform used by the actual Canvas frame. A camera command
    // alone does not tell us whether that frame was drawn with the new camera.
    const transform = context.getTransform();
    const ratio = window.devicePixelRatio || 1;
    const left = (bounds.x[0] * transform.a + transform.e) / ratio;
    const right = (bounds.x[1] * transform.a + transform.e) / ratio;
    const top = (bounds.y[0] * transform.d + transform.f) / ratio;
    const bottom = (bounds.y[1] * transform.d + transform.f) / ratio;
    const centered = Math.abs(left + right - size.width) <= size.width * 0.2
      && Math.abs(top + bottom - size.height) <= size.height * 0.2;
    if (!centered || left < 0 || top < 0 || right > size.width || bottom > size.height) {
      view.zoomToFit(0, padding);
      return;
    }
    state.painted = true;
    setPainted(true);
  }

  React.useEffect(() => {
    if (!visible || autoFit.current.manual || !autoFit.current.settled) return;
    const frame = requestAnimationFrame(() => {
      if (!autoFit.current.manual) instance.current?.zoomToFit(0, padding);
    });
    return () => cancelAnimationFrame(frame);
  }, [size.width, size.height, padding, visible]);

  return (
    <div ref={container} role="group" aria-label={copy.title}
      onPointerDownCapture={() => { if (painted) autoFit.current.manual = true; }}
      onWheelCapture={() => { if (painted) autoFit.current.manual = true; }}
      className="relative min-h-0 min-w-0 flex-1 overflow-hidden bg-background bg-[radial-gradient(ellipse_at_40%_35%,color-mix(in_oklch,var(--tool-codex)_11%,var(--background)),transparent_82%)]">
      {visible && (
        <div className={`absolute inset-0 overflow-hidden ${painted ? "" : "invisible"}`}>
        <ForceGraph2D<SceneNode, SceneLink>
          ref={instance}
          graphData={data}
          linkSource="from"
          linkTarget="to"
          width={size.width}
          height={size.height}
          nodeVal="weight"
          nodeRelSize={nodeRadiusScale}
          nodeLabel={() => ""}
          linkLabel={() => ""}
          linkCurvature="curvature"
          minZoom={0.2}
          maxZoom={5}
          warmupTicks={reducedMotion ? 120 : 60}
          cooldownTicks={reducedMotion ? 1 : Infinity}
          cooldownTime={reducedMotion ? 15000 : Infinity}
          d3VelocityDecay={0.35}
          d3AlphaDecay={0.024}
          nodeColor={(node) => selection && !focusNodes.has(node.id) ? "rgba(124,139,158,0.30)" : graphColors[node.kind]}
          linkColor={(link) => isRelated(link) ? (dark ? "rgb(143 186 246)" : "rgb(71 123 177)") : (selection ? "rgba(118,139,166,0.12)" : "rgba(118,139,166,0.30)")}
          linkWidth={(link) => isRelated(link) ? 1.8 : 0.85}
          linkDirectionalArrowLength={(link) => isRelated(link) ? 5 : 0}
          nodeCanvasObjectMode={() => "after"}
          nodeCanvasObject={(node, context, scale) => {
            if (node.x === undefined || node.y === undefined || (scale < 0.45 && !focusNodes.has(node.id))) return;
            const radius = Math.sqrt(node.weight) * nodeRadiusScale;
            context.save();
            if (node.id === nodeId) {
              context.beginPath();
              context.arc(node.x, node.y, radius + 3 / scale, 0, 2 * Math.PI);
              context.strokeStyle = graphColors[node.kind];
              context.lineWidth = 1.5 / scale;
              context.stroke();
            }
            context.font = `${node.id === nodeId ? 600 : 500} ${12 / scale}px system-ui, sans-serif`;
            context.textAlign = "center";
            context.textBaseline = "top";
            context.strokeStyle = dark ? "rgb(20 26 35)" : "rgb(248 251 255)";
            context.lineWidth = 3 / scale;
            context.fillStyle = dark ? "rgb(214 225 238)" : "rgb(40 62 86)";
            context.globalAlpha = selection && !focusNodes.has(node.id) ? 0.45 : 1;
            const y = node.y + radius + 5 / scale;
            context.strokeText(node.displayLabel, node.x, y);
            context.fillText(node.displayLabel, node.x, y);
            context.restore();
          }}
          onNodeClick={(node) => onSelect({ nodeId: node.id })}
          onLinkClick={(link) => onSelect({ edgeIndex: link.index })}
          onBackgroundClick={() => onSelect(null)}
          onEngineTick={keepNodesInFrame}
          onRenderFramePost={revealCenteredFrame}
          onEngineStop={() => { autoFit.current.settled = true; }}
        />
        </div>
      )}
      <div className="absolute bottom-3 right-3 flex items-center rounded-md border border-border bg-background p-0.5 shadow-sm">
        <Button type="button" variant="ghost" size="icon-sm" title={copy.zoomOut} aria-label={copy.zoomOut}
          disabled={!painted} onClick={() => zoom(0.8)}><Minus aria-hidden="true" /></Button>
        <Button type="button" variant="ghost" size="icon-sm" title={copy.resetZoom} aria-label={copy.resetZoom}
          disabled={!painted} onClick={fit}><Maximize2 aria-hidden="true" /></Button>
        <Button type="button" variant="ghost" size="icon-sm" title={copy.zoomIn} aria-label={copy.zoomIn}
          disabled={!painted} onClick={() => zoom(1.25)}><Plus aria-hidden="true" /></Button>
      </div>
    </div>
  );
}
