/**
 * Keyboard and focus helpers shared by every modal and popup (1.0 a11y pass;
 * `docs/qa/a11y.md`):
 *
 * - `useReturnFocus`: remembers the element that had the focus when a modal
 *   opened and gives it the focus back when the modal closes (unless the
 *   dialog moved the focus somewhere on purpose, e.g. into the formula bar);
 * - `arrowNav`: arrow keys / Home / End inside a `tablist`, `radiogroup`,
 *   `listbox`, `menu` or `toolbar` whose items take the focus themselves
 *   (selection follows the focus for tabs, radios and options, as in the
 *   ARIA Authoring Practices);
 * - `focusFirst`: the first focusable control of a container.
 */
import { useEffect, useId, useRef, type RefObject } from "react";
import { FOCUSABLE } from "./trapTab";

/** The visible, enabled elements Tab can reach inside `root`, in DOM order. */
export function tabbables(root: ParentNode): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.hidden && el.getAttribute("aria-hidden") !== "true" && el.getClientRects().length > 0);
}

/** Focuses the first focusable control inside `root` (true when one took the focus). */
export function focusFirst(root: HTMLElement | null): boolean {
  if (!root) return false;
  const el = tabbables(root)[0];
  el?.focus();
  return el !== undefined;
}

/** Whether `el` belongs to the canvas grid (its cell editor), which `focusGrid()` already handles. */
function inGrid(el: Element | null): boolean {
  return !!el?.closest(".tally-grid");
}

/**
 * Gives the focus back to the element that had it when the calling component
 * mounted (the opener: a toolbar button, a sidebar row, …) when it unmounts,
 * but only when nothing else took it:
 *
 * - a dialog that focused something on purpose while closing keeps it;
 * - a dialog that sent the focus to the grid (`focusGrid()` on OK / Cancel)
 *   keeps it there, as in Calc, unless the opener sits in another modal that
 *   is still open (Page Setup… from Print Preview goes back to the preview);
 * - `overlay` (menus, popups) always returns to the opener unless the focus
 *   moved somewhere other than the grid.
 */
export function useReturnFocus(opts: { overlay?: boolean } = {}): () => void {
  const opener = useRef<Element | null>(typeof document === "undefined" ? null : document.activeElement);
  const overlay = opts.overlay === true;
  useEffect(() => {
    const from = opener.current;
    return () => {
      if (!(from instanceof HTMLElement) || from === document.body || inGrid(from)) return;
      // After React has removed the dialog (and its own close handler ran).
      setTimeout(() => {
        if (!from.isConnected || from.getClientRects().length === 0) return;
        const now = document.activeElement;
        if (now && now !== document.body && !inGrid(now)) return;
        if (now && inGrid(now) && !overlay && !from.closest('[aria-modal="true"]')) return;
        from.focus();
      }, 0);
    };
  }, [overlay]);
  // Gives the focus back right away (a menu item does this before running its command, so a
  // dialog opened by the command returns to the menu's opener, not to the vanished item).
  return () => {
    const from = opener.current;
    if (from instanceof HTMLElement && from !== document.body && from.isConnected) from.focus();
  };
}

/** Moves the focus into `ref` when it mounts unless it is already inside (modal dialogs). */
export function useFocusInside(ref: RefObject<HTMLElement | null>, enabled = true): void {
  useEffect(() => {
    const el = ref.current;
    if (!enabled || !el || el.contains(document.activeElement)) return;
    if (!focusFirst(el)) el.focus();
  }, [ref, enabled]);
}

/** Popups that may sit inside a toolbar (FormatBar's colour and border pickers, menus). */
const POPUP = '[role="dialog"], [role="menu"]';

const GROUP_OF: Readonly<Record<string, string>> = {
  tab: "tablist",
  radio: "radiogroup",
  option: "listbox",
  menuitem: "menu",
  menuitemcheckbox: "menu",
  menuitemradio: "menu",
};

interface KeyLike {
  key: string;
  target: EventTarget | null;
  altKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
  defaultPrevented: boolean;
  preventDefault(): void;
  stopPropagation(): void;
}

/**
 * Arrow-key navigation between the items of the composite widget that holds
 * the focused element. Returns true (and prevents the default) when it moved
 * the focus. Widgets that manage their own keys (`aria-activedescendant`
 * listboxes, the font combo boxes) handle the event first and prevent it.
 */
export function arrowNav(e: KeyLike, opts: { select?: boolean } = {}): boolean {
  if (e.defaultPrevented || e.altKey || e.metaKey || e.ctrlKey) return false;
  const el = e.target instanceof HTMLElement ? e.target : null;
  if (!el) return false;
  const role = el.getAttribute("role") ?? "";
  let groupRole = GROUP_OF[role];
  let group: HTMLElement | null = groupRole ? el.closest<HTMLElement>(`[role="${groupRole}"]`) : null;
  let items: HTMLElement[];
  if (group) {
    const sel = groupRole === "menu" ? '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]' : `[role="${role}"]`;
    items = [...group.querySelectorAll<HTMLElement>(sel)].filter((x) => x.closest(`[role="${groupRole}"]`) === group);
  } else {
    // Plain buttons inside a toolbar. A popup (colour or border picker, menu) opened from a
    // toolbar button is its own widget: its buttons never get the toolbar's navigation.
    group = el.closest<HTMLElement>('[role="toolbar"]');
    if (!group || el.tagName === "INPUT" || el.tagName === "SELECT" || el.tagName === "TEXTAREA") return false;
    const toolbar = group;
    const inPopup = (x: HTMLElement): boolean => {
      const b = x.closest(POPUP);
      return b !== null && toolbar.contains(b);
    };
    if (inPopup(el)) return false;
    groupRole = "toolbar";
    items = tabbables(group).filter((x) => x.tagName !== "INPUT" && x.tagName !== "SELECT" && x.tagName !== "TEXTAREA" && !inPopup(x));
  }
  items = items.filter((x) => !(x as HTMLButtonElement).disabled && x.getAttribute("aria-disabled") !== "true" && x.getClientRects().length > 0);
  const i = items.indexOf(el);
  if (i < 0 || items.length < 2) return false;
  const orientation = group.getAttribute("aria-orientation") ?? (groupRole === "listbox" || groupRole === "menu" ? "vertical" : groupRole === "radiogroup" ? "both" : "horizontal");
  const prev = orientation === "vertical" ? ["ArrowUp"] : orientation === "horizontal" ? ["ArrowLeft"] : ["ArrowUp", "ArrowLeft"];
  const next = orientation === "vertical" ? ["ArrowDown"] : orientation === "horizontal" ? ["ArrowRight"] : ["ArrowDown", "ArrowRight"];
  let j: number;
  if (prev.includes(e.key)) j = (i - 1 + items.length) % items.length;
  else if (next.includes(e.key)) j = (i + 1) % items.length;
  else if (e.key === "Home") j = 0;
  else if (e.key === "End") j = items.length - 1;
  else return false;
  e.preventDefault();
  e.stopPropagation();
  const to = items[j]!;
  to.focus();
  // Selection follows the focus (tabs, radio buttons, single-select lists). Never in a
  // multi-select list (a click toggles the item), nor where a click acts or commits:
  // `select: false` from the caller (the Styles panel applies the style) or
  // `data-select-on-focus="false"` on the group (the chart editor's type and segmented
  // pickers commit an undo step per click); Enter / Space activate the focused item there.
  const selects = opts.select !== false && group.getAttribute("data-select-on-focus") !== "false" && group.getAttribute("aria-multiselectable") !== "true";
  if (selects && (role === "tab" || role === "radio" || role === "option")) to.click();
  return true;
}

/** A DOM id for a dialog's title (`aria-labelledby`). */
export function useTitleId(): string {
  return `dlg-title-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
}
