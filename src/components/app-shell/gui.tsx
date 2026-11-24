import { ActivityRing } from "@/components/activity-ring";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Sidebar, type SidebarDevice } from "@/components/sidebar";
import { WindowControls, isMac } from "@/components/window-controls";
import { createStore } from "@/lib/store";
import { HealthBanner } from "@/components/health-banner";
import { SecurityBanner } from "@/components/security-banner";
import "./styles.scss";
import { useWindowDrag } from "@/hooks/use-window-drag";

const dirs = ["North", "South", "East", "West", "NorthWest", "NorthEast", "SouthWest", "SouthEast"] as const;

/** The title bar's toolbar slot, and how many screens are drawing into it. */
const toolbarSlot = createStore<{ el: HTMLElement | null; users: number }>({ el: null, users: 0 });
const setSlot = (el: HTMLElement | null) => toolbarSlot.set((s) => (s.el === el ? s : { ...s, el }));

/**
 * Draws a screen's toolbar into the title bar's row, before the window controls (Round 4J:
 * Develop's top bar). Without a shell (tests, a screen on its own) it draws in place.
 */
export function TitlebarPortal({ children }: { children: ReactNode }) {
  const { el } = toolbarSlot.use();
  useEffect(() => {
    toolbarSlot.set((s) => ({ ...s, users: s.users + 1 }));
    return () => toolbarSlot.set((s) => ({ ...s, users: s.users - 1 }));
  }, []);
  return el ? createPortal(children, el) : <>{children}</>;
}

export function AppShell({
  children,
  devices,
  sidebarCollapsed,
  onToggleSidebar,
  sidebar = true,
}: {
  children: ReactNode;
  devices: SidebarDevice[];
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  /** Off for screens that take the whole window (Settings, Round 6). */
  sidebar?: boolean;
}) {
  const mac = isMac();
  const { users } = toolbarSlot.use();
  // Any empty part of the bar drags, the toolbar slot's gaps included (Round 15).
  const drag = useWindowDrag();
  return (
    <div className="app-shell" data-part="shell" data-mac={mac}>
      {sidebar && <Sidebar collapsed={sidebarCollapsed} onToggle={onToggleSidebar} devices={devices} />}
      <div className="app-shell__body">
        <div className="app-shell__titlebar" data-part="titlebar" data-toolbar={users > 0} {...drag}>
          <div className="app-shell__toolbar" data-part="toolbar" ref={setSlot} />
          {/* Background work, top right on every OS (left of the controls where they show). */}
          <ActivityRing compact placement="titlebar" />
          <WindowControls />
        </div>
        <div className="app-shell__content" data-part="main">
          <HealthBanner />
          <SecurityBanner />
          <main className="app-shell__main">{children}</main>
        </div>
      </div>
      {!mac &&
        dirs.map((d) => (
          <div
            key={d}
            className={`app-shell__resize app-shell__resize--${d.toLowerCase()}`}
            onMouseDown={(e) => {
              e.preventDefault();
              getCurrentWindow().startResizeDragging(d);
            }}
          />
        ))}
    </div>
  );
}
