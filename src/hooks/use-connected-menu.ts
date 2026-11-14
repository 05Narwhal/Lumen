import { useCallback } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { useVolumes } from "@/hooks/use-devices";
import { useIphones } from "@/hooks/use-iphones";
import { invoke, isAppError } from "@/lib/ipc";
import { connectedMenu, type ConnectedKind, type MenuItem } from "@/lib/menus";

export interface ConnectedSource {
  kind: ConnectedKind;
  /** Mount, UDID or Mac disk id, as the connected page's URL has it. */
  id: string;
  label: string;
  /** A card's `import_seen` key, for *Forget this device*. */
  key?: string;
}

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** `/connected/{kind}/{id}`, with `?browse=1` to open the device browser on arrival. */
export const connectedPath = (s: Pick<ConnectedSource, "kind" | "id">, browse = false) =>
  `/connected/${s.kind}/${encodeURIComponent(s.id)}${browse ? "?browse=1" : ""}`;

/**
 * A Connected entry's menu (Round 4G): open its page, browse it, import from it, and for a
 * card or drive *Eject* and *Forget this device*. Read-only to the device throughout.
 */
export function useConnectedMenu() {
  const navigate = useNavigate();
  const { startImport: importVolume, forget } = useVolumes();
  const { startImport: importIphone } = useIphones();
  return useCallback(
    (s: ConnectedSource): MenuItem[] => {
      const start = async () => {
        try {
          const batch = s.kind === "volume" ? await importVolume(s.id) : await importIphone(s.id);
          if (batch != null) toast({ message: `Importing from ${s.label}` });
        } catch (e) {
          fail(e, "Could not start the import.");
        }
      };
      return connectedMenu(s.kind, {
        open: () => navigate(connectedPath(s)),
        browse: () => navigate(connectedPath(s, true)),
        importNew: s.kind === "mac" ? undefined : () => void start(),
        importAgain:
          s.kind === "volume" && s.key
            ? async () => {
                await forget(s.key!);
                await start();
              }
            : undefined,
        eject:
          s.kind === "volume"
            ? () =>
                void invoke("eject_volume", { mount: s.id }).then(
                  () => toast({ message: `${s.label} can be removed safely` }),
                  (e: unknown) => fail(e, "The drive could not be ejected."),
                )
            : undefined,
        forget: s.kind === "volume" && s.key ? () => void forget(s.key!) : undefined,
      });
    },
    [navigate, importVolume, importIphone, forget],
  );
}
