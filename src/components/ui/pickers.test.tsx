import * as React from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi } from "vitest";
import { renderApp } from "@/test/fixtures";
import { DatePicker, formatYmd, parseYmd } from "./date-picker";
import { NumberInput } from "./number-input";
import { TimePicker } from "./time-picker";

vi.mock("@/api/memory", () => ({
  getSettings: vi.fn().mockResolvedValue({ language: "zh", theme: null, accent: null, manual_projects: [] }),
  updateUiPrefs: vi.fn(),
  listMemories: vi.fn(),
}));

function Controlled<T>({ initial, children }: { initial: T; children: (v: T, set: (v: T) => void) => React.ReactNode }) {
  const [v, setV] = React.useState(initial);
  return <>{children(v, setV)}</>;
}

describe("ui pickers", () => {
  it("parses and formats local dates", () => {
    expect(formatYmd(parseYmd("2026-03-09")!)).toBe("2026-03-09");
    expect(parseYmd("2026/03/09")).toBeUndefined();
  });

  it("DatePicker picks a day and respects max", async () => {
    const onChange = vi.fn();
    renderApp(<DatePicker aria-label="开始日期" value="2026-03-09" max="2026-03-20" onChange={onChange} />);
    await userEvent.click(screen.getByRole("button", { name: "开始日期" }));
    const grid = await screen.findByRole("grid");
    await userEvent.click(within(grid).getByText("12"));
    expect(onChange).toHaveBeenCalledWith("2026-03-12");
    await userEvent.click(screen.getByRole("button", { name: "开始日期" }));
    expect(within(await screen.findByRole("grid")).getByText("31").closest("button")).toBeDisabled();
  });

  it("NumberInput steps and clamps", async () => {
    render(
      <Controlled initial="89">
        {(v, set) => <NumberInput aria-label="天数" value={v} onChange={set} min={1} max={90} incrementLabel="加" />}
      </Controlled>,
    );
    await userEvent.click(screen.getByRole("button", { name: "加" }));
    expect(screen.getByLabelText("天数")).toHaveValue("90");
    expect(screen.getByRole("button", { name: "加" })).toBeDisabled();
  });

  it("TimePicker sets hour and minute", async () => {
    render(
      <Controlled initial="09:00">
        {(v, set) => <TimePicker aria-label="时间" value={v} onChange={set} hourLabel="时" minuteLabel="分" />}
      </Controlled>,
    );
    await userEvent.click(screen.getByLabelText("时间"));
    await userEvent.click(within(screen.getByRole("listbox", { name: "时" })).getByRole("option", { name: "14" }));
    await userEvent.click(within(screen.getByRole("listbox", { name: "分" })).getByRole("option", { name: "45" }));
    expect(screen.getByLabelText("时间")).toHaveTextContent("14:45");
  });
});
