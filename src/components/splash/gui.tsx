import { AnimatePresence, motion } from "motion/react";
import { BrandMark } from "@/components/brand-mark";
import { WindowControls } from "@/components/window-controls";
import "./styles.scss";

/** Full-screen loading surface shown while settings and the library registry load. */
export function Splash({ visible, note }: { visible: boolean; note?: string }) {
  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          className="splash"
          role="status"
          aria-label="Loading Lumen"
          data-tauri-drag-region
          initial={{ opacity: 1 }}
          exit={{ opacity: 0, scale: 1.02 }}
          transition={{ duration: 0.32, ease: [0.2, 0.8, 0.2, 1] }}
        >
          {/* A stuck start can still be minimised or closed (Round 21 §11). */}
          <div className="splash__controls">
            <WindowControls />
          </div>
          <div className="splash__logo">
            <BrandMark size={96} />
          </div>
          <div className="splash__wordmark">Lumen</div>
          <div className="splash__bar" aria-hidden>
            <i />
          </div>
          {/* Round 24 §6.3: a start that sticks says why. */}
          {note && (
            <p className="splash__note" role="alert">
              {note}
            </p>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
