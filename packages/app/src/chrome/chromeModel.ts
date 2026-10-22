/**
 * Which parts of the upper chrome show (AOO View ▸ Toolbars and a toolbar's
 * "Visible Buttons"): the whole bars (Toolbar, Formatting bar, Formula bar,
 * Status bar) and, per toolbar, each button. Kept in the app prefs
 * (`AppPrefs.chrome`, prefs.json in the desktop app and broadcast to every
 * window; localStorage in the browser build). Default: everything visible.
 *
 * Hidden buttons stay reachable through the menus, shortcuts and ⌘K.
 * Separators collapse when the buttons around them are hidden
 * ({@link visibleSections}); buttons that do not fit the window go into a
 * "»" overflow menu ({@link fitCount}).
 */

export type BarId = "toolbar" | "formatting" | "formulaBar" | "statusBar";
export type ToolbarId = "toolbar" | "formatting";

export const BARS: readonly BarId[] = ["toolbar", "formatting", "formulaBar", "statusBar"];
export const TOOLBARS: readonly ToolbarId[] = ["toolbar", "formatting"];

/** A section element: a button, or a pill group of buttons. */
export type BarElement = string | readonly string[];
/** Sections are separated by a thin rule. */
export type BarSections = readonly (readonly BarElement[])[];

/** The top row's buttons left of the spacer (title first, not customisable). */
export const TOOLBAR_SECTIONS: BarSections = [["save"], ["undo", "redo"], ["sort", "filter", "addUp", "chart"]];
/** The top row's right-hand buttons (no separators). */
export const TOOLBAR_RIGHT: readonly string[] = ["theme", "glance", "share"];

export const FORMATTING_SECTIONS: BarSections = [
  ["fontName", "fontSize"],
  [["bold", "italic", "underline", "strike"]],
  [["fontColor", "fillColor"]],
  [["alignLeft", "alignCenter", "alignRight", "wrap"]],
  [["merge", "borders"]],
  ["numberFormat", ["currency", "percent"]],
  ["highlight", "colorByValue"],
];

const flat = (s: BarSections): string[] => s.flatMap((sec) => sec.flatMap((e) => (typeof e === "string" ? [e] : [...e])));

/** Every button of a toolbar, in order. */
export const TOOLBAR_ITEMS: Readonly<Record<ToolbarId, readonly string[]>> = {
  toolbar: [...flat(TOOLBAR_SECTIONS), ...TOOLBAR_RIGHT],
  formatting: flat(FORMATTING_SECTIONS),
};

/** English labels (the `t()` keys are `chrome.item.<id>`). */
export const ITEM_LABELS: Readonly<Record<string, string>> = {
  save: "Save",
  undo: "Undo",
  redo: "Redo",
  sort: "Sort",
  filter: "Filter",
  addUp: "Add up",
  chart: "Chart",
  theme: "Theme",
  glance: "At a glance",
  share: "Share",
  fontName: "Font name",
  fontSize: "Font size",
  bold: "Bold",
  italic: "Italic",
  underline: "Underline",
  strike: "Strikethrough",
  fontColor: "Font colour",
  fillColor: "Fill colour",
  alignLeft: "Align left",
  alignCenter: "Align centre",
  alignRight: "Align right",
  wrap: "Wrap text",
  merge: "Merge cells",
  borders: "Borders",
  numberFormat: "Number format",
  currency: "Currency",
  percent: "Percent",
  highlight: "Highlight cells",
  colorByValue: "Colour by value",
};

export const BAR_LABELS: Readonly<Record<BarId, string>> = {
  toolbar: "Toolbar",
  formatting: "Formatting Bar",
  formulaBar: "Formula Bar",
  statusBar: "Status Bar",
};

export interface ChromePrefs {
  /** Whole bars shown. */
  bars: Record<BarId, boolean>;
  /** Hidden buttons per toolbar (known ids only, in toolbar order). */
  hidden: Record<ToolbarId, string[]>;
}

export const DEFAULT_CHROME: ChromePrefs = {
  bars: { toolbar: true, formatting: true, formulaBar: true, statusBar: true },
  hidden: { toolbar: [], formatting: [] },
};

/** Merges an unknown JSON value over the defaults (unknown bars and buttons are dropped). */
export function parseChrome(raw: unknown): ChromePrefs {
  const o = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const b = typeof o.bars === "object" && o.bars !== null ? (o.bars as Record<string, unknown>) : {};
  const h = typeof o.hidden === "object" && o.hidden !== null ? (o.hidden as Record<string, unknown>) : {};
  const bars = { ...DEFAULT_CHROME.bars };
  for (const id of BARS) if (typeof b[id] === "boolean") bars[id] = b[id] as boolean;
  const hidden: Record<ToolbarId, string[]> = { toolbar: [], formatting: [] };
  for (const id of TOOLBARS) {
    const list = Array.isArray(h[id]) ? (h[id] as unknown[]) : [];
    hidden[id] = TOOLBAR_ITEMS[id].filter((item) => list.includes(item));
  }
  return { bars, hidden };
}

export function isDefaultChrome(c: ChromePrefs): boolean {
  return BARS.every((b) => c.bars[b]) && TOOLBARS.every((t) => c.hidden[t].length === 0);
}

export function withBar(c: ChromePrefs, bar: BarId, visible: boolean): ChromePrefs {
  return { ...c, bars: { ...c.bars, [bar]: visible } };
}

/** Shows or hides one button. */
export function withItem(c: ChromePrefs, bar: ToolbarId, item: string, visible: boolean): ChromePrefs {
  const set = new Set(c.hidden[bar]);
  if (visible) set.delete(item);
  else set.add(item);
  return { ...c, hidden: { ...c.hidden, [bar]: TOOLBAR_ITEMS[bar].filter((i) => set.has(i)) } };
}

export function isItemVisible(c: ChromePrefs, bar: ToolbarId, item: string): boolean {
  return !c.hidden[bar].includes(item);
}

/**
 * The sections with hidden buttons removed: empty pill groups and empty
 * sections disappear, so no two separators meet and none leads or trails.
 */
export function visibleSections(sections: BarSections, hidden: readonly string[]): BarElement[][] {
  const out: BarElement[][] = [];
  for (const sec of sections) {
    const els: BarElement[] = [];
    for (const e of sec) {
      if (typeof e === "string") {
        if (!hidden.includes(e)) els.push(e);
      } else {
        const g = e.filter((i) => !hidden.includes(i));
        if (g.length > 0) els.push(g);
      }
    }
    if (els.length > 0) out.push(els);
  }
  return out;
}

/** A row of slots as drawn: elements and the separators between sections. */
export type Slot = { kind: "sep"; key: string } | { kind: "el"; key: string; el: BarElement };

export function slotsOf(sections: readonly BarElement[][]): Slot[] {
  const out: Slot[] = [];
  sections.forEach((sec, i) => {
    if (i > 0) out.push({ kind: "sep", key: `sep-${i}` });
    for (const el of sec) out.push({ kind: "el", key: typeof el === "string" ? el : el.join("+"), el });
  });
  return out;
}

/**
 * How many slots fit in `available` px: slot widths plus `gap` between them,
 * reserving `moreWidth` (+ gap) for the "»" button when not all fit. A
 * separator never ends the visible part.
 */
export function fitCount(widths: readonly number[], isSep: readonly boolean[], available: number, gap: number, moreWidth: number): number {
  const total = widths.reduce((s, w) => s + w, 0) + gap * Math.max(0, widths.length - 1);
  if (total <= available + 0.5) return widths.length;
  const room = available - moreWidth - gap;
  let used = 0;
  let n = 0;
  for (let i = 0; i < widths.length; i++) {
    const next = used + (i > 0 ? gap : 0) + (widths[i] ?? 0);
    if (next > room + 0.5) break;
    used = next;
    n = i + 1;
  }
  while (n > 0 && isSep[n - 1]) n--;
  return n;
}

/** How a row is drawn: compact (labels of wide buttons dropped) and how many slots show. */
export interface RowLayout {
  compact: boolean;
  n: number;
}

/**
 * The layout of a row: everything with labels when it fits; else compact
 * buttons (icon-only) when the row has any, still everything if that fits;
 * only then the "»" overflow. `null` widths are not measured yet: the
 * answer is the layout to draw so they can be measured.
 */
export function chooseLayout(full: readonly (number | null)[], compact: readonly (number | null)[] | null, isSep: readonly boolean[], available: number, gap: number, moreWidth: number): RowLayout {
  const len = full.length;
  if (full.some((w) => w === null)) return { compact: false, n: len };
  const fw = full as number[];
  const sum = (ws: readonly number[]) => ws.reduce((s, w) => s + w, 0) + gap * Math.max(0, ws.length - 1);
  if (sum(fw) <= available + 0.5) return { compact: false, n: len };
  if (!compact) return { compact: false, n: fitCount(fw, isSep, available, gap, moreWidth) };
  if (compact.some((w) => w === null)) return { compact: true, n: len };
  return { compact: true, n: fitCount(compact as number[], isSep, available, gap, moreWidth) };
}
