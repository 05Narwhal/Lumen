import { Button } from "@mantine/core";
import { IconEye } from "@tabler/icons-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import type { TryState } from "@/lib/theme-try";

/** The bar over the page while a theme is being tried: its name, the seconds left, Keep and Revert. */
export function TryBar({ state, onKeep, onRevert }: { state: TryState | null; onKeep: () => void; onRevert: () => void }) {
  const [now, setNow] = useState(() => Date.now());
  const until = state?.until;
  useEffect(() => {
    if (until === undefined) return;
    setNow(Date.now());
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, [until]);
  const left = until === undefined ? 0 : Math.max(0, Math.ceil((until - now) / 1000));
  return (
    <AnimatePresence>
      {state && (
        <motion.div
          className="try-bar"
          role="status"
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 16 }}
          transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
        >
          <IconEye size={16} aria-hidden />
          <span className="try-bar__text">
            Trying <strong>{state.name}</strong>
            <span className="try-bar__left">{left} s</span>
          </span>
          <Button size="xs" radius="xl" onClick={onKeep}>
            Keep
          </Button>
          <Button size="xs" radius="xl" variant="default" onClick={onRevert}>
            Revert
          </Button>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
