import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * Opens a Radix Select (`trigger`) and picks an option by its value or visible text,
 * the counterpart of `userEvent.selectOptions` for a native `<select>`.
 */
export async function selectOption(trigger: HTMLElement, valueOrText: string): Promise<void> {
  await userEvent.click(trigger);
  const options = await screen.findAllByRole("option");
  const match =
    options.find((o) => o.getAttribute("data-value") === valueOrText) ?? options.find((o) => o.textContent?.trim() === valueOrText);
  if (!match) throw new Error(`no option "${valueOrText}" in [${options.map((o) => o.textContent).join(", ")}]`);
  await userEvent.click(match);
}

/** Visible labels of a Radix Select's options (opens and closes it). */
export async function optionLabels(trigger: HTMLElement): Promise<string[]> {
  await userEvent.click(trigger);
  const labels = (await screen.findAllByRole("option")).map((o) => o.textContent ?? "");
  await userEvent.keyboard("{Escape}");
  return labels;
}
