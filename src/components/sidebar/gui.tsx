import {
  IconAperture,
  IconBrandApple,
  IconAlbum,
  IconChevronDown,
  IconChevronLeft,
  IconClock,
  IconUsers,
  IconDeviceSdCard,
  IconDeviceUsb,
  IconDownload,
  IconEyeOff,
  IconFolder,
  IconHome,
  IconLock,
  IconMap,
  IconPhoto,
  IconSettings,
  IconTrash,
} from "@tabler/icons-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState, type DragEvent } from "react";
import type React from "react";
import { NavLink, useLocation, useNavigate } from "react-router";
import { ActivityRing } from "@/components/activity-ring";
import { useContextMenu } from "@/components/context-menu";
import { connectedPath, useConnectedMenu } from "@/hooks/use-connected-menu";
import { useAlbumMenu, useCollectionMenu, usePrimaryMenu } from "@/hooks/use-library-menus";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { contentsOf } from "@/lib/album-collections";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { deviceMenu } from "@/lib/menus";
import { DRAG_MIME } from "@/components/photo-grid";
import { useAlbums } from "@/hooks/use-albums";
import { useDevices, useVolumes } from "@/hooks/use-devices";
import { useIphones } from "@/hooks/use-iphones";
import { useMacDisks } from "@/hooks/use-mac-disks";
import type { DeviceGroup } from "@/types/generated/DeviceGroup";
import { appleDeviceIcon, deviceIcon } from "@/lib/device-icons";
import { BrandMark } from "@/components/brand-mark";
import { groupDevices } from "./device-groups";
import "./styles.scss";
import { useWindowDrag } from "@/hooks/use-window-drag";

export interface SidebarDevice {
  id: number;
  name: string;
  /** `DEVICE_KIND_*` from use-devices; unknown when absent. */
  kind?: number;
  /** What kind of thing it is, for gathering devices of one kind under one row. */
  group?: DeviceGroup;
}

export { deviceIcon };

type Icon = typeof IconHome;
interface NavItem {
  to: string;
  label: string;
  icon: Icon;
  end?: boolean;
  /** Path prefixes that light this item too: a slideshow's editor belongs under Memories. */
  also?: string[];
}

const primary: NavItem[] = [
  { to: "/", label: "Home", icon: IconHome, end: true },
  { to: "/library", label: "Library", icon: IconPhoto, end: true },
  { to: "/albums", label: "Albums", icon: IconAlbum },
  { to: "/map", label: "Map", icon: IconMap },
  { to: "/memories", label: "Memories", icon: IconClock, also: ["/slideshows"] },
  { to: "/people", label: "People & Pets", icon: IconUsers, also: ["/pets"] },
  { to: "/hidden", label: "Hidden", icon: IconLock },
];

const footer: NavItem[] = [
  { to: "/import", label: "Import", icon: IconDownload },
  { to: "/develop", label: "Develop", icon: IconAperture },
  { to: "/trash", label: "Trash", icon: IconTrash },
  { to: "/settings", label: "Settings", icon: IconSettings },
];

const MAX_SIDEBAR_ALBUMS = 6;
/** Devices shown before their list scrolls on its own (36 px rows). */
export const MAX_DEVICE_ROWS = 5;
/**
 * Round 20: Devices stay out of the sidebar until it can be customised; the section's code is
 * kept. An object rather than a bare constant only so the tests of that code can switch it on.
 */
export const SHOW_DEVICES = { value: false };

function readList(key: string): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/**
 * A set of ids remembered per viewer: the sections folded away (open unless listed) and the
 * device groups expanded (closed unless listed).
 */
function useToggles(key: string, openUnlessListed: boolean) {
  const [listed, setListed] = useState<string[]>(() => readList(key));
  const toggle = (id: string) =>
    setListed((f) => {
      const next = f.includes(id) ? f.filter((x) => x !== id) : [...f, id];
      try {
        localStorage.setItem(key, JSON.stringify(next));
      } catch {
        // Private windows refuse; the toggle still works until the app closes.
      }
      return next;
    });
  return { isOpen: (id: string) => listed.includes(id) !== openUnlessListed, toggle };
}
const useFolded = () => useToggles("lumen.sidebar.folded", true);

/**
 * A titled group with a chevron that folds it away. In the rail there is no room for a
 * title, so the items always show.
 */
/** A section's fold: the shell's standard ease, a little quicker than a page. */
const FOLD = { duration: 0.2, ease: [0.2, 0.8, 0.2, 1] as const };

function Section({
  id,
  title,
  collapsed,
  folds,
  children,
}: {
  id: string;
  title: string;
  collapsed: boolean;
  folds: ReturnType<typeof useFolded>;
  children: React.ReactNode;
}) {
  const open = collapsed || folds.isOpen(id);
  return (
    <div className="sidebar__group">
      {!collapsed && (
        <button type="button" className="sidebar__section" aria-expanded={open} onClick={() => folds.toggle(id)}>
          <span>{title}</span>
          <IconChevronDown size={14} className="sidebar__chevron" aria-hidden />
        </button>
      )}
      {/* Height and fade, so the list below slides up instead of jumping (Round 4A).
          Reduced motion is honoured by the app's MotionConfig. */}
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            key="fold"
            className="sidebar__fold"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={FOLD}
          >
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function Item({
  to,
  label,
  icon: Icon,
  end,
  collapsed,
  onDropIds,
  onContextMenu,
  editing,
  onRename,
  badge,
  nested,
  also,
}: NavItem & {
  collapsed: boolean;
  onDropIds?: (ids: number[]) => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  /** When true the label is an input; `onRename(null)` cancels, `onRename(name)` commits. */
  editing?: boolean;
  onRename?: (name: string | null) => void;
  /** Count shown on the right, e.g. how many items on a card are not imported yet. */
  badge?: number;
  /** One device of an expanded group, indented under it. */
  nested?: boolean;
}) {
  const [over, setOver] = useState(false);
  const [draft, setDraft] = useState(label);
  useEffect(() => {
    if (editing) setDraft(label);
  }, [editing, label]);
  const accepts = (e: DragEvent) => !!onDropIds && e.dataTransfer.types.includes(DRAG_MIME);
  // A link with a query (a collection, `/albums?c=7`) is active only on that query: the router
  // matches the path alone, which would light every collection on the Albums page.
  const { search, pathname } = useLocation();
  const alsoHere = !!also?.some((p) => pathname === p || pathname.startsWith(`${p}/`));
  const query = to.includes("?") ? to.slice(to.indexOf("?")) : null;
  const matches = (isActive: boolean) => {
    if (alsoHere) return true;
    if (!isActive || query == null) return isActive;
    const here = new URLSearchParams(search);
    return [...new URLSearchParams(query)].every(([k, v]) => here.get(k) === v);
  };
  return (
    <NavLink
      to={to}
      end={end || query != null}
      className={({ isActive }) => (matches(isActive) ? "sidebar__item active" : "sidebar__item")}
      data-nested={nested || undefined}
      aria-label={label}
      title={collapsed ? label : undefined}
      data-drop-over={over || undefined}
      onContextMenu={onContextMenu}
      onDragOver={(e) => {
        if (!accepts(e)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        setOver(false);
        if (!accepts(e)) return;
        e.preventDefault();
        try {
          const ids = JSON.parse(e.dataTransfer.getData(DRAG_MIME)) as number[];
          if (ids.length) onDropIds?.(ids);
        } catch {
          /* not our payload */
        }
      }}
    >
      {({ isActive }: { isActive: boolean }) => (
        <>
          {matches(isActive) && <motion.span layoutId="sidebar-indicator" className="sidebar__indicator" aria-hidden transition={spring} />}
          <Icon size={18} stroke={1.75} className="sidebar__icon" />
          {!collapsed && editing ? (
            <input
              className="sidebar__rename"
              aria-label="Device name"
              autoFocus
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onClick={(e) => e.preventDefault()}
              onBlur={() => onRename?.(draft.trim() && draft.trim() !== label ? draft.trim() : null)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onRename?.(draft.trim() && draft.trim() !== label ? draft.trim() : null);
                if (e.key === "Escape") onRename?.(null);
              }}
            />
          ) : (
            !collapsed && <span className="sidebar__label">{label}</span>
          )}
          {badge != null && badge > 0 && (
            <span className="sidebar__badge" aria-label={`${badge} new`}>
              {badge > 999 ? "999+" : badge}
            </span>
          )}
        </>
      )}
    </NavLink>
  );
}

const spring = { type: "spring", stiffness: 420, damping: 38, mass: 0.9 } as const;

export function Sidebar({
  collapsed,
  onToggle,
  devices,
}: {
  collapsed: boolean;
  onToggle: () => void;
  devices: SidebarDevice[];
}) {
  const { albums, addTo } = useAlbums();
  const { collections: folders } = useAlbumCollections();
  const collectionMenus = useCollectionMenu();
  // The top level: collections, then albums in none, `MAX_SIDEBAR_ALBUMS` rows in all.
  const top = contentsOf(folders, albums, null);
  const topCollections = top.collections.slice(0, MAX_SIDEBAR_ALBUMS);
  const shownAlbums = top.albums.slice(0, Math.max(0, MAX_SIDEBAR_ALBUMS - topCollections.length));
  const { rename: renameDevice, setHidden } = useDevices();
  const { open: openMenu } = useContextMenu();
  const navigate = useNavigate();
  const primaryMenu = usePrimaryMenu();
  const albumMenus = useAlbumMenu();
  const album = (a: AlbumSummary, nested = false) => (
    <Item
      key={a.id}
      to={`/albums/${a.id}`}
      label={a.name}
      icon={a.hidesItems ? IconEyeOff : IconAlbum}
      collapsed={collapsed}
      nested={nested}
      onDropIds={(ids) => void addTo({ albumId: a.id, assetIds: ids, albumName: a.name })}
      onContextMenu={(e) => openMenu(e, albumMenus.menuFor(a))}
    />
  );
  const connectedMenuFor = useConnectedMenu();
  const [renaming, setRenaming] = useState<number | null>(null);
  const folds = useFolded();
  const expanded = useToggles("lumen.sidebar.expanded", false);
  const deviceRows = groupDevices(devices);
  const shownDeviceRows = deviceRows.reduce(
    (n, row) => n + 1 + (row.kind === "group" && expanded.isOpen(row.group) ? row.devices.length : 0),
    0,
  );
  // Cards, external disks and cameras the watcher can see right now, with how many items
  // on each have not been imported yet. The volume watcher pushes `device://volumes`
  // whether or not the Import screen is open, so this stays live from anywhere in the app.
  const { volumes } = useVolumes();
  // Deliberately not polling from here: a usbmuxd round-trip on every tick is the cost
  // the Import and Home screens opt into. Whatever they last saw still shows up.
  const { devices: iphones } = useIphones();
  // Mac-formatted disks Windows cannot mount, read through their partition (device://macdisks).
  const { disks: macDisks } = useMacDisks();
  const sources = [
    ...volumes.map((v) => ({
      source: { kind: "volume" as const, id: v.mount, label: v.label || v.mount, key: v.key },
      icon: v.removable ? IconDeviceSdCard : IconDeviceUsb,
      newItems: v.newItems,
    })),
    ...macDisks.map((d) => ({
      source: { kind: "mac" as const, id: d.id, label: d.name },
      icon: IconBrandApple,
      newItems: undefined as number | undefined,
    })),
    ...iphones.map((d) => ({
      source: { kind: "iphone" as const, id: d.udid, label: d.name },
      icon: appleDeviceIcon(d.model),
      newItems: undefined as number | undefined,
    })),
  ].map((c) => ({ ...c, to: connectedPath(c.source), label: c.source.label }));

  const drag = useWindowDrag();
  return (
    <nav className="sidebar" aria-label="Primary" data-collapsed={collapsed}>
      {albumMenus.element}
      {collectionMenus.element}
      <div className="sidebar__top" {...drag}>
        {!collapsed && (
          <span className="sidebar__brand">
            <BrandMark size={16} />
            Lumen
          </span>
        )}
        <button
          type="button"
          className="sidebar__toggle"
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          onClick={onToggle}
        >
          <IconChevronLeft size={16} className="sidebar__toggle-icon" />
        </button>
      </div>
      <div className="sidebar__scroll">
        <div className="sidebar__group">
          {primary.map((i) => (
            <Item key={i.to} {...i} collapsed={collapsed} onContextMenu={(e) => openMenu(e, primaryMenu(i.to))} />
          ))}
        </div>
        {(topCollections.length > 0 || shownAlbums.length > 0) && (
          <Section id="albums" title="Albums" collapsed={collapsed} folds={folds}>
            {/* Collections first (Round 5A), each folding open on what it holds. */}
            {topCollections.map((c) => {
              const open = expanded.isOpen(`collection-${c.id}`);
              const held = contentsOf(folders, albums, c.id);
              return (
                <div key={`c${c.id}`} className="sidebar__device-group">
                  <div className="sidebar__group-row">
                    <Item to={`/albums?c=${c.id}`} label={c.name} icon={IconFolder} collapsed={collapsed} onContextMenu={(e) => openMenu(e, collectionMenus.menuFor(c))} />
                    {!collapsed && (held.albums.length > 0 || held.collections.length > 0) && (
                      <button
                        type="button"
                        className="sidebar__expand"
                        aria-label={`Show what is in ${c.name}`}
                        aria-expanded={open}
                        onClick={() => expanded.toggle(`collection-${c.id}`)}
                      >
                        <IconChevronDown size={14} className="sidebar__chevron" aria-hidden />
                      </button>
                    )}
                  </div>
                  {open &&
                    !collapsed &&
                    held.collections.map((k) => (
                      <Item key={`c${k.id}`} to={`/albums?c=${k.id}`} label={k.name} icon={IconFolder} collapsed={collapsed} nested onContextMenu={(e) => openMenu(e, collectionMenus.menuFor(k))} />
                    ))}
                  {open && !collapsed && held.albums.map((a) => album(a, true))}
                </div>
              );
            })}
            {shownAlbums.map((a) => album(a))}
          </Section>
        )}
        {SHOW_DEVICES.value && devices.length > 0 && (
          <Section id="devices" title="Devices" collapsed={collapsed} folds={folds}>
            <div className="sidebar__list" data-scroll={shownDeviceRows > MAX_DEVICE_ROWS ? "true" : undefined}>
              {deviceRows.map((row) => {
                const device = (d: SidebarDevice, nested = false) => (
                  <Item
                    key={d.id}
                    to={`/library/${d.id}`}
                    label={d.name}
                    icon={deviceIcon(d.kind, d.group)}
                    collapsed={collapsed}
                    nested={nested}
                    editing={renaming === d.id}
                    onRename={(name) => {
                      setRenaming(null);
                      if (name) void renameDevice({ id: d.id, name });
                    }}
                    onContextMenu={(e) =>
                      openMenu(
                        e,
                        deviceMenu({
                          open: () => navigate(`/library/${d.id}`),
                          rename: () => setRenaming(d.id),
                          hide: () => void setHidden({ id: d.id, hidden: true }),
                        }),
                      )
                    }
                  />
                );
                if (row.kind === "device") return device(row.device);
                const open = expanded.isOpen(row.group);
                return (
                  <div key={`group-${row.group}`} className="sidebar__device-group">
                    <div className="sidebar__group-row">
                      <Item
                        to={`/library/group/${row.group}`}
                        label={row.label}
                        icon={deviceIcon(row.devices[0].kind, row.group)}
                        collapsed={collapsed}
                        badge={undefined}
                      />
                      {!collapsed && (
                        <button
                          type="button"
                          className="sidebar__expand"
                          aria-label={`Show each of ${row.label}`}
                          aria-expanded={open}
                          onClick={() => expanded.toggle(row.group)}
                        >
                          <IconChevronDown size={14} className="sidebar__chevron" aria-hidden />
                        </button>
                      )}
                    </div>
                    {open && !collapsed && row.devices.map((d) => device(d, true))}
                  </div>
                );
              })}
            </div>
          </Section>
        )}
        {sources.length > 0 && (
          <Section id="connected" title="Connected" collapsed={collapsed} folds={folds}>
            {sources.map((c) => (
              <Item key={c.to} to={c.to} label={c.label} icon={c.icon} collapsed={collapsed} badge={c.newItems} onContextMenu={(e) => openMenu(e, connectedMenuFor(c.source))} />
            ))}
          </Section>
        )}
      </div>
      {/* Always in reach: the list above scrolls and fades out under it. */}
      <div className="sidebar__footer-nav">
        <div className="sidebar__group">
          {footer.map((i) => (
            <Item key={i.to} {...i} collapsed={collapsed} onContextMenu={(e) => openMenu(e, primaryMenu(i.to))} />
          ))}
        </div>
        <div className="sidebar__footer">
          <ActivityRing compact={collapsed} />
        </div>
      </div>
    </nav>
  );
}
