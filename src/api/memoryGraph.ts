import { invoke } from "@tauri-apps/api/core";

export type GraphTool = "claude" | "codex";
export type GraphKind = "entity" | "concept" | "decision" | "tool" | "process";

export interface GraphNode {
  id: string;
  label: string;
  kind: GraphKind;
  sourceFileIds: string[];
}

export interface GraphRelationship {
  fromNodeId: string;
  toNodeId: string;
  label: string;
  sourceFileIds: string[];
}

export interface MemoryGraph {
  nodes: GraphNode[];
  relationships: GraphRelationship[];
  sources: { fileId: string; title: string; line: number }[];
  unread: number;
}

export interface SavedMemoryGraph {
  graph: MemoryGraph;
  tool: GraphTool;
}

export function getMemoryGraph(projectId: string) {
  return invoke<SavedMemoryGraph | null>("memory_graph_get", { projectId });
}

export function extractMemoryGraph(projectId: string, tool: GraphTool, language: "zh" | "en") {
  return invoke<MemoryGraph>("memory_graph_extract", { projectId, tool, language });
}
