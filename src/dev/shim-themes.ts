// Browser-shim sample data for installed `.lumentheme` themes (Round 26 B2). Only active with
// `?shimtheme=top`: it lists "Skyline", whose theme.css moves the sidebar to the top, and makes it
// the theme in use. Imported once from `browser-shim.ts`; it wraps the shim's `invoke` after the
// shim has set it, so the shim's own command table stays untouched.

const ID = "studio.shim.skyline";
const swatch = (bg: string, surface: string, sidebar: string, fg: string, fg2: string, accent: string, placeholder: string) => ({
  bg, surface, sidebar, fg, fg2, accent, placeholder, radius: "10px", gap: "6px",
});

export const skyline = {
  id: ID,
  name: "Skyline",
  author: "Shim Studio",
  version: "1.0.0",
  description: "Navigation along the top.",
  license: "CC-BY-4.0",
  // The base follows `?theme=` so the look can be checked on each of them.
  base: new URLSearchParams(window.location.search).get("theme") ?? "darkroom",
  defaultScheme: "dark",
  layout: true,
  hasIcon: false,
  tokens: {
    light: swatch("#f6f7fb", "#ffffff", "#e9ecf5", "#14161f", "#5b6075", "#3b5bdb", "#dfe3ef"),
    dark: swatch("#12141c", "#1a1d29", "#171a25", "#eef0f8", "#9aa0b8", "#7c93ff", "#252a3a"),
  },
};

// What Rust writes for a theme.css holding THEMES.md's example, scoped and kept off protected parts.
const NOT_PROTECTED =
  ':not(:is([data-part="window-controls"], [data-part="dialog"], [data-part="confirm"], [data-part="toast"], [data-part="menu"], [data-part="install-sheet"], [data-part="palette"]) *)';
const scope = `[data-plugin-theme="${ID}/theme"]`;
export const skylineCss = [
  `${scope} [data-part="shell"]${NOT_PROTECTED} { grid-template-areas: "titlebar" "sidebar" "main"; grid-template-columns: minmax(0, 1fr); grid-template-rows: auto auto minmax(0, 1fr); }`,
  `${scope} [data-part="sidebar"]${NOT_PROTECTED} { flex-direction: row; height: auto; border-right-width: 0; border-bottom: 1px solid var(--border); }`,
].join("\n");

if (new URLSearchParams(window.location.search).get("shimtheme") === "top") {
  // The shim sets `__TAURI_INTERNALS__` when its own module body runs, after this one; a
  // microtask runs before `main.tsx` goes on from `await import("@/dev/browser-shim")`.
  queueMicrotask(() => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__;
    const real = internals.invoke;
    internals.invoke = async (cmd, args) => {
      if (cmd === "themes_list") return [...((await real(cmd, args)) as unknown[]), skyline];
      if (cmd === "plugin_theme_css") return `${(await real(cmd, args)) as string}\n${skylineCss}`;
      if (cmd === "theme_tried") return null;
      if (cmd === "get_settings") {
        const s = (await real(cmd, args)) as { appearance: Record<string, unknown> };
        return { ...s, appearance: { ...s.appearance, theme: { plugin: ID, theme: "theme" } } };
      }
      return real(cmd, args);
    };
  });
}
