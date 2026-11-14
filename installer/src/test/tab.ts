import userEvent from "@testing-library/user-event";
import { expect } from "vitest";

/** Every enabled control inside `root`, in the DOM order. */
export function controls(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('button, input, [role="switch"], a[href]')].filter(
    (el) => !(el as HTMLButtonElement).disabled,
  );
}

/** Tabs through the window and asserts that each enabled control took focus (none is mouse-only). */
export async function expectEveryControlReachableByTab(root: HTMLElement) {
  const user = userEvent.setup();
  const seen = new Set<Element>();
  for (let i = 0; i < controls(root).length + 2; i++) {
    await user.tab();
    if (document.activeElement) seen.add(document.activeElement);
  }
  for (const el of controls(root)) {
    expect(seen.has(el), `${el.outerHTML.slice(0, 80)} was not reached by Tab`).toBe(true);
  }
}
