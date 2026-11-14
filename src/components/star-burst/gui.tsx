import { IconStarFilled } from "@tabler/icons-react";
import { AnimatePresence, motion } from "motion/react";
import { createStore } from "@/lib/store";
import "./styles.scss";

interface Burst {
  id: number;
  x: number;
  y: number;
  /** Star size in px; the sparks and ring are scaled from it. */
  size: number;
}

const burstStore = createStore<{ bursts: Burst[] }>({ bursts: [] });
let nextId = 1;

/** Where a burst should play. An element bursts over its own centre. */
export type BurstTarget = Element | { x: number; y: number } | null | undefined;

const SCREEN_SIZE = 96;
const MIN_SIZE = 40;
const MAX_SIZE = 120;

/**
 * Fire the favorite animation. Over an image it plays at the centre of that image, sized
 * to it; with nothing to aim at — the palette, a toolbar button — it plays large in the
 * middle of the window. It never plays at the pointer: a star that appears under the
 * cursor reads as a click artefact rather than as the picture being favourited.
 */
export function playStarBurst(target?: BurstTarget) {
  let x = window.innerWidth / 2;
  let y = window.innerHeight / 2;
  let size = SCREEN_SIZE;
  if (target && "getBoundingClientRect" in target) {
    const r = target.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) {
      x = r.left + r.width / 2;
      y = r.top + r.height / 2;
      // Roughly half the short side, so the star sits inside the picture at any tile size.
      size = Math.min(MAX_SIZE, Math.max(MIN_SIZE, Math.min(r.width, r.height) * 0.5));
    }
  } else if (target) {
    x = target.x;
    y = target.y;
    size = MIN_SIZE;
  }
  const id = nextId++;
  burstStore.set((s) => ({ bursts: [...s.bursts, { id, x, y, size }] }));
  setTimeout(() => burstStore.set((s) => ({ bursts: s.bursts.filter((b) => b.id !== id) })), 900);
}

/** Twelve sparks on an uneven ring: a perfectly regular star looks mechanical. */
const SPARKS = Array.from({ length: 12 }, (_, i) => {
  const a = (i / 12) * Math.PI * 2 + (i % 3) * 0.14;
  const reach = i % 3 === 0 ? 1.25 : i % 3 === 1 ? 0.95 : 1.1;
  return {
    cos: Math.cos(a),
    sin: Math.sin(a),
    reach,
    dot: i % 4 === 0 ? 0.11 : 0.075,
    warm: i % 3 !== 2,
    delay: 0.03 + (i % 4) * 0.015,
  };
});

/** Mount once in the root layout. */
export function StarBurstHost() {
  const { bursts } = burstStore.use();
  return (
    <div className="star-burst-host" aria-hidden>
      <AnimatePresence>
        {bursts.map((b) => (
          <div key={b.id} className="star-burst" style={{ left: b.x, top: b.y }}>
            {/* A ring of light leaving the star, which reads as the burst even at small sizes. */}
            <motion.span
              className="star-burst__ring"
              style={{ width: b.size, height: b.size, marginLeft: -b.size / 2, marginTop: -b.size / 2 }}
              initial={{ scale: 0.3, opacity: 0.55 }}
              animate={{ scale: 2.1, opacity: 0 }}
              transition={{ duration: 0.65, ease: "easeOut" }}
            />
            <motion.span
              className="star-burst__star"
              style={{ marginLeft: -b.size / 2, marginTop: -b.size / 2 }}
              initial={{ scale: 0, rotate: -35, opacity: 0 }}
              animate={{ scale: [0, 1.3, 1, 1, 0.85], rotate: [-35, 6, 0, 0, 0], opacity: [0, 1, 1, 1, 0] }}
              transition={{ duration: 0.85, times: [0, 0.3, 0.45, 0.7, 1], ease: [0.2, 0.8, 0.2, 1] }}
            >
              <IconStarFilled size={b.size} />
            </motion.span>
            {SPARKS.map((s, i) => {
              const dot = Math.round(b.size * s.dot);
              const reach = b.size * 0.95 * s.reach;
              return (
                <motion.span
                  key={i}
                  className="star-burst__spark"
                  data-warm={s.warm}
                  style={{ width: dot, height: dot, marginLeft: -dot / 2, marginTop: -dot / 2 }}
                  initial={{ x: 0, y: 0, scale: 0.3, opacity: 0 }}
                  animate={{
                    x: s.cos * reach,
                    y: s.sin * reach,
                    scale: [0.3, 1, 0.2],
                    opacity: [0, 1, 0],
                  }}
                  transition={{ duration: 0.7, ease: [0.16, 1, 0.3, 1], delay: s.delay }}
                />
              );
            })}
          </div>
        ))}
      </AnimatePresence>
    </div>
  );
}
