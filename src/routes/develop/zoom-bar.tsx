import { Menu, Tooltip } from "@mantine/core";
import { IconChevronUp, IconColumns2, IconZoomIn, IconZoomOut } from "@tabler/icons-react";
import type { CompareMode } from "@/hooks/use-develop-session";
import { isMac } from "@/components/window-controls";
import { LEVELS, percent, type ZoomState } from "./zoom";

const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

const COMPARE: { value: CompareMode; label: string; key?: string }[] = [
  { value: "off", label: "Off" },
  { value: "toggle", label: "Toggle", key: "\\" },
  { value: "split", label: "Split", key: "Y" },
  { value: "side", label: "Side by side", key: "Y" },
];

/**
 * The canvas's zoom controls, at its lower right (Round 4J): before and after, fit, fill,
 * 100 %, a menu of levels and steps. The level button names the zoom the canvas shows.
 */
export function ZoomBar({
  zoom,
  shown,
  onZoom,
  onStep,
  compare,
  onCompare,
}: {
  zoom: ZoomState;
  /** The level on screen, for the label (a fill is some level too). */
  shown: number;
  onZoom: (z: ZoomState) => void;
  onStep: (dir: 1 | -1) => void;
  compare: CompareMode;
  onCompare: (mode: CompareMode) => void;
}) {
  const mod = isMac() ? "⌘" : "Ctrl+";
  const label = zoom.mode === "fit" ? "Fit" : zoom.mode === "fill" ? "Fill" : percent(shown);
  const isLevel = (l: number) => zoom.mode === "level" && Math.abs(zoom.level - l) < 1e-6;
  return (
    // Clicks here are the bar's own: they neither pan nor zoom the canvas beneath.
    <div className="develop-zoom" role="toolbar" aria-label="Zoom" onPointerDown={stop} onDoubleClick={stop}>
      <Menu position="top" shadow="md" withinPortal>
        <Menu.Target>
          <button type="button" className="develop-zoom__btn" aria-label="Before and after" aria-pressed={compare !== "off"} title="Before and after (\, Y)">
            <IconColumns2 size={15} />
          </button>
        </Menu.Target>
        <Menu.Dropdown>
          <Menu.Label>Before and after</Menu.Label>
          {COMPARE.map((c) => (
            <Menu.Item key={c.value} onClick={() => onCompare(c.value)} data-active={compare === c.value} rightSection={c.key && <span aria-hidden>{c.key}</span>}>
              {c.label}
            </Menu.Item>
          ))}
        </Menu.Dropdown>
      </Menu>
      <span className="develop-zoom__sep" aria-hidden />
      <Tooltip label={`Zoom out (${mod}−)`} openDelay={500} withArrow>
        <button type="button" className="develop-zoom__btn" aria-label="Zoom out" onClick={() => onStep(-1)}>
          <IconZoomOut size={15} />
        </button>
      </Tooltip>
      <Menu position="top" shadow="md" withinPortal>
        <Menu.Target>
          <button type="button" className="develop-zoom__level" aria-label="Zoom level">
            {label} <IconChevronUp size={11} />
          </button>
        </Menu.Target>
        <Menu.Dropdown>
          {[...LEVELS].reverse().map((l) => (
            <Menu.Item key={l} onClick={() => onZoom({ mode: "level", level: l })} data-active={isLevel(l)}>
              {percent(l)}
            </Menu.Item>
          ))}
        </Menu.Dropdown>
      </Menu>
      <Tooltip label={`Zoom in (${mod}=)`} openDelay={500} withArrow>
        <button type="button" className="develop-zoom__btn" aria-label="Zoom in" onClick={() => onStep(1)}>
          <IconZoomIn size={15} />
        </button>
      </Tooltip>
      <span className="develop-zoom__sep" aria-hidden />
      <Tooltip label={`Fit (${mod}0)`} openDelay={500} withArrow>
        <button type="button" className="develop-zoom__text" aria-pressed={zoom.mode === "fit"} onClick={() => onZoom({ mode: "fit" })}>
          Fit
        </button>
      </Tooltip>
      <button type="button" className="develop-zoom__text" aria-pressed={zoom.mode === "fill"} onClick={() => onZoom({ mode: "fill" })}>
        Fill
      </button>
      <Tooltip label="100 % (Z)" openDelay={500} withArrow>
        <button type="button" className="develop-zoom__text" aria-label="Zoom to 100 %" aria-pressed={isLevel(1)} onClick={() => onZoom({ mode: "level", level: 1 })}>
          1:1
        </button>
      </Tooltip>
    </div>
  );
}
