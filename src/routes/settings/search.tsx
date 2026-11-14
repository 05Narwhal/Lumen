import { Children, createContext, isValidElement, useContext, useSyncExternalStore, type ReactNode } from "react";

/** The sidebar's query while Settings is searching (empty: not searching). */
export const SearchContext = createContext("");
export const useSearchQuery = () => useContext(SearchContext);

/** The plain text of a label or hint, however it is wrapped. */
export function textOf(node: ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return Children.toArray(node.props.children).map(textOf).join("");
  return "";
}

export const matches = (query: string, ...parts: ReactNode[]) => {
  const q = query.trim().toLowerCase();
  return !q || parts.some((p) => textOf(p).toLowerCase().includes(q));
};

/** `node` with each match of `query` in a `<mark>`; anything but a string is left as it is. */
export function highlight(node: ReactNode, query: string): ReactNode {
  const q = query.trim();
  if (!q || typeof node !== "string") return node;
  const lower = node.toLowerCase();
  const needle = q.toLowerCase();
  const out: ReactNode[] = [];
  let at = 0;
  for (let i = lower.indexOf(needle); i !== -1; i = lower.indexOf(needle, at)) {
    if (i > at) out.push(node.slice(at, i));
    out.push(<mark key={i}>{node.slice(i, i + q.length)}</mark>);
    at = i + q.length;
  }
  if (at < node.length) out.push(node.slice(at));
  return out;
}

/** The row the pointer or focus is on, for the inspector; kept outside React state so a hover never re-renders a section. */
export interface Pointed {
  label: ReactNode;
  hint: ReactNode;
}
let pointed: Pointed | null = null;
const listeners = new Set<() => void>();
export const pointAt = (p: Pointed | null) => {
  if (p === pointed) return;
  pointed = p;
  listeners.forEach((l) => l());
};
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};
export const usePointed = () => useSyncExternalStore(subscribe, () => pointed);
