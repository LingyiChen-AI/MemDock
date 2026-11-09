import { render, screen } from "@testing-library/react";
import { vi } from "vitest";
import type { ToolQuota } from "@/api/types";
import { QuotaCards } from "./QuotaCards";

vi.mock("@/i18n/I18nProvider", async () => {
  const { getMessages } = await import("@/i18n");
  return { useI18n: () => ({ language: "zh", t: getMessages("zh") }) };
});

it("distinguishes a reported zero from a missing five-hour quota", () => {
  const weekly = { used_percent: 31, resets_at: null, window_minutes: 10080 };
  const weeklyOnly: ToolQuota = { status: "ok", source: "live", updated_at: null, weekly, five_hour: null };
  render(
    <QuotaCards
      claude={{ ...weeklyOnly, five_hour: { used_percent: 0, resets_at: 60_000, window_minutes: 300 } }}
      codex={weeklyOnly}
      now={0}
      onEnableClaude={() => {}}
      onEnableCodex={() => {}}
      enabling={false}
    />,
  );

  expect(screen.getAllByText("5 小时窗口")).toHaveLength(1);
  expect(screen.getByRole("progressbar", { name: "5 小时窗口" })).toHaveAttribute("aria-valuenow", "0");
  expect(screen.getAllByRole("progressbar", { name: "本周" })).toHaveLength(2);
});
