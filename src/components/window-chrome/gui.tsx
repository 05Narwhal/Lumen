import { Portal } from "@mantine/core";
import { useState } from "react";
import { WindowControls } from "@/components/window-controls";
import "./styles.scss";

/**
 * The window's controls for screens that cover the shell's title bar (viewer, film player,
 * quick look; Round 15). The strip lets clicks through to the overlay's own top bar, which
 * drags the window with `useWindowDrag`; only the controls take the pointer. With `autoHide`
 * they stay out of the photo until the pointer reaches them or focus is inside. `overModal`
 * puts them above a full-screen Mantine modal (device browser, photo picker), whose header
 * leaves `--window-controls-w` free on the right.
 */
export function WindowChrome({ autoHide = false, overModal = false }: { autoHide?: boolean; overModal?: boolean }) {
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const visible = !autoHide || hover || focus;
  const chrome = (
    <div className="window-chrome" data-visible={visible} data-over-modal={overModal || undefined}>
      <div
        className="window-chrome__controls"
        // An overlay closes on a click outside its content (quick look): a control is never that.
        onClick={(e) => e.stopPropagation()}
        onMouseEnter={() => setHover(true)}
        onMouseLeave={() => setHover(false)}
        onFocus={() => setFocus(true)}
        onBlur={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFocus(false);
        }}
      >
        <WindowControls />
      </div>
    </div>
  );
  return overModal ? <Portal>{chrome}</Portal> : chrome;
}
