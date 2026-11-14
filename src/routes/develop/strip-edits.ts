import { useCallback } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { stripIdsWhere } from "@/hooks/use-develop-strip";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { developSelection } from "@/lib/develop";
import { clearDevelop, developSourceStore, removeFromDevelop, restoreDevelop, type DevelopSource } from "@/lib/develop-source";

/** Every photo the strip holds, whatever the mark filter hides for now. */
async function everyPhoto(source: DevelopSource): Promise<number[]> {
  return source.only ?? stripIdsWhere(source, { flag: null, colorLabels: [], minRating: null });
}

/**
 * Emptying the filmstrip and taking photos out of it (Round 4A). Both flush the open photo's
 * pending save first, and both are reversible with the toast's Undo, which reopens the
 * photo that was open. `shown` is the strip as it is drawn, for choosing the next photo.
 */
export function useStripEdits(session: DevelopSession, assetId: number, shown: number[]) {
  const navigate = useNavigate();

  const clear = useCallback(async () => {
    await session.flush();
    const prev = clearDevelop();
    developSelection.set({ ids: new Set(), anchor: null });
    navigate("/develop");
    toast({
      message: "Filmstrip cleared",
      // `/develop` reopens the photo that was open (`lastDevelopPhoto`).
      undo: () => restoreDevelop(prev),
    });
  }, [session, navigate]);

  const remove = useCallback(
    async (targets: number[]) => {
      await session.flush();
      const all = await everyPhoto(developSourceStore.get());
      const prev = removeFromDevelop(targets, all);
      developSelection.set({ ids: new Set(), anchor: null });
      if (targets.includes(assetId)) {
        // The next photo along the strip that stays, else the one before, else nothing.
        const gone = new Set(targets);
        const i = shown.indexOf(assetId);
        const next = shown.slice(i + 1).find((id) => !gone.has(id)) ?? shown.slice(0, Math.max(0, i)).reverse().find((id) => !gone.has(id));
        navigate(next !== undefined ? `/develop/${next}` : "/develop");
      }
      toast({
        message: targets.length === 1 ? "Removed from the filmstrip" : `Removed ${targets.length.toLocaleString()} from the filmstrip`,
        undo: () => {
          restoreDevelop(prev);
          navigate(`/develop/${assetId}`);
        },
      });
    },
    [session, assetId, shown, navigate],
  );

  return { clear, remove };
}
