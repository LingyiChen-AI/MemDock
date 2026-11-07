import type { GraphNode, MemoryGraph } from "@/api/memoryGraph";
import type { MemoryFile } from "@/api/types";

export function projectGraphFiles(files: MemoryFile[], projectId: string): MemoryFile[] {
  return files.filter(
    (file) => file.exists && file.kind !== "codex_db" && file.scope.type === "project" && file.scope.project_id === projectId,
  );
}

export type SceneNode = Pick<GraphNode, "id" | "label" | "kind"> & {
  weight: number;
  displayLabel: string;
  x?: number;
  y?: number;
  vx?: number;
  vy?: number;
};
export type SceneLink = { index: number; from: string; to: string; curvature: number };

export const graphColors: Record<GraphNode["kind"], string> = {
  entity: "oklch(0.60 0.17 251)",
  concept: "oklch(0.59 0.17 294)",
  decision: "oklch(0.63 0.14 57)",
  tool: "oklch(0.57 0.12 173)",
  process: "oklch(0.60 0.16 355)",
};

export function createForceGraphData(graph: MemoryGraph): { nodes: SceneNode[]; links: SceneLink[] } {
  const degree = new Map<string, number>();
  const pairs = new Map<string, number>();
  const links = graph.relationships.map((edge, index) => {
    for (const id of [edge.fromNodeId, edge.toNodeId]) degree.set(id, (degree.get(id) ?? 0) + 1);
    const key = JSON.stringify([edge.fromNodeId, edge.toNodeId].sort());
    const lane = pairs.get(key) ?? 0;
    pairs.set(key, lane + 1);
    const direction = edge.fromNodeId < edge.toNodeId ? 1 : -1;
    return { index, from: edge.fromNodeId, to: edge.toNodeId, curvature: direction * Math.min(0.55, 0.12 + lane * 0.07) };
  });
  // The simulation mutates nodes and links. Keep saved graph records independent.
  const nodes = graph.nodes.map(({ id, label, kind }) => {
    const characters = Array.from(label);
    return {
      id, label, kind,
      displayLabel: characters.length > 23 ? `${characters.slice(0, 22).join("")}…` : label,
      weight: 2.6 + Math.min(degree.get(id) ?? 0, 10) * 0.55,
    };
  });
  return { nodes, links };
}
