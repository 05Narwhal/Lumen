import { Button } from "@mantine/core";
import { AnimatePresence, motion, useReducedMotionConfig } from "motion/react";
import { type CSSProperties, type ReactNode, useEffect, useState } from "react";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import { StoreIcon, swatchOf } from "./tile";

/** What the carousel needs of a plugin or a theme. */
export type HeroItem = Pick<StoreEntry, "id" | "name" | "author" | "jl" | "summary" | "icon" | "screenshots">;

const ADVANCE_MS = 7000;
const EASE = [0.22, 1, 0.36, 1] as const;

/**
 * The featured carousel: text left, a tilted screenshot right that straightens on hover, dots,
 * and an auto-advance that pauses while the pointer or the focus is inside and never runs
 * under reduced motion.
 */
export function Hero<T extends HeroItem>({
  items,
  action,
  open,
  stage,
  label = "Featured plugins",
}: {
  items: T[];
  /** The primary button (Get / Open / Update, Try / Get ...). */
  action: (e: T) => ReactNode;
  open: (e: T) => void;
  /** What fills the tilted stage instead of the first screenshot (a theme's miniature). */
  stage?: (e: T) => ReactNode;
  label?: string;
}) {
  const [at, setAt] = useState(0);
  const [held, setHeld] = useState(false);
  const reduced = useReducedMotionConfig() ?? false;
  const n = items.length;
  const i = n === 0 ? 0 : at % n;

  useEffect(() => {
    if (n < 2 || held || reduced) return;
    const t = window.setTimeout(() => setAt((a) => (a + 1) % n), ADVANCE_MS);
    return () => window.clearTimeout(t);
  }, [n, held, reduced, at]);

  if (n === 0) return null;
  const e = items[i];
  const shot = e.screenshots[0];
  return (
    <section
      className="store-hero"
      aria-label="Featured"
      aria-roledescription="carousel"
      onPointerEnter={() => setHeld(true)}
      onPointerLeave={() => setHeld(false)}
      onFocus={() => setHeld(true)}
      onBlur={(ev) => {
        if (!ev.currentTarget.contains(ev.relatedTarget as Node | null)) setHeld(false);
      }}
    >
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.div
          key={e.id}
          className="store-hero__slide"
          aria-roledescription="slide"
          aria-label={`${i + 1} of ${n}: ${e.name}`}
          initial={{ opacity: 0, x: 24 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: -24 }}
          transition={{ duration: 0.45, ease: EASE }}
        >
          <div className="store-hero__text">
            <span className="store-hero__eyebrow">{e.jl ? "Featured · By JL Studios" : "Featured"}</span>
            <div className="store-hero__title">
              <StoreIcon url={e.icon} size={56} />
              <h2>{e.name}</h2>
            </div>
            <span className="store-hero__author">{e.author}</span>
            <p className="store-hero__summary">{e.summary}</p>
            <div className="store-hero__actions">
              {action(e)}
              <Button size="sm" radius="xl" variant="subtle" onClick={() => open(e)} aria-label={`Learn more about ${e.name}`}>
                Learn more
              </Button>
            </div>
          </div>
          <button type="button" className="store-hero__stage" onClick={() => open(e)} tabIndex={-1} aria-hidden>
            <span className="store-hero__shot" style={{ "--tile-swatch": `var(--cover-swatch-${swatchOf(e.id)})` } as CSSProperties}>
              {stage ? stage(e) : shot ? <img src={shot} alt={`${e.name} screenshot`} draggable={false} /> : e.icon ? <img className="store-hero__wash" src={e.icon} alt="" draggable={false} /> : null}
            </span>
          </button>
        </motion.div>
      </AnimatePresence>
      {n > 1 && (
        <div className="store-hero__dots" role="group" aria-label={label}>
          {items.map((x, k) => (
            <button key={x.id} type="button" className="store-hero__dot" aria-label={`Show ${x.name}`} aria-current={k === i} onClick={() => setAt(k)}>
              {k === i && !held && !reduced && <span key={at} className="store-hero__dot-fill" style={{ animationDuration: `${ADVANCE_MS}ms` }} />}
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
