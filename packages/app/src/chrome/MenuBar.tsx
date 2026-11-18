/**
 * The in-app menu bar (Windows: the document windows have no OS frame, so
 * no native menu bar can show; `apps/desktop/src/TitleBar.tsx` mounts this
 * in its strip). It draws the shell's menu model (`menu_model` /
 * `menu-changed`, defined once in `apps/desktop/src-tauri/src/menu.rs`) and
 * reports the chosen item's id; it holds no labels or structure of its own.
 *
 * Behaviour follows a Windows menu bar and the ARIA menubar pattern: click
 * opens, hovering switches while a menu is open, Alt or F10 moves the focus
 * to the bar and back, arrows / Home / End / Enter / Space / Esc navigate,
 * a letter jumps to the next item starting with it. The pop-ups are
 * `position: fixed` in this document (the document webview fills its
 * window), so they paint over the sheet.
 */
import type { MenuNode } from "@tally/bridge";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { cx } from "../context";
import styles from "./MenuBar.module.css";

/** What the app adds to the shell's `enabled` for an item (the command registry's state). */
export interface MenuItemState {
  enabled: boolean;
  /** Why the item is off (tooltip). */
  reason?: string;
}

export interface MenuBarProps {
  /** The menus of the bar (the top-level submenus of the shell's model). */
  model: readonly MenuNode[];
  /** Accessible name of the bar. */
  label: string;
  /** An item was chosen (the focus is back where it was before the bar took it). */
  onActivate: (id: string) => void;
  /** The app's own state of an item, asked when its menu opens. */
  itemState?: (id: string) => MenuItemState;
}

const selector = (id: string): string => `[data-id=${JSON.stringify(id)}]`;
const choosable = (menu: MenuNode): MenuNode[] => menu.items.filter((n) => n.kind !== "separator");

/** Pure: the item after `from` (wrapping) whose label starts with `char`; -1 when none. */
export function typeAhead(labels: readonly string[], from: number, char: string): number {
  const c = char.toLocaleLowerCase();
  for (let k = 1; k <= labels.length; k++) {
    const i = (from + k) % labels.length;
    if (labels[i]?.toLocaleLowerCase().startsWith(c)) return i;
  }
  return -1;
}

export function MenuBar({ model, label, onActivate, itemState }: MenuBarProps) {
  const tops = model.filter((n) => n.kind === "submenu");
  /** Ids of the open menus: the bar's menu, then its open submenus. */
  const [path, setPath] = useState<string[]>([]);
  /** The bar item in the tab order (roving tabindex). */
  const [current, setCurrent] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  /** An item to focus once it is rendered. */
  const want = useRef<string | null>(null);
  /** What had the focus before the bar took it. */
  const returnTo = useRef<HTMLElement | null>(null);
  const live = useRef({ path, tops, current });
  live.current = { path, tops, current };

  const inside = (el: unknown): boolean => el instanceof Node && !!root.current?.contains(el);

  const focusId = useCallback((id: string): void => {
    const el = root.current?.querySelector<HTMLElement>(selector(id));
    if (el) el.focus();
    else want.current = id;
  }, []);

  useLayoutEffect(() => {
    if (want.current === null) return;
    const el = root.current?.querySelector<HTMLElement>(selector(want.current));
    if (el) {
      want.current = null;
      el.focus();
    }
  });

  // A model change (a document closed, the language) can remove an open menu.
  useEffect(() => {
    if (path.length > 0 && !tops.some((t) => t.id === path[0])) setPath([]);
    if (current >= tops.length && tops.length > 0) setCurrent(0);
  }, [path, tops, current]);

  const remember = useCallback((): void => {
    const el = document.activeElement;
    if (el instanceof HTMLElement && !inside(el)) returnTo.current = el;
    // `inside` reads a ref only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Closes every menu; `restore` gives the focus back to where it was. */
  const leave = useCallback((restore: boolean): void => {
    setPath([]);
    want.current = null;
    if (!restore) return;
    const el = returnTo.current;
    returnTo.current = null;
    if (el?.isConnected) el.focus();
    else if (inside(document.activeElement)) (document.activeElement as HTMLElement).blur();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openTop = (i: number, focus: "first" | "last" | "bar"): void => {
    const n = tops.length;
    const top = tops[((i % n) + n) % n];
    if (!top) return;
    setCurrent(tops.indexOf(top));
    setPath([top.id]);
    const items = choosable(top);
    const target = focus === "first" ? items[0] : focus === "last" ? items[items.length - 1] : undefined;
    focusId(target?.id ?? top.id);
  };

  const focusTop = (i: number): void => {
    const n = tops.length;
    const top = tops[((i % n) + n) % n];
    if (!top) return;
    setCurrent(tops.indexOf(top));
    focusId(top.id);
  };

  const stateOf = (node: MenuNode): MenuItemState => {
    if (!node.enabled) return { enabled: false };
    return node.kind === "submenu" ? { enabled: true } : (itemState?.(node.id) ?? { enabled: true });
  };

  const activate = (node: MenuNode): void => {
    if (!stateOf(node).enabled) return;
    leave(true);
    onActivate(node.id);
  };

  /** The open menu at `level` (1 = the bar's menu). */
  const menuAt = (level: number): MenuNode | undefined => {
    let list: readonly MenuNode[] = tops;
    let found: MenuNode | undefined;
    for (let k = 0; k < level; k++) {
      found = list.find((n) => n.id === path[k]);
      if (!found) return undefined;
      list = found.items;
    }
    return found;
  };

  const onKeyDown = (e: ReactKeyboardEvent): void => {
    // Alt and F10 belong to the window-level handler below.
    if (e.key === "Alt" || e.key === "F10") return;
    const el = e.target instanceof HTMLElement ? e.target.closest<HTMLElement>("[data-id]") : null;
    if (!el) return;
    // While the bar has the focus no key reaches the app's keymap or the grid.
    e.stopPropagation();
    if (e.key === "Tab") {
      e.preventDefault();
      leave(true);
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) {
      e.preventDefault();
      return;
    }
    const id = el.dataset.id ?? "";
    const level = Number(el.dataset.level);
    const topIndex = tops.findIndex((t) => t.id === path[0]);
    e.preventDefault();
    if (level === 0) {
      const i = tops.findIndex((t) => t.id === id);
      const move = (j: number) => (path.length > 0 ? openTop(j, "bar") : focusTop(j));
      if (e.key === "ArrowRight") move(i + 1);
      else if (e.key === "ArrowLeft") move(i - 1);
      else if (e.key === "Home") move(0);
      else if (e.key === "End") move(tops.length - 1);
      else if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") openTop(i, "first");
      else if (e.key === "ArrowUp") openTop(i, "last");
      else if (e.key === "Escape") {
        if (path.length > 0) setPath([]);
        else leave(true);
      } else if (e.key.length === 1) {
        const j = typeAhead(tops.map((t) => t.label), i, e.key);
        if (j >= 0) openTop(j, "first");
      }
      return;
    }
    const menu = menuAt(level);
    if (!menu) return;
    const items = choosable(menu);
    const i = items.findIndex((n) => n.id === id);
    const node = items[i];
    const go = (j: number) => {
      const to = items[(j + items.length) % items.length];
      if (!to) return;
      setPath(path.slice(0, level));
      focusId(to.id);
    };
    const openSub = (sub: MenuNode) => {
      setPath([...path.slice(0, level), sub.id]);
      const first = choosable(sub)[0];
      if (first) focusId(first.id);
    };
    const closeLevel = () => {
      // Back to the item (or bar item) this menu hangs from.
      focusId(menu.id);
      setPath(path.slice(0, level - 1));
    };
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(items.length - 1);
    else if (e.key === "ArrowRight") {
      if (node?.kind === "submenu") openSub(node);
      else openTop(topIndex + 1, "first");
    } else if (e.key === "ArrowLeft") {
      if (level > 1) closeLevel();
      else openTop(topIndex - 1, "first");
    } else if (e.key === "Enter" || e.key === " ") {
      if (node?.kind === "submenu") openSub(node);
      else if (node) activate(node);
    } else if (e.key === "Escape") closeLevel();
    else if (e.key.length === 1) {
      const j = typeAhead(items.map((n) => n.label), i, e.key);
      if (j >= 0) go(j);
    }
  };

  // Alt (pressed and released alone) and F10 move the focus to the bar and
  // back; a click elsewhere or the window losing the focus closes the menus.
  useEffect(() => {
    let altArmed = false;
    const toggle = (): void => {
      if (inside(document.activeElement)) {
        leave(true);
        return;
      }
      // A modal dialog keeps the keyboard.
      if (document.querySelector('[aria-modal="true"]')) return;
      const { tops, current } = live.current;
      const top = tops[current] ?? tops[0];
      if (!top) return;
      remember();
      focusId(top.id);
    };
    const down = (e: KeyboardEvent): void => {
      if (e.key === "F10" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault();
        e.stopPropagation();
        altArmed = false;
        toggle();
        return;
      }
      altArmed = e.key === "Alt" && !e.ctrlKey && !e.shiftKey && !e.metaKey && (altArmed || !e.repeat);
    };
    const up = (e: KeyboardEvent): void => {
      if (e.key !== "Alt" || !altArmed) return;
      altArmed = false;
      e.preventDefault();
      toggle();
    };
    const mouse = (e: MouseEvent): void => {
      altArmed = false;
      if (!inside(e.target) && live.current.path.length > 0) leave(false);
    };
    const blur = (): void => {
      altArmed = false;
      if (live.current.path.length > 0) leave(false);
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("mousedown", mouse, true);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("mousedown", mouse, true);
      window.removeEventListener("blur", blur);
    };
    // `inside` and `live` are refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [leave, remember, focusId]);

  const anchorOf = (id: string) => () => root.current?.querySelector<HTMLElement>(selector(id)) ?? null;

  return (
    <div
      ref={root}
      className={styles.root}
      data-tauri-drag-region="false"
      onKeyDown={onKeyDown}
      onDoubleClick={(e) => e.stopPropagation()}
      onFocus={(e) => {
        // The focus came from the page (Tab, a click): that is where it goes back to.
        if (e.relatedTarget instanceof HTMLElement && !inside(e.relatedTarget)) returnTo.current = e.relatedTarget;
      }}
      onBlur={(e) => {
        // The focus went to something else in the page: the menus close.
        if (e.relatedTarget instanceof Node && !inside(e.relatedTarget)) setPath([]);
      }}
    >
      <div className={styles.bar} role="menubar" aria-label={label} data-testid="menu-bar">
        {tops.map((top, i) => {
          const open = path[0] === top.id;
          return (
            <button
              key={top.id}
              type="button"
              role="menuitem"
              aria-haspopup="menu"
              aria-expanded={open}
              tabIndex={i === current ? 0 : -1}
              data-id={top.id}
              data-level={0}
              className={cx(styles.top, open && styles.topOpen)}
              onMouseDown={remember}
              onMouseEnter={() => {
                if (path.length > 0 && !open) openTop(i, "bar");
              }}
              onClick={() => {
                if (open) setPath([]);
                else openTop(i, "bar");
              }}
            >
              {top.label}
            </button>
          );
        })}
      </div>
      {path.map((id, k) => {
        const menu = menuAt(k + 1);
        if (!menu) return null;
        const level = k + 1;
        return (
          <Popup key={id} anchor={anchorOf(id)} side={k === 0 ? "below" : "right"} label={menu.label} id={id}>
            {menu.items.map((node, j) => {
              if (node.kind === "separator") return <div key={`sep-${j}`} className={styles.separator} role="separator" />;
              const sub = node.kind === "submenu";
              const check = node.kind === "check";
              const state = stateOf(node);
              const open = sub && path[level] === node.id;
              return (
                <button
                  key={node.id}
                  type="button"
                  role={check ? "menuitemcheckbox" : "menuitem"}
                  aria-checked={check ? node.checked === true : undefined}
                  aria-haspopup={sub ? "menu" : undefined}
                  aria-expanded={sub ? open : undefined}
                  aria-disabled={state.enabled ? undefined : true}
                  aria-keyshortcuts={node.accelerator ?? undefined}
                  title={state.enabled ? undefined : state.reason}
                  tabIndex={-1}
                  data-id={node.id}
                  data-level={level}
                  className={cx(styles.item, open && styles.itemOpen)}
                  onMouseEnter={(e) => {
                    e.currentTarget.focus();
                    setPath([...path.slice(0, level), ...(sub ? [node.id] : [])]);
                  }}
                  onClick={() => {
                    if (sub) setPath([...path.slice(0, level), node.id]);
                    else activate(node);
                  }}
                >
                  <span className={styles.check} aria-hidden="true">
                    {node.checked ? "✓" : ""}
                  </span>
                  <span className={styles.label}>{node.label}</span>
                  {node.accelerator ? (
                    <span className={styles.accel} aria-hidden="true">
                      {node.accelerator}
                    </span>
                  ) : null}
                  {sub ? (
                    <span className={styles.arrow} aria-hidden="true">
                      ›
                    </span>
                  ) : null}
                </button>
              );
            })}
          </Popup>
        );
      })}
    </div>
  );
}

/** A menu's pop-up: below its bar item, or to the right of its parent item, kept inside the window. */
function Popup({ anchor, side, label, id, children }: { anchor: () => HTMLElement | null; side: "below" | "right"; label: string; id: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<CSSProperties>({ left: 0, top: 0 });
  useLayoutEffect(() => {
    const a = anchor();
    const el = ref.current;
    if (!a || !el) return;
    const r = a.getBoundingClientRect();
    const m = 4;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const w = el.offsetWidth;
    let left = side === "below" ? r.left : r.right - 2;
    // No room on the right: a submenu opens to the left of its parent menu.
    if (side === "right" && left + w > vw - m) left = r.left - w + 2;
    left = Math.max(m, Math.min(left, vw - m - w));
    let top = side === "below" ? r.bottom : r.top - 5;
    if (side === "right") top = Math.max(m, Math.min(top, vh - m - Math.min(el.offsetHeight, vh - 2 * m)));
    setPos({ left, top, maxHeight: Math.max(96, vh - m - top) });
    // Measured once per menu.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);
  return (
    <div ref={ref} className={styles.menu} style={pos} role="menu" aria-label={label} data-menu={id}>
      {children}
    </div>
  );
}
