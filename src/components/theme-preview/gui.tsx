import type { ThemeId } from "@/types/theme";
import "./styles.scss";

const NAV = ["Home", "Library", "Albums", "Map", "Memories"];

/**
 * A static miniature of the shell rendered with the real tokens of `theme`/`scheme`,
 * so previews in setup and Settings can never drift from the real thing. A plugin theme is
 * drawn by its base plus `plugin` (`<plugin>/<id>`), the same two attributes the page carries.
 */
export function ThemePreview({ theme, scheme, plugin }: { theme: ThemeId; scheme: "light" | "dark"; plugin?: string | null }) {
  return (
    <div
      className="theme-preview"
      data-theme={theme}
      data-plugin-theme={plugin ?? undefined}
      data-mantine-color-scheme={scheme}
      aria-hidden
    >
      <div className="theme-preview__sidebar">
        <div className="theme-preview__dots">
          <i />
          <i />
          <i />
        </div>
        <div className="theme-preview__brand">Lumen</div>
        {NAV.map((l, i) => (
          <div key={l} className="theme-preview__nav" data-active={i === 1}>
            {l}
          </div>
        ))}
      </div>
      <div className="theme-preview__main">
        <div className="theme-preview__header">
          <b>September 2026</b>
          <span>1,248 items</span>
        </div>
        <div className="theme-preview__grid">
          {Array.from({ length: 10 }, (_, i) => (
            <i key={i} data-selected={i === 2} />
          ))}
        </div>
      </div>
    </div>
  );
}
