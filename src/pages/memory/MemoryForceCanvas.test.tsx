import { act, fireEvent, screen } from "@testing-library/react";
import { vi } from "vitest";
import { renderApp } from "@/test/fixtures";
import { memoryGraphCopy } from "./memoryGraphCopy";
import MemoryForceCanvas from "./MemoryForceCanvas";

const engine = vi.hoisted(() => ({
  tick: undefined as (() => void) | undefined,
  stop: undefined as (() => void) | undefined,
  paint: undefined as ((context: CanvasRenderingContext2D) => void) | undefined,
  cooldown: undefined as number | undefined,
  zoomToFit: vi.fn(),
  d3Force: vi.fn(),
  d3ReheatSimulation: vi.fn(),
  getGraphBbox: vi.fn(() => ({ x: [-50, 50], y: [-50, 50] })),
}));

vi.mock("react-force-graph-2d", async () => {
  const React = await import("react");
  return {
    default: ({ ref, onEngineTick, onEngineStop, onRenderFramePost, width, height, cooldownTicks }: {
      ref: React.Ref<unknown>;
      onEngineTick: () => void;
      onEngineStop: () => void;
      onRenderFramePost: (context: CanvasRenderingContext2D) => void;
      width: number;
      height: number;
      cooldownTicks: number;
    }) => {
      React.useImperativeHandle(ref, () => engine, []);
      engine.tick = onEngineTick;
      engine.stop = onEngineStop;
      engine.paint = onRenderFramePost;
      engine.cooldown = cooldownTicks;
      return <canvas data-testid="force-canvas" width={width} height={height} />;
    },
  };
});

it("keeps an off-center frame hidden and respects manual camera input after revealing", () => {
  vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(800);
  vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(400);
  try {
    renderApp(<MemoryForceCanvas graph={{ nodes: [], relationships: [], sources: [], unread: 0 }}
      selection={null} onSelect={vi.fn()} copy={memoryGraphCopy.zh} />);
    const canvas = screen.getByTestId("force-canvas");
    expect(canvas).toHaveAttribute("width", "800");
    expect(canvas).toHaveAttribute("height", "400");
    expect(canvas.parentElement).toHaveClass("absolute", "inset-0");
    expect(canvas.parentElement).toHaveClass("invisible");
    const frame = (x: number, y: number) => ({
      getTransform: () => ({ a: 1, d: 1, e: x, f: y }),
    } as CanvasRenderingContext2D);
    act(() => engine.paint?.(frame(0, 0)));
    expect(canvas.parentElement).toHaveClass("invisible");
    expect(engine.zoomToFit).toHaveBeenCalledWith(0, 44);
    act(() => engine.paint?.(frame(400, 200)));
    expect(canvas.parentElement).not.toHaveClass("invisible");
    expect(engine.cooldown).toBe(Infinity);
    act(() => engine.stop?.());
    expect(engine.zoomToFit).toHaveBeenCalledTimes(1);
    fireEvent.wheel(screen.getByRole("group", { name: "记忆图谱" }));
    act(() => engine.stop?.());
    expect(engine.zoomToFit).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "适应画布" }));
    expect(engine.zoomToFit).toHaveBeenCalledTimes(2);
  } finally {
    vi.restoreAllMocks();
  }
});
