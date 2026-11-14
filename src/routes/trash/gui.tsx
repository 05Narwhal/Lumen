import { Button } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { IconRestore, IconTrash, IconTrashX } from "@tabler/icons-react";
import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { PhotoGrid } from "@/components/photo-grid";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { useSelection } from "@/hooks/use-selection";
import { useSettings } from "@/hooks/use-settings";
import { useTrash } from "@/hooks/use-trash";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { ROW_HEIGHTS } from "@/routes/library";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { LayoutSummary } from "@/types/generated/LayoutSummary";
import "./styles.scss";

const TRASH_SCOPE: LayoutScope = { ...LIBRARY_SCOPE, trash: true };

export function TrashRoute() {
  const { settings } = useSettings();
  const { restore, deleteNow, emptyTrash } = useTrash();
  const { ids, count } = useSelection();
  const [, setSearchParams] = useSearchParams();
  // The count comes from a cheap layout summary at a fixed width; the grid has its own.
  const summary = useQuery({
    queryKey: queryKeys.layout(1000, 200, 4, TRASH_SCOPE),
    queryFn: () => invoke<LayoutSummary>("get_layout", { width: 1000, rowHeight: 200, gap: 4, scope: TRASH_SCOPE }),
  });
  const total = summary.data?.itemCount ?? 0;
  const tileSize = settings?.appearance.tileSize ?? "m";
  const scope = useMemo(() => TRASH_SCOPE, []);
  const onOpen = useCallback(
    (id: number) =>
      setSearchParams((p) => {
        p.set("asset", String(id));
        return p;
      }),
    [setSearchParams],
  );
  const extraMenuItems = useCallback(
    (sel: number[]) => [
      { label: sel.length === 1 ? "Restore" : `Restore ${sel.length}`, icon: <IconRestore size={16} />, onSelect: () => void restore(sel) },
      { label: "Delete now", icon: <IconTrashX size={16} />, danger: true, onSelect: () => void deleteNow(sel) },
    ],
    [restore, deleteNow],
  );

  return (
    <div className="trash-route">
      <PageHeader
        title="Trash"
        search={false}
        toolbar={
          total > 0 && (
            <>
              <span className="trash-route__meta">Items are deleted after 30 days</span>
              <Button size="xs" variant="default" leftSection={<IconRestore size={14} />} disabled={count === 0} onClick={() => void restore([...ids])}>
                Restore
              </Button>
              <Button size="xs" variant="default" color="red" leftSection={<IconTrashX size={14} />} disabled={count === 0} onClick={() => void deleteNow([...ids])}>
                Delete now
              </Button>
              <Button size="xs" color="red" onClick={() => void emptyTrash(total)}>
                Empty trash
              </Button>
            </>
          )
        }
      />
      {summary.data && total === 0 ? (
        <EmptyState icon={<IconTrash />} title="Trash is empty" description="Deleted items stay here for 30 days before they are removed." />
      ) : (
        <div className="trash-route__grid">
          <PhotoGrid rowHeight={ROW_HEIGHTS[tileSize]} scope={scope} onOpen={onOpen} extraMenuItems={extraMenuItems} />
        </div>
      )}
    </div>
  );
}
