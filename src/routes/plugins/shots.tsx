import { ActionIcon, Modal } from "@mantine/core";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";

/** A plugin page's screenshots: a snapping strip with arrows when it overflows. */
export function Shots({ name, shots, onOpen }: { name: string; shots: string[]; onOpen: (i: number) => void }) {
  const track = useRef<HTMLUListElement>(null);
  const [edge, setEdge] = useState({ start: true, end: true });
  const measure = () => {
    const t = track.current;
    if (!t) return;
    setEdge({ start: t.scrollLeft <= 2, end: t.scrollLeft + t.clientWidth >= t.scrollWidth - 2 });
  };
  useEffect(() => {
    measure();
    const t = track.current;
    if (!t || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(t);
    return () => ro.disconnect();
  }, [shots]);
  const step = (dir: 1 | -1) => {
    const t = track.current;
    t?.scrollBy({ left: dir * t.clientWidth * 0.8, behavior: "smooth" });
  };
  return (
    <section className="plugin-shots" aria-label={`Screenshots of ${name}`}>
      <ul ref={track} className="plugin-shots__track" onScroll={measure}>
        {shots.map((src, i) => (
          <li key={src}>
            <button type="button" className="plugin-shots__item" onClick={() => onOpen(i)} aria-label={`Screenshot ${i + 1}`}>
              <img src={src} alt="" loading="lazy" draggable={false} />
            </button>
          </li>
        ))}
      </ul>
      {!edge.start && (
        <ActionIcon className="plugin-shots__arrow plugin-shots__arrow--prev" variant="default" radius="xl" size="lg" onClick={() => step(-1)} aria-label="Previous screenshots">
          <IconChevronLeft size={18} />
        </ActionIcon>
      )}
      {!edge.end && (
        <ActionIcon className="plugin-shots__arrow plugin-shots__arrow--next" variant="default" radius="xl" size="lg" onClick={() => step(1)} aria-label="More screenshots">
          <IconChevronRight size={18} />
        </ActionIcon>
      )}
    </section>
  );
}

/** One screenshot enlarged; arrows (and ← →) move through them, Escape or a click closes. */
export function Lightbox({ shots, at, onChange }: { shots: string[]; at: number | null; onChange: (i: number | null) => void }) {
  const n = shots.length;
  const go = (d: 1 | -1) => at !== null && onChange((at + d + n) % n);
  return (
    <Modal
      opened={at !== null}
      onClose={() => onChange(null)}
      size="auto"
      centered
      withCloseButton={false}
      classNames={{ content: "plugin-lightbox", body: "plugin-lightbox__body" }}
      onKeyDown={(e) => {
        if (n < 2) return;
        if (e.key === "ArrowRight") go(1);
        else if (e.key === "ArrowLeft") go(-1);
      }}
    >
      {at !== null && (
        <>
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.img
              key={shots[at]}
              src={shots[at]}
              alt={`Screenshot ${at + 1}, enlarged`}
              onClick={() => onChange(null)}
              initial={{ opacity: 0, scale: 0.98 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.22 }}
            />
          </AnimatePresence>
          {n > 1 && (
            <div className="plugin-lightbox__bar">
              <ActionIcon variant="default" radius="xl" onClick={() => go(-1)} aria-label="Previous screenshot">
                <IconChevronLeft size={16} />
              </ActionIcon>
              <span className="plugin-lightbox__count">
                {at + 1} / {n}
              </span>
              <ActionIcon variant="default" radius="xl" onClick={() => go(1)} aria-label="Next screenshot">
                <IconChevronRight size={16} />
              </ActionIcon>
            </div>
          )}
        </>
      )}
    </Modal>
  );
}
