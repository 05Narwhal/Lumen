import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Toaster } from "@/components/toast";
import { ImportStep } from "./steps/import";
import { LibraryStep } from "./steps/library";
import { ThemeStep } from "./steps/theme";
import { WelcomeStep } from "./steps/welcome";
import "./styles.scss";
import { useWindowDrag } from "@/hooks/use-window-drag";

const STEPS = 4;

/** First-run wizard: welcome → theme → library → first import. Every step has a default. */
export function SetupRoute() {
  const [step, setStep] = useState(0);
  const [dir, setDir] = useState(1);
  const go = (to: number) => {
    setDir(to > step ? 1 : -1);
    setStep(Math.min(Math.max(to, 0), STEPS - 1));
  };
  const next = () => go(step + 1);
  const back = () => go(step - 1);

  const drag = useWindowDrag();
  return (
    <div className="setup">
      <div className="setup__drag" {...drag} />
      {/* The panel keeps one fixed box; steps cross-fade inside it so nothing reflows. */}
      <div className="setup__panel">
        <AnimatePresence mode="wait" initial={false} custom={dir}>
          <motion.div
            key={step}
            className="setup__stage"
            custom={dir}
            variants={{
              enter: (d: number) => ({ opacity: 0, x: 32 * d, filter: "blur(4px)" }),
              center: { opacity: 1, x: 0, filter: "blur(0px)" },
              exit: (d: number) => ({ opacity: 0, x: -32 * d, filter: "blur(4px)" }),
            }}
            initial="enter"
            animate="center"
            exit="exit"
            transition={{ duration: 0.24, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {step === 0 && <WelcomeStep onNext={next} />}
            {step === 1 && <ThemeStep onNext={next} onBack={back} />}
            {step === 2 && <LibraryStep onBack={back} onNext={next} />}
            {step === 3 && <ImportStep onBack={back} />}
          </motion.div>
        </AnimatePresence>
      </div>
      <div className="setup__dots" aria-label={`Step ${step + 1} of ${STEPS}`}>
        {Array.from({ length: STEPS }, (_, i) => (
          <i key={i} data-active={i === step} />
        ))}
      </div>
      <Toaster />
    </div>
  );
}
