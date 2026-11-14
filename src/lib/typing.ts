/** Inputs whose keys are the user's text, not the page's shortcuts. */
const TEXT_INPUTS = new Set(["text", "search", "email", "url", "tel", "password", "number", "date", "time", "datetime-local", "month", "week"]);

/**
 * Whether keys pressed in `target` are typing (Round 8B). A slider, a checkbox or a button
 * is not: the viewer's seek bar kept focus after a drag and swallowed Escape.
 */
export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.getAttribute("contenteditable") === "true") return true;
  if (target instanceof HTMLTextAreaElement) return true;
  return target instanceof HTMLInputElement && TEXT_INPUTS.has(target.type);
}
