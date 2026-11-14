import { useEffect, type CSSProperties, type ReactNode } from "react";
import { api } from "@/lib/api";
import { useWindowDrag } from "@/lib/use-window-drag";
import "./styles.scss";

interface Props {
  title?: string;
  /** The wash's brightness, 0.45 at the start to 1 when Lumen is ready. */
  light?: number;
  /** Centre the content (Welcome, Done). */
  center?: boolean;
  children: ReactNode;
}

/** The 640 × 420 frame: wash, 38 px title bar with minimise and close, content. */
export function Window({ title = "Lumen Setup", light = 0.45, center = false, children }: Props) {
  const drag = useWindowDrag();
  useEffect(() => {
    document.title = title;
  }, [title]);
  return (
    <div className="setup-window" style={{ "--light": light } as CSSProperties}>
      <header className="setup-bar" {...drag}>
        <i className="setup-bar__mark" aria-hidden />
        <span className="setup-bar__title">{title}</span>
        <div className="setup-bar__controls">
          <button type="button" aria-label="Minimise" onClick={() => void api.minimize()}>
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden><path d="M1 5h8" stroke="currentColor" strokeWidth="1.2" /></svg>
          </button>
          <button type="button" aria-label="Close" onClick={() => void api.close()}>
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden><path d="M1 1l8 8M9 1L1 9" stroke="currentColor" strokeWidth="1.2" /></svg>
          </button>
        </div>
      </header>
      <main className={center ? "setup-body setup-body--center" : "setup-body"}>{children}</main>
    </div>
  );
}
