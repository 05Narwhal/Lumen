import { WindowChrome } from "@/components/window-chrome";
import { Button, Modal } from "@mantine/core";
import { useEffect, useMemo, useState } from "react";
import { PhotoGrid } from "@/components/photo-grid";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { clearSelection, selectionStore, useSelection } from "@/hooks/use-selection";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import { NO_FILTER, PickerFilters } from "./filters";
import "./styles.scss";

/**
 * The library grid in a modal, in selection mode: "Add photos" for an album, "Choose
 * photos" for a new slideshow. Uses the shared selection store, so it is cleared on open
 * and on close.
 */
export function PhotoPicker({
  title = "Add photos",
  confirmLabel = "Add",
  onClose,
  onPick,
  filters = false,
  scope: within,
}: {
  /** Show *RAW only* and the device filter above the grid (Develop's pickers). */
  filters?: boolean;
  /** Pick from these photos rather than the whole library (an album's cover, Round 4G). */
  scope?: LayoutScope;
  title?: string;
  confirmLabel?: string;
  onClose: () => void;
  onPick: (ids: number[]) => void;
}) {
  const { count } = useSelection();
  const [filter, setFilter] = useState(NO_FILTER);
  const scope = useMemo<LayoutScope>(() => {
    if (within) return within;
    if (!filters) return LIBRARY_SCOPE;
    // Develop wants every RAW, including those the library hides behind their JPEG.
    return {
      ...LIBRARY_SCOPE,
      query: { ...EMPTY_QUERY, includeRawPairs: true, kind: filter.rawOnly ? "raw" : null, deviceIds: filter.deviceIds },
    };
  }, [filter, filters, within]);
  useEffect(() => {
    clearSelection();
    return () => clearSelection();
  }, []);
  return (
    <Modal opened onClose={onClose} title={title} fullScreen classNames={{ content: "photo-picker", body: "photo-picker__body", header: "photo-picker__header" }}>
      <WindowChrome overModal />
      <div className="photo-picker__grid">
        <PhotoGrid rowHeight={160} scope={scope} onOpen={(id) => selectionStore.set((s) => ({ ids: new Set([...s.ids, id]), anchor: id }))} />
      </div>
      <div className="photo-picker__actions">
        {filters && <PickerFilters value={filter} onChange={setFilter} />}
        <span className="photo-picker__count">{count ? `${count} selected` : "Click photos to select them"}</span>
        <Button variant="default" onClick={onClose}>
          Cancel
        </Button>
        <Button disabled={count === 0} onClick={() => onPick([...selectionStore.get().ids])}>
          {confirmLabel} {count || ""}
        </Button>
      </div>
    </Modal>
  );
}
