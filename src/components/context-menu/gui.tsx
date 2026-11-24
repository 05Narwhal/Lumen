import { type FloatingPosition, Menu } from "@mantine/core";
import { IconStar, IconStarFilled } from "@tabler/icons-react";
import { useCallback, useEffect, useState, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { createStore } from "@/lib/store";
import { editableAt, pastable, snapshot, systemClipboard, textMenu } from "./text-menu";
import "./styles.scss";

export type MenuItem =
  | { divider: true }
  | { label: string; icon?: ReactNode; iconUrl?: string; shortcut?: string; danger?: boolean; disabled?: boolean; onSelect: () => void }
  | { label: string; icon?: ReactNode; iconUrl?: string; children: MenuItem[] }
  /** Five stars in one row: click or drag across them; clicking the current rating clears it. */
  | { label: string; rating: number; onRate: (stars: number) => void };

/** A menu factory's output. `dense` (Round 24 §3) asks the renderer for tighter rows, for that menu only. */
export type MenuItems = MenuItem[] & { dense?: boolean };

interface MenuState {
  open: boolean;
  x: number;
  y: number;
  items: MenuItems;
  /** Opened from the keyboard: focus goes back here when the menu closes. */
  returnTo: HTMLElement | null;
}

const menuStore = createStore<MenuState>({ open: false, x: 0, y: 0, items: [], returnTo: null });

/** Set while a keyboard-made `contextmenu` event is being dispatched (see `openMenuFor`). */
let keyboardTarget: HTMLElement | null = null;

export function openContextMenu(x: number, y: number, items: MenuItems) {
  menuStore.set({ open: true, x, y, items, returnTo: keyboardTarget });
}

export const closeContextMenu = () => {
  const { returnTo, open } = menuStore.get();
  menuStore.set((s) => ({ ...s, open: false, returnTo: null }));
  if (open && returnTo?.isConnected) returnTo.focus({ preventScroll: true });
};

/** Where a menu opened from the keyboard appears: just inside the element's lower-left corner. */
const cornerOf = (el: HTMLElement) => {
  const r = el.getBoundingClientRect();
  return { x: r.left + 8, y: r.bottom - 8 };
};

/**
 * Opens `el`'s own menu as a right-click would, at its corner: a `contextmenu` event on it,
 * so every surface's existing `onContextMenu` serves the keyboard too (Shift+F10, the
 * context-menu key). Returns whether anything opened.
 */
export function openMenuFor(el: HTMLElement): boolean {
  const { x, y } = cornerOf(el);
  keyboardTarget = el;
  try {
    el.dispatchEvent(new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 }));
  } finally {
    keyboardTarget = null;
  }
  return menuStore.get().open;
}

/** Shift+F10 or the context-menu key, on whatever has focus. */
const isMenuKey = (e: KeyboardEvent) => e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey);

/** `onContextMenu={(e) => open(e, items)}` on anything. One host renders the menu. */
export function useContextMenu() {
  const open = useCallback((e: MouseEvent, items: MenuItems) => {
    e.preventDefault();
    e.stopPropagation();
    openContextMenu(e.clientX, e.clientY, items);
  }, []);
  return { open };
}

/** The item's own icon, else an app's picture (Round 22 lane C) at the icon's size. */
const leftOf = (it: { icon?: ReactNode; iconUrl?: string }) =>
  it.icon ?? (it.iconUrl ? <img className="context-menu__app-icon" src={it.iconUrl} alt="" width={16} height={16} draggable={false} /> : undefined);

/**
 * A menu factory's items inside a Mantine `Menu` (the shared host, or a local one such as the
 * Masks panel's): dividers, disabled and danger entries, and submenus. `subPosition` is where
 * a submenu opens (`left-start` for a menu at the window's right edge).
 */
export function Items({ items, subPosition, dense = items.dense }: { items: MenuItems; subPosition?: FloatingPosition; dense?: boolean }) {
  return (
    <>
      {items.map((it, i) => {
        if ("divider" in it) return <Menu.Divider key={i} />;
        if ("onRate" in it) return <StarRow key={i} label={it.label} rating={it.rating} onRate={it.onRate} />;
        if ("children" in it) {
          return (
            <Menu.Sub key={i} position={subPosition}>
              <Menu.Sub.Target>
                <Menu.Sub.Item leftSection={leftOf(it)}>{it.label}</Menu.Sub.Item>
              </Menu.Sub.Target>
              <Menu.Sub.Dropdown className="context-menu__dropdown" data-dense={dense || undefined}>
                <Items items={it.children} subPosition={subPosition} dense={dense} />
              </Menu.Sub.Dropdown>
            </Menu.Sub>
          );
        }
        return (
          <Menu.Item
            key={i}
            leftSection={leftOf(it)}
            rightSection={it.shortcut ? <kbd className="context-menu__kbd">{it.shortcut}</kbd> : undefined}
            color={it.danger ? "red" : undefined}
            disabled={it.disabled}
            onClick={() => {
              closeContextMenu();
              it.onSelect();
            }}
          >
            {it.label}
          </Menu.Item>
        );
      })}
    </>
  );
}

function StarRow({ label, rating, onRate }: { label: string; rating: number; onRate: (n: number) => void }) {
  const [hover, setHover] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);
  const shown = hover ?? rating;
  const starAt = (e: PointerEvent) => {
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-star]");
    return el ? Number(el.dataset.star) : null;
  };
  const commit = (n: number) => {
    closeContextMenu();
    onRate(n === rating ? 0 : n);
  };
  return (
    <div
      className="context-menu__stars"
      role="group"
      aria-label={label}
      onPointerLeave={() => !dragging && setHover(null)}
      onPointerDown={(e) => {
        const n = starAt(e);
        if (n === null) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(true);
        setHover(n);
      }}
      onPointerMove={(e) => {
        const n = starAt(e);
        if (n !== null) setHover(n);
      }}
      onPointerUp={(e) => {
        if (!dragging) return;
        setDragging(false);
        const n = starAt(e) ?? hover;
        if (n !== null) commit(n);
      }}
    >
      <span className="context-menu__stars-label">{label}</span>
      {[1, 2, 3, 4, 5].map((n) => (
        <button
          key={n}
          type="button"
          data-star={n}
          data-on={n <= shown || undefined}
          aria-label={`${n} star${n > 1 ? "s" : ""}`}
          aria-pressed={n === rating}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              commit(n);
            }
          }}
        >
          {n <= shown ? <IconStarFilled size={16} /> : <IconStar size={16} />}
        </button>
      ))}
    </div>
  );
}

/** Mount once in the root layout. */
export function ContextMenuHost() {
  const s = menuStore.use();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isMenuKey(e)) return;
      const el = document.activeElement;
      if (!(el instanceof HTMLElement) || el === document.body) return;
      e.preventDefault();
      openMenuFor(el);
    };
    // A text field anywhere: its edit menu, before (and instead of) any object's menu around it.
    const onMenu = (e: globalThis.MouseEvent) => {
      const el = editableAt(e.target);
      if (!el) return;
      e.preventDefault();
      e.stopPropagation();
      const sel = snapshot(el);
      const clip = systemClipboard();
      const { clientX: x, clientY: y } = e;
      void pastable(clip).then((text) => openContextMenu(x, y, textMenu(el, sel, clip, text)));
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("contextmenu", onMenu, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("contextmenu", onMenu, true);
    };
  }, []);
  return (
    <Menu
      opened={s.open}
      onChange={(o) => !o && closeContextMenu()}
      position="bottom-start"
      offset={2}
      shadow="lg"
      // Grows with its longest label (*Edit in PhotoCraft*) between the bounds in styles.scss.
      width="max-content"
      transitionProps={{ transition: "pop-top-left", duration: 120 }}
      classNames={{ dropdown: "context-menu__dropdown" }}
      withinPortal
    >
      <Menu.Target>
        <span className="context-menu__anchor" style={{ left: s.x, top: s.y }} aria-hidden />
      </Menu.Target>
      <Menu.Dropdown role="menu" data-nested={s.items.some((it) => "children" in it) || undefined} data-dense={s.items.dense || undefined}>
        <Items items={s.items} />
      </Menu.Dropdown>
    </Menu>
  );
}
