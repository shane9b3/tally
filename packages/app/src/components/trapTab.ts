/** Focus trap shared by every modal (kept apart from the dialog bundle so the chrome can use it without loading it). */
export const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Keeps Tab / Shift+Tab inside `form` (every dialog is modal: §4 item 46,
 * "dialogs focus-trapped"). Returns true when the key was handled.
 *
 * Every Tab press is handled here (not only at the ends), so a dialog tabs
 * through all of its controls even in WebKit, where Tab skips buttons unless
 * "Press Tab to highlight each item" is on; hidden and collapsed controls
 * are skipped.
 */
export function trapTab(form: HTMLElement, e: { key: string; shiftKey: boolean; altKey?: boolean; metaKey?: boolean; ctrlKey?: boolean; preventDefault(): void }, active: Element | null): boolean {
  if (e.key !== "Tab" || e.altKey || e.metaKey || e.ctrlKey) return false;
  const all = tabStops([...form.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.hidden && el.getAttribute("aria-hidden") !== "true" && el.tabIndex >= 0), active);
  // Skip controls that are not rendered (collapsed sections); without layout (tests) keep them all.
  const shown = all.filter((el) => el === active || el.getClientRects().length > 0);
  const items = shown.some((el) => el !== active) ? shown : all;
  e.preventDefault();
  if (items.length === 0) return true;
  const idx = active && form.contains(active) ? items.indexOf(active as HTMLElement) : -1;
  const n = items.length;
  const next = idx === -1 ? (e.shiftKey ? n - 1 : 0) : (idx + (e.shiftKey ? -1 : 1) + n) % n;
  items[next]?.focus();
  return true;
}

/**
 * The Tab stops among `els`, as the browser has them: no `tabindex="-1"` element (roving
 * items are reached with the arrow keys), and one stop per radio group (`name`): the
 * checked radio, or the first one when none is checked. `active` always stays in.
 */
export function tabStops(els: HTMLElement[], active: Element | null = null): HTMLElement[] {
  const radio = (el: Element | null): el is HTMLInputElement => el instanceof HTMLInputElement && el.type === "radio" && el.name !== "";
  const stopOf = new Map<string, HTMLElement>();
  if (radio(active)) stopOf.set(active.name, active);
  for (const el of els) {
    if (!radio(el) || stopOf.get(el.name) === active) continue;
    const cur = stopOf.get(el.name) as HTMLInputElement | undefined;
    if (!cur || (el.checked && !cur.checked)) stopOf.set(el.name, el);
  }
  return els.filter((el) => !radio(el) || stopOf.get(el.name) === el);
}
