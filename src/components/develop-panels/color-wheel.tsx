import { useRef, type KeyboardEvent, type PointerEvent } from "react";

/** A point in a wheel of `radius` (relative to its centre, y down) as hue and saturation. */
export function pointToHueSat(dx: number, dy: number, radius: number): { hue: number; sat: number } {
  const hue = ((Math.atan2(-dy, dx) * 180) / Math.PI + 360) % 360;
  const sat = Math.min(1, Math.hypot(dx, dy) / radius);
  return { hue: Math.round(hue), sat: Math.round(sat * 100) / 100 };
}

/** Where the puck sits, as a fraction of the wheel's size from its top-left. */
export function puckPosition(hue: number, sat: number): { left: number; top: number } {
  const t = (hue * Math.PI) / 180;
  return { left: 0.5 + 0.5 * sat * Math.cos(t), top: 0.5 - 0.5 * sat * Math.sin(t) };
}

/**
 * A colour-grading wheel: the puck's angle is the tint's hue (OKLab's, so it matches the
 * render), its distance from the centre the strength. Drag or click to place it; arrow keys
 * turn it (←/→) and strengthen it (↑/↓); double-click returns it to the centre.
 */
export function ColorWheel({
  label,
  hue,
  sat,
  onChange,
  onCommit,
}: {
  label: string;
  hue: number;
  sat: number;
  onChange: (hue: number, sat: number) => void;
  onCommit: (hue: number, sat: number, label: string) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const last = useRef({ hue, sat });
  const at = (e: PointerEvent) => {
    const r = ref.current!.getBoundingClientRect();
    return pointToHueSat(e.clientX - (r.left + r.width / 2), e.clientY - (r.top + r.height / 2), r.width / 2);
  };
  const describe = (h: number, s: number) => `${label} ${Math.round(h)}° ${Math.round(s * 100)}`;
  const down = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    last.current = at(e);
    onChange(last.current.hue, last.current.sat);
  };
  const move = (e: PointerEvent<HTMLDivElement>) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    last.current = at(e);
    onChange(last.current.hue, last.current.sat);
  };
  const up = (e: PointerEvent<HTMLDivElement>) => {
    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
    e.currentTarget.releasePointerCapture(e.pointerId);
    onCommit(last.current.hue, last.current.sat, describe(last.current.hue, last.current.sat));
  };
  const key = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 15 : 5;
    let [h, s] = [hue, sat];
    if (e.key === "ArrowLeft") h = (h + step) % 360;
    else if (e.key === "ArrowRight") h = (h - step + 360) % 360;
    else if (e.key === "ArrowUp") s = Math.min(1, Math.round((s + 0.05) * 100) / 100);
    else if (e.key === "ArrowDown") s = Math.max(0, Math.round((s - 0.05) * 100) / 100);
    else return;
    e.preventDefault();
    onChange(h, s);
    onCommit(h, s, describe(h, s));
  };
  const p = puckPosition(hue, sat);
  return (
    <div
      ref={ref}
      className="color-wheel"
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuetext={`Hue ${Math.round(hue)} degrees, strength ${Math.round(sat * 100)}`}
      aria-valuenow={Math.round(sat * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onKeyDown={key}
      onDoubleClick={() => {
        onChange(0, 0);
        onCommit(0, 0, `${label} reset`);
      }}
    >
      <span className="color-wheel__puck" style={{ left: `${p.left * 100}%`, top: `${p.top * 100}%` }} data-centre={sat === 0} />
    </div>
  );
}
