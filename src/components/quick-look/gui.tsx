import { AnimatePresence, motion } from "motion/react";
import { useEffect } from "react";
import { matches } from "@/lib/keybinds";
import { mediaUrl } from "@/utils/media";
import { closeQuickLook, quickLookStore } from "./store";
import { WindowChrome } from "@/components/window-chrome";
import "./styles.scss";

/** Space on a selected tile: a fast 1024 px look without the viewer's chrome. */
export function QuickLook() {
  const { id } = quickLookStore.use();
  useEffect(() => {
    if (id === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (matches(e, "grid.quick-look") || e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeQuickLook();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [id]);

  return (
    <AnimatePresence>
      {id !== null && (
        <motion.div
          className="quick-look"
          role="dialog"
          aria-label="Quick look"
          onClick={closeQuickLook}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
        >
          <WindowChrome autoHide />
          <motion.img
            className="quick-look__img"
            src={mediaUrl("thumb", id, { s: 1024 })}
            alt=""
            initial={{ scale: 0.96 }}
            animate={{ scale: 1 }}
            exit={{ scale: 0.96 }}
            transition={{ type: "spring", stiffness: 380, damping: 32 }}
          />
        </motion.div>
      )}
    </AnimatePresence>
  );
}
