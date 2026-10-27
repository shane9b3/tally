/**
 * @tally/tokens: design tokens for Tally.
 *
 * - `@tally/tokens/tokens.css`: CSS custom properties: light, and dark under
 *   `[data-theme="dark"]` (also the default when the OS prefers dark and no
 *   `data-theme` is set).
 * - `@tally/tokens/fonts`: bundles Inter Variable.
 * - `readTheme()`: reads the variables into a typed {@link Theme}, because
 *   canvas drawing cannot use CSS variables. Call it again whenever
 *   `data-theme` changes.
 */

/** Colours the canvas grid draws with (CSS colour strings). */
export interface ThemeColors {
  bg: string;
  gridLine: string;
  headerBg: string;
  headerText: string;
  text: string;
  textMuted: string;
  selection: string;
  selectionBorder: string;
  /** Bottom/right edge of the header strips (slightly darker than gridLine). */
  headerLine: string;
  /** Header cell behind the active row/column. */
  headerActiveBg: string;
  /** Header label of the active row/column. */
  headerActiveText: string;
  /** The square at the bottom-right of the selection. */
  fillHandle: string;
  /** Column resize guide line and hovered resize gutter. */
  resizeGuide: string;
  /** Background of the in-cell editor. */
  editorBg: string;
  /** The divider between frozen panes and the scrolling area. */
  frozenLine: string;
  accent: string;
  chipGreenBg: string;
  chipGreenFg: string;
  chipAmberBg: string;
  chipAmberFg: string;
  chipRedBg: string;
  chipRedFg: string;
  /** Hyperlink text. */
  link: string;
  /** The note indicator (a small triangle at a cell's top-right corner). */
  noteMarker: string;
  /** Page-break lines and the print range outline. */
  pageBreak: string;
}

/** Typed snapshot of the design tokens. Lengths are CSS pixels. */
export interface Theme {
  colors: ThemeColors;
  fonts: {
    /** CSS font-family list, e.g. `"Inter Variable", system-ui, sans-serif`. */
    family: string;
    /** Base cell font size (the size of a 10 pt cell at zoom 100%). */
    sizePx: number;
    /** Column letters and row numbers. */
    headerSizePx: number;
  };
  radii: { sm: number; card: number; pill: number };
  spacing: { s1: number; s2: number; s3: number; s4: number };
}

/** The CSS variable behind every {@link Theme} field. */
export const THEME_VARIABLES = {
  colors: {
    bg: "--tally-bg",
    gridLine: "--tally-grid-line",
    headerBg: "--tally-header-bg",
    headerText: "--tally-header-text",
    text: "--tally-text",
    textMuted: "--tally-text-muted",
    selection: "--tally-selection",
    selectionBorder: "--tally-selection-border",
    headerLine: "--tally-header-line",
    headerActiveBg: "--tally-header-active-bg",
    headerActiveText: "--tally-header-active-text",
    fillHandle: "--tally-fill-handle",
    resizeGuide: "--tally-resize-guide",
    editorBg: "--tally-editor-bg",
    frozenLine: "--tally-frozen-line",
    accent: "--tally-accent",
    chipGreenBg: "--tally-chip-green-bg",
    chipGreenFg: "--tally-chip-green-fg",
    chipAmberBg: "--tally-chip-amber-bg",
    chipAmberFg: "--tally-chip-amber-fg",
    chipRedBg: "--tally-chip-red-bg",
    chipRedFg: "--tally-chip-red-fg",
    link: "--tally-link",
    noteMarker: "--tally-note-marker",
    pageBreak: "--tally-page-break",
  },
  fonts: { family: "--tally-font-family", sizePx: "--tally-font-size", headerSizePx: "--tally-font-size-header" },
  radii: { sm: "--tally-radius-sm", card: "--tally-radius-card", pill: "--tally-radius-pill" },
  spacing: { s1: "--tally-space-1", s2: "--tally-space-2", s3: "--tally-space-3", s4: "--tally-space-4" },
} as const satisfies {
  colors: Record<keyof ThemeColors, string>;
  fonts: Record<keyof Theme["fonts"], string>;
  radii: Record<keyof Theme["radii"], string>;
  spacing: Record<keyof Theme["spacing"], string>;
};

/** Light-theme values, used when a variable is missing (e.g. CSS not loaded). */
export const DEFAULT_THEME: Theme = {
  colors: {
    bg: "#ffffff",
    gridLine: "#edeef1",
    headerBg: "#f9fafb",
    headerText: "#5f6673",
    text: "#111827",
    textMuted: "#5f6673",
    selection: "rgba(37, 99, 235, 0.08)",
    selectionBorder: "#2563eb",
    headerLine: "#e2e4e8",
    headerActiveBg: "#e8effd",
    headerActiveText: "#1d4ed8",
    fillHandle: "#2563eb",
    resizeGuide: "#2563eb",
    editorBg: "#ffffff",
    frozenLine: "#a3a9b4",
    accent: "#2563eb",
    chipGreenBg: "#dcfce7",
    chipGreenFg: "#166534",
    chipAmberBg: "#fef3c7",
    chipAmberFg: "#92400e",
    chipRedBg: "#fee2e2",
    chipRedFg: "#991b1b",
    link: "#1d4ed8",
    noteMarker: "#dc2626",
    pageBreak: "#2563eb",
  },
  fonts: {
    family: '"Inter Variable", "Inter", system-ui, -apple-system, "Segoe UI", sans-serif',
    sizePx: 13,
    headerSizePx: 11,
  },
  radii: { sm: 4, card: 10, pill: 999 },
  spacing: { s1: 4, s2: 8, s3: 12, s4: 16 },
};

/** Anything with `getPropertyValue`, such as a `CSSStyleDeclaration`. */
export interface StyleSource {
  getPropertyValue(name: string): string;
}

function readGroup<K extends string, V>(
  src: StyleSource,
  vars: Record<K, string>,
  defaults: Record<K, V>,
  parse: (raw: string) => V | null,
): Record<K, V> {
  const out = { ...defaults };
  for (const k of Object.keys(vars) as K[]) {
    const raw = src.getPropertyValue(vars[k]).trim();
    const v = raw === "" ? null : parse(raw);
    if (v !== null) out[k] = v;
  }
  return out;
}

/** Parses a pixel length (`"13px"` or `"13"`); `null` if not a number. */
function parsePx(raw: string): number | null {
  const n = Number.parseFloat(raw);
  return Number.isFinite(n) ? n : null;
}

/**
 * Reads the current design tokens.
 *
 * @param source defaults to `getComputedStyle(document.documentElement)`;
 *   pass another element's computed style (or a stub in tests) to override.
 *   Missing variables fall back to {@link DEFAULT_THEME}.
 */
export function readTheme(source?: StyleSource): Theme {
  const src = source ?? getComputedStyle(document.documentElement);
  return {
    colors: readGroup(src, THEME_VARIABLES.colors, DEFAULT_THEME.colors, (s) => s),
    fonts: {
      family: readGroup(src, { family: THEME_VARIABLES.fonts.family }, { family: DEFAULT_THEME.fonts.family }, (s) => s)
        .family,
      ...readGroup(
        src,
        { sizePx: THEME_VARIABLES.fonts.sizePx, headerSizePx: THEME_VARIABLES.fonts.headerSizePx },
        { sizePx: DEFAULT_THEME.fonts.sizePx, headerSizePx: DEFAULT_THEME.fonts.headerSizePx },
        parsePx,
      ),
    },
    radii: readGroup(src, THEME_VARIABLES.radii, DEFAULT_THEME.radii, parsePx),
    spacing: readGroup(src, THEME_VARIABLES.spacing, DEFAULT_THEME.spacing, parsePx),
  };
}
