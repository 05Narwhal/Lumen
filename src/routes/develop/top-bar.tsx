import { Menu, Tooltip } from "@mantine/core";
import { IconArrowBackUp, IconArrowForwardUp, IconChevronDown, IconClipboardCopy, IconClipboardText, IconFileExport, IconFilter, IconPhotoPlus, IconRefresh } from "@tabler/icons-react";
import { useState } from "react";
import { AiChip } from "@/components/ai-chip";
import { TitlebarPortal } from "@/components/app-shell";
import { PhotoPicker } from "@/components/photo-picker";
import { openExportSheet } from "@/components/export-sheet";
import { MarkFilter } from "@/components/mark-filter";
import { StarRating } from "@/components/star-rating";
import { isMac } from "@/components/window-controls";
import { useAsset } from "@/hooks/use-asset";
import { stripIdsWhere, useStripIds } from "@/hooks/use-develop-strip";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { developSelection } from "@/lib/develop";
import { addToDevelop, developSourceStore, withFilters } from "@/lib/develop-source";
import { FLAG_PICK, FLAG_REJECT, LABELS } from "@/lib/marks";
import { useStripEdits } from "./strip-edits";
import type { StripSort } from "@/types/generated/StripSort";
import { useWindowDrag } from "@/hooks/use-window-drag";

const SORTS: { value: StripSort; label: string }[] = [
  { value: "captured", label: "Capture time" },
  { value: "rating", label: "Rating" },
  { value: "filename", label: "File name" },
];

const photos = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);

/**
 * Develop's top bar (spec §1, revised): the source and its count, selection, the mark filter
 * and minimum rating that narrow the filmstrip, its order, and what acts on the selection.
 * It is drawn into the title bar's row (Round 4J); on a narrow window the filters and the
 * order fold into one *Filter* menu.
 */
export function TopBar({
  session,
  assetId,
  clip,
}: {
  session: DevelopSession;
  assetId: number;
  clip: { openCopy: () => void; paste: () => Promise<void>; sync: () => void };
}) {
  const source = developSourceStore.use();
  const sel = developSelection.use();
  const ids = useStripIds(source).data ?? [];
  const query = source.scope.query ?? EMPTY_QUERY;
  const [adding, setAdding] = useState(false);
  const mod = isMac() ? "⌘" : "Ctrl+";
  const select = (list: number[]) => developSelection.set({ ids: new Set(list), anchor: list[0] ?? null });
  const selectWhere = async (extra: Parameters<typeof stripIdsWhere>[1]) => select(await stripIdsWhere(source, extra));
  const selectedCount = sel.ids.size;
  const exportIds = selectedCount > 0 ? [...sel.ids] : [assetId];
  const sortLabel = SORTS.find((s) => s.value === source.sort)?.label ?? "Capture time";
  const edits = useStripEdits(session, assetId, ids);
  const filename = useAsset(assetId).data?.filename;
  const position = ids.indexOf(assetId) + 1;

  const filters = (
    <>
      <MarkFilter query={query} onChange={(patch) => developSourceStore.set((s) => withFilters(s, patch))} />
      <StarRating
        value={query.minRating ?? 0}
        onChange={(v) => developSourceStore.set((s) => withFilters(s, { minRating: v === (query.minRating ?? 0) ? null : v }))}
      />
    </>
  );
  const sortItems = SORTS.map((s) => (
    <Menu.Item key={s.value} onClick={() => developSourceStore.set((x) => ({ ...x, sort: s.value }))} data-active={s.value === source.sort}>
      {s.label}
    </Menu.Item>
  ));
  const exportLabel = selectedCount > 1 ? `Export ${selectedCount.toLocaleString()}…` : "Export…";
  const narrowed = query.flag != null || (query.colorLabels?.length ?? 0) > 0 || (query.minRating ?? 0) > 0;

  const drag = useWindowDrag();
  return (
    <TitlebarPortal>
    <header className="develop__top" aria-label="Develop" {...drag}>
      <span className="develop__source">{source.label}</span>
      <span className="develop__count">{selectedCount > 0 ? `${selectedCount.toLocaleString()} of ${photos(ids.length)}` : photos(ids.length)}</span>
      {filename && (
        <span className="develop__current" data-testid="develop-current" title={filename}>
          {filename}
          {position > 0 && ` · ${position.toLocaleString()} of ${ids.length.toLocaleString()}`}
        </span>
      )}
      {source.only !== null && (
        <Tooltip label="Add photos to the filmstrip" openDelay={500} withArrow>
          <button type="button" className="develop__icon" aria-label="Add photos…" onClick={() => setAdding(true)}>
            <IconPhotoPlus size={16} />
          </button>
        </Tooltip>
      )}
      {adding && (
        <PhotoPicker
          title="Add photos to develop"
          filters
          onClose={() => setAdding(false)}
          onPick={(picked) => {
            setAdding(false);
            addToDevelop(picked);
          }}
        />
      )}

      <Menu position="bottom-start" shadow="md" withinPortal>
        <Menu.Target>
          <button type="button" className="develop__menu-btn">
            Select <IconChevronDown size={12} />
          </button>
        </Menu.Target>
        {/* The button sits at the window's right edge: the label submenu opens to the left, and
            the dropdown must not clip it (globals give dropdowns `overflow-y: auto`). */}
        <Menu.Dropdown className="develop__select-menu">
          <Menu.Item onClick={() => select(ids)} rightSection={`${mod}A`}>
            Select all
          </Menu.Item>
          <Menu.Item onClick={() => select([])} rightSection={`${mod}D`}>
            Select none
          </Menu.Item>
          <Menu.Item onClick={() => select(ids.filter((i) => !sel.ids.has(i)))}>Invert selection</Menu.Item>
          <Menu.Divider />
          <Menu.Item onClick={() => void selectWhere({ flag: FLAG_PICK })}>Select picked</Menu.Item>
          <Menu.Item onClick={() => void selectWhere({ flag: FLAG_REJECT })}>Select rejected</Menu.Item>
          <Menu.Sub position="right-start" offset={6}>
            <Menu.Sub.Target>
              <Menu.Sub.Item>Select by label</Menu.Sub.Item>
            </Menu.Sub.Target>
            <Menu.Sub.Dropdown>
              {LABELS.map((l) => (
                <Menu.Item key={l.value} leftSection={<span className="develop__label-dot" style={{ background: l.color }} />} onClick={() => void selectWhere({ colorLabels: [l.value] })}>
                  {l.name}
                </Menu.Item>
              ))}
            </Menu.Sub.Dropdown>
          </Menu.Sub>
          <Menu.Divider />
          {selectedCount > 0 && (
            <Menu.Item onClick={() => void edits.remove([...sel.ids])}>
              {selectedCount === 1 ? "Remove from filmstrip" : `Remove ${selectedCount.toLocaleString()} from filmstrip`}
            </Menu.Item>
          )}
          <Menu.Item onClick={() => void edits.clear()}>Clear filmstrip</Menu.Item>
        </Menu.Dropdown>
      </Menu>

      <div className="develop__filters">{filters}</div>

      <Menu position="bottom-start" shadow="md" withinPortal>
        <Menu.Target>
          <button type="button" className="develop__menu-btn develop__sort" aria-label={`Sort by ${sortLabel}`}>
            {sortLabel} <IconChevronDown size={12} />
          </button>
        </Menu.Target>
        <Menu.Dropdown>{sortItems}</Menu.Dropdown>
      </Menu>

      <Menu position="bottom-start" shadow="md" withinPortal closeOnItemClick={false}>
        <Menu.Target>
          <button type="button" className="develop__menu-btn develop__filter-menu" aria-pressed={narrowed}>
            <IconFilter size={14} /> Filter <IconChevronDown size={12} />
          </button>
        </Menu.Target>
        <Menu.Dropdown>
          <div className="develop__filter-pane">{filters}</div>
          <Menu.Divider />
          <Menu.Label>Order</Menu.Label>
          {sortItems}
        </Menu.Dropdown>
      </Menu>

      <div className="develop__drag" />

      {selectedCount > 1 && (
        <Tooltip label="Give the selection every setting of this photo" openDelay={500} withArrow>
          <button type="button" className="develop__text-btn" aria-label="Sync settings" onClick={clip.sync}>
            <IconRefresh size={15} /> <span className="develop__sync-label">Sync settings</span>
          </button>
        </Tooltip>
      )}
      <Tooltip label={`Copy settings… (${mod}Shift+C)`} openDelay={500} withArrow>
        <button type="button" className="develop__icon" aria-label="Copy settings…" onClick={clip.openCopy}>
          <IconClipboardCopy size={18} />
        </button>
      </Tooltip>
      <Tooltip label={`Paste settings (${mod}Shift+V)`} openDelay={500} withArrow>
        <button type="button" className="develop__icon" aria-label="Paste settings" onClick={() => void clip.paste()}>
          <IconClipboardText size={18} />
        </button>
      </Tooltip>
      <AiChip />
      <Tooltip label={`Undo (${mod}Z)`} openDelay={500} withArrow>
        <button type="button" className="develop__icon" aria-label="Undo" disabled={!session.canUndo} onClick={session.undo}>
          <IconArrowBackUp size={18} />
        </button>
      </Tooltip>
      <Tooltip label={`Redo (${mod}Shift+Z)`} openDelay={500} withArrow>
        <button type="button" className="develop__icon" aria-label="Redo" disabled={!session.canRedo} onClick={session.redo}>
          <IconArrowForwardUp size={18} />
        </button>
      </Tooltip>
      <button
        type="button"
        className="develop__text-btn develop__text-btn--primary"
        aria-label={exportLabel}
        onClick={() => void session.flush().then(() => openExportSheet(exportIds))}
      >
        <IconFileExport size={15} /> <span className="develop__export-label">{exportLabel}</span>
      </button>
    </header>
    </TitlebarPortal>
  );
}
