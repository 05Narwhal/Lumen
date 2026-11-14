import { AnimatePresence, motion } from "motion/react";
import { dismissToast, toastStore } from "./store";
import "./styles.scss";

export function Toaster() {
  const { toasts } = toastStore.use();
  return (
    <div className="toaster" aria-live="polite">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            className="toast"
            layout
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 12 }}
            transition={{ duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <span className="toast__message">{t.message}</span>
            {t.action && (
              <button
                type="button"
                className="toast__undo"
                onClick={() => {
                  t.action?.run();
                  dismissToast(t.id);
                }}
              >
                {t.action.label}
              </button>
            )}
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}
