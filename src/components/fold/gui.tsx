import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";
import "./styles.scss";

/** The sidebar's section fold (`sidebar/gui.tsx`): the shell's standard ease, a little quicker than a page. */
const FOLD = { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] as const };

/**
 * Children that slide open and closed (height and fade) instead of jumping, so what is below
 * moves up smoothly. Closed, they are not mounted. Reduced motion is the app's `MotionConfig`.
 */
export function Fold({ open, children, className }: { open: boolean; children: ReactNode; className?: string }) {
  return (
    <AnimatePresence initial={false}>
      {open && (
        <motion.div
          key="fold"
          className={className ? `fold ${className}` : "fold"}
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          exit={{ height: 0, opacity: 0 }}
          transition={FOLD}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
