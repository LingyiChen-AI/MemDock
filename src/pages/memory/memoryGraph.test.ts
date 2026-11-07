import type { MemoryGraph } from "@/api/memoryGraph";
import { globalClaude, projAutos, projClaude } from "@/test/fixtures";
import { createForceGraphData, projectGraphFiles } from "./memoryGraph";

it("only considers existing files for the selected project", () => {
  const missing = { ...projAutos[0], exists: false };
  expect(projectGraphFiles([globalClaude, projClaude, missing], "/work/jev-rpa")).toEqual([projClaude]);
  expect(projectGraphFiles([projClaude], "/other")).toEqual([]);
});

it("maps stable selection indices and gives the force engine independent scene records", () => {
  const graph: MemoryGraph = {
    nodes: [
      { id: "a", label: "Workflow", kind: "process", sourceFileIds: ["file"] },
      { id: "b", label: "Tool", kind: "tool", sourceFileIds: ["file"] },
      { id: "c", label: "Independent", kind: "entity", sourceFileIds: ["file"] },
    ],
    relationships: [
      { fromNodeId: "a", toNodeId: "b", label: "uses", sourceFileIds: ["file"] },
      { fromNodeId: "b", toNodeId: "a", label: "supports", sourceFileIds: ["file"] },
    ],
    sources: [],
    unread: 0,
  };
  const data = createForceGraphData(graph);
  expect(data.links).toMatchObject([
    { index: 0, from: "a", to: "b" },
    { index: 1, from: "b", to: "a" },
  ]);
  expect(data.links[0].curvature).not.toBe(data.links[1].curvature);
  expect(data.nodes[0].weight).toBeGreaterThan(data.nodes[2].weight);
  data.nodes[0].label = "Changed";
  data.links[0].from = "changed";
  expect(graph.nodes[0].label).toBe("Workflow");
  expect(graph.relationships[0].fromNodeId).toBe("a");
});
