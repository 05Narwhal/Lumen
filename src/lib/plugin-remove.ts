import { toast } from "@/components/toast";
import { isAppError } from "@/lib/ipc";
import { removePlugin, undoRemovePlugin } from "@/lib/plugins";

/** Removes a plugin, then offers the 6 s undo toast; `reload` refreshes the lists, `after` runs once removed (a page leaves itself). */
export function removePluginWithUndo(p: { id: string; name: string }, reload: () => void, after?: () => void): Promise<unknown> {
  return removePlugin(p.id).then(
    (token) => {
      reload();
      after?.();
      toast({
        message: `Removed ${p.name}`,
        undo: () => void undoRemovePlugin(token).catch((e) => toast({ message: isAppError(e) ? e.message : "Could not bring it back." })).finally(reload),
      });
    },
    (e) => toast({ message: isAppError(e) ? e.message : "Could not remove that plugin." }),
  );
}
