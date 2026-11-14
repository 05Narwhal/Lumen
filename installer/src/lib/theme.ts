// Glass, light or dark with Windows. `?scheme=` overrides it for screenshots.
export function applyScheme(doc: Document = document, win: Window = window): () => void {
  const forced = new URLSearchParams(win.location.search).get("scheme");
  const set = (dark: boolean) => {
    doc.documentElement.setAttribute("data-mantine-color-scheme", dark ? "dark" : "light");
    // Native controls (the Remove box) draw in this scheme, not the system's.
    doc.documentElement.style.colorScheme = dark ? "dark" : "light";
  };
  doc.documentElement.setAttribute("data-theme", "glass");
  if (forced === "dark" || forced === "light") {
    set(forced === "dark");
    return () => {};
  }
  const mq = win.matchMedia("(prefers-color-scheme: dark)");
  set(mq.matches);
  const on = (e: MediaQueryListEvent) => set(e.matches);
  mq.addEventListener("change", on);
  return () => mq.removeEventListener("change", on);
}
