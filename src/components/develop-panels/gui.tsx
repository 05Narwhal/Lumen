import { IconPhotoAi, IconWand } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { AiButton } from "@/components/ai-button";
import { DevelopHistogram } from "@/components/develop-histogram";
import { StatusBadge } from "@/components/status-badge";
import { useAiTask } from "@/hooks/use-ai-task";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { useSettings } from "@/hooks/use-settings";
import { DEFAULT_DEVELOP, developOf } from "@/lib/develop";
import { maskSession, maskUi } from "@/lib/develop-masks";
import { experimentalCatalogQuery, prismInstalled } from "@/lib/experimental";
import { BasicPanel } from "./basic";
import { CropButton, CropPanel, useFrameAspect } from "./crop";
import { DevelopMasks } from "@/components/develop-masks";
import { CurvePanel } from "./curve";
import { DetailPanel, EffectsPanel } from "./detail-effects";
import { GradingPanel } from "./grading";
import { autoRequests, autoTone } from "./auto";
import { HslPanel } from "./hsl";
import type { PanelScope } from "./panel";
import { retouchOn, retouchPhoto, retouchRequests } from "./retouch";
import "./styles.scss";
import type { AutoResult } from "@/types/generated/AutoResult";
import type { RetouchResult } from "@/types/generated/RetouchResult";

/** Runs `run` each time `store`'s counter changes after mounting (a key or the palette asked). */
function useRequests(store: typeof autoRequests, run: () => void) {
  const asked = store.use();
  const seen = useRef(asked);
  const latest = useRef(run);
  latest.current = run;
  useEffect(() => {
    if (asked === seen.current) return;
    seen.current = asked;
    latest.current();
  }, [asked]);
}

/** The right column (spec §1): the histogram, then every panel. */
export function DevelopPanels({ session }: { session: DevelopSession }) {
  const changed = session.draft.develop !== null && JSON.stringify(session.draft.develop) !== JSON.stringify(DEFAULT_DEVELOP);
  // One task per AI action, whichever way it was started: button, key or palette.
  const autoAi = useAiTask<AutoResult | null>("auto");
  const retouchAi = useAiTask<RetouchResult | null>("retouch");
  // Read on every render, so switching it in Settings shows or hides Retouch at once.
  const showRetouch = retouchOn(useSettings().settings);
  // Round 14: Retouch runs Lumen Prism; until one is installed it shows, disabled, In development.
  const catalog = useQuery({ ...experimentalCatalogQuery, enabled: showRetouch });
  const prismMissing = !prismInstalled(catalog.data);
  useRequests(autoRequests, () => void autoTone(session, autoAi));
  useRequests(retouchRequests, () => {
    if (showRetouch && !prismMissing) void retouchPhoto(session, retouchAi);
  });
  // Masks are drawn on the unedited photo, so their thumbnails take its shape.
  const unedited = useFrameAspect(session.assetId, { ...session.draft, rotate: 0 });
  // Round 19 §1: with a mask selected the column is about that mask. The same panels edit its
  // adjustments through `maskSession`; Crop, Auto and the photo's own values step aside.
  const picked = maskUi.use().selected;
  const mask = picked === null ? undefined : developOf(session.draft).masks?.find((m) => m.id === picked);
  const scope: PanelScope = mask ? "mask" : "photo";
  const sections = mask ? maskSession(session, mask.id) : session;
  return (
    <div className="develop-panels" data-mask-mode={mask ? "" : undefined}>
      <DevelopHistogram histogram={session.histogram} clipping={session.clipping} onClipping={session.setClipping} />
      {!mask && (
      <div className="develop-panels__bar">
        <div className="develop-panels__tools" role="toolbar" aria-label="Develop tools">
          <AiButton
            label="Auto"
            icon={<IconWand size={14} />}
            state={autoAi.state}
            onRun={() => void autoTone(session, autoAi)}
            onCancel={() => void autoAi.cancel()}
          />
          {showRetouch && (
            <AiButton
              label="Retouch"
              title={prismMissing ? "Retouch arrives with Lumen Prism." : "Retouch with Lumen Prism (experimental)"}
              icon={<IconPhotoAi size={14} />}
              state={retouchAi.state}
              disabled={prismMissing}
              onRun={() => void retouchPhoto(session, retouchAi)}
              onCancel={() => void retouchAi.cancel()}
            />
          )}
          {showRetouch && prismMissing && <StatusBadge>In development</StatusBadge>}
          <CropButton session={session} />
        </div>
        <button type="button" className="develop-panel__link" disabled={!changed} onClick={() => session.apply(() => DEFAULT_DEVELOP, "Reset all")}>
          Reset all
        </button>
      </div>
      )}
      {!mask && <CropPanel session={session} />}
      <DevelopMasks session={session} aspect={unedited} />
      <BasicPanel session={sections} scope={scope} />
      <CurvePanel session={sections} />
      <HslPanel session={sections} />
      <GradingPanel session={sections} />
      <DetailPanel session={sections} scope={scope} />
      <EffectsPanel session={sections} scope={scope} />
    </div>
  );
}
