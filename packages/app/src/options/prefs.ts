/**
 * App preferences (Tools ▸ Options): persisted by the desktop shell in
 * `prefs.json` (`get_prefs` / `set_prefs`, ≤ 1 MiB) and, in the browser
 * build, in `localStorage`. Document-level settings (calculation, view,
 * custom sort lists) live in the document and go through ops instead.
 */

import { DEFAULT_CHROME, parseChrome, type ChromePrefs } from "../chrome/chromeModel";
import { parseUserAutoFormats, serializeUserAutoFormats, type UserAutoFormat } from "../styles/userAutoFormat";

export type UiLanguage = "en" | "de" | "fr" | "es";
export type EnterMoves = "down" | "right" | "up" | "left";
export type DefaultFormat = "ods" | "xlsx";
/** Where the desktop app opens documents (read by the shell, `windows.rs`). */
export type OpenDocumentsIn = "tabs" | "windows";

export interface AutoCorrectPrefs {
  /** Correct TWo INitial CApitals. */
  twoInitialCapitals: boolean;
  /** Capitalise the first letter of every sentence. */
  capitalizeSentence: boolean;
  /** Replace ` - ` with an en dash and `--` with an em dash. */
  replaceDashes: boolean;
  /** 1st → 1ˢᵗ (ordinal suffix superscript). */
  ordinalSuffixes: boolean;
  /** Straight quotes → typographic quotes. */
  smartQuotes: boolean;
  /** Use the replacement table. */
  replacementTable: boolean;
}

export interface AppPrefs {
  /** UI language (menus, dialogs, panels). */
  uiLanguage: UiLanguage;
  /** AutoInput (column completion while typing). */
  autoInput: boolean;
  autoCorrect: AutoCorrectPrefs;
  /** Where Enter moves the selection after a commit. */
  enterMoves: EnterMoves;
  /** Formula syntax: `;` (OpenOffice) or `,` (Excel-style) argument separator for display. */
  argumentSeparator: ";" | ",";
  /** Show references as Excel A1 (`Sheet1!A1`) instead of CalcA1 (`Sheet1.A1`). */
  excelA1Display: boolean;
  /** Default Save As format. */
  defaultFormat: DefaultFormat;
  /** Autosave interval in minutes (0 = off). */
  autosaveMinutes: number;
  /** Document locale for new workbooks (formatting and input recognition). */
  defaultLocale: string;
  /** Delete key opens the Delete Contents dialog (AOO); off = clears at once. */
  deleteKeyDialog: boolean;
  /** AOO "Show overwrite warning when pasting data": ask before a paste replaces cells that hold data. */
  pasteOverwriteWarning: boolean;
  /** Tools ▸ Detective ▸ AutoRefresh (AOO `Input/DetectiveAuto`, on by default): redraw the arrows after changes. */
  detectiveAuto: boolean;
  /** Desktop: new and opened documents become tabs of the front window, or get windows of their own. */
  openDocumentsIn: OpenDocumentsIn;
  /** Which bars and toolbar buttons show (View ▸ Toolbars, Customize Toolbars…). */
  chrome: ChromePrefs;
  /** The user's own AutoFormats (AOO's autotbl.fmt; at most 100). */
  autoFormats: UserAutoFormat[];
  /**
   * AOO Tools ▸ Options ▸ General ▸ "Year (two digits)": two-digit years are
   * read as `start..start + 99` (1583–9857, default 1930). Documents created
   * or opened from now on take it, unless their file sets its own
   * (ODS `table:null-year`); the desktop shell hands it to the engine.
   */
  twoDigitYearStart: number;
}

/** Range of the two-digit year option (AOO's `NF_YEARVALUE` field). */
export const TWO_DIGIT_YEAR_MIN = 1583;
export const TWO_DIGIT_YEAR_MAX = 9857;

/** `true` for a valid two-digit year window start (an integer in 1583–9857). */
export function isTwoDigitYearStart(v: unknown): v is number {
  return typeof v === "number" && Number.isInteger(v) && v >= TWO_DIGIT_YEAR_MIN && v <= TWO_DIGIT_YEAR_MAX;
}

export const DEFAULT_AUTOCORRECT: AutoCorrectPrefs = {
  twoInitialCapitals: true,
  capitalizeSentence: true,
  replaceDashes: true,
  ordinalSuffixes: true,
  smartQuotes: false,
  replacementTable: true,
};

export const DEFAULT_PREFS: AppPrefs = {
  uiLanguage: "en",
  autoInput: true,
  autoCorrect: DEFAULT_AUTOCORRECT,
  enterMoves: "down",
  argumentSeparator: ";",
  excelA1Display: false,
  defaultFormat: "ods",
  autosaveMinutes: 10,
  defaultLocale: "en-US",
  deleteKeyDialog: true,
  pasteOverwriteWarning: true,
  detectiveAuto: true,
  openDocumentsIn: "tabs",
  chrome: DEFAULT_CHROME,
  autoFormats: [],
  twoDigitYearStart: 1930,
};

export const UI_LANGUAGES: readonly { id: UiLanguage; label: string; tag: string }[] = [
  { id: "en", label: "English", tag: "en-US" },
  { id: "de", label: "Deutsch", tag: "de-DE" },
  { id: "fr", label: "Français", tag: "fr-FR" },
  { id: "es", label: "Español", tag: "es-ES" },
];

/** Longest prefs document the shell accepts (bytes). */
export const MAX_PREFS_BYTES = 1024 * 1024;

function bool(v: unknown, dflt: boolean): boolean {
  return typeof v === "boolean" ? v : dflt;
}

function oneOf<T extends string>(v: unknown, allowed: readonly T[], dflt: T): T {
  return typeof v === "string" && (allowed as readonly string[]).includes(v) ? (v as T) : dflt;
}

/** Merges an unknown JSON value over the defaults, ignoring unknown and malformed fields. */
export function parsePrefs(raw: unknown): AppPrefs {
  const o = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const ac = typeof o.autoCorrect === "object" && o.autoCorrect !== null ? (o.autoCorrect as Record<string, unknown>) : {};
  const minutes = typeof o.autosaveMinutes === "number" && Number.isFinite(o.autosaveMinutes) ? Math.max(0, Math.min(120, Math.round(o.autosaveMinutes))) : DEFAULT_PREFS.autosaveMinutes;
  const locale = typeof o.defaultLocale === "string" && /^[a-z]{2}(-[A-Z]{2})?$/.test(o.defaultLocale) ? o.defaultLocale : DEFAULT_PREFS.defaultLocale;
  return {
    uiLanguage: oneOf(o.uiLanguage, ["en", "de", "fr", "es"] as const, DEFAULT_PREFS.uiLanguage),
    autoInput: bool(o.autoInput, DEFAULT_PREFS.autoInput),
    autoCorrect: {
      twoInitialCapitals: bool(ac.twoInitialCapitals, DEFAULT_AUTOCORRECT.twoInitialCapitals),
      capitalizeSentence: bool(ac.capitalizeSentence, DEFAULT_AUTOCORRECT.capitalizeSentence),
      replaceDashes: bool(ac.replaceDashes, DEFAULT_AUTOCORRECT.replaceDashes),
      ordinalSuffixes: bool(ac.ordinalSuffixes, DEFAULT_AUTOCORRECT.ordinalSuffixes),
      smartQuotes: bool(ac.smartQuotes, DEFAULT_AUTOCORRECT.smartQuotes),
      replacementTable: bool(ac.replacementTable, DEFAULT_AUTOCORRECT.replacementTable),
    },
    enterMoves: oneOf(o.enterMoves, ["down", "right", "up", "left"] as const, DEFAULT_PREFS.enterMoves),
    argumentSeparator: oneOf(o.argumentSeparator, [";", ","] as const, DEFAULT_PREFS.argumentSeparator),
    excelA1Display: bool(o.excelA1Display, DEFAULT_PREFS.excelA1Display),
    defaultFormat: oneOf(o.defaultFormat, ["ods", "xlsx"] as const, DEFAULT_PREFS.defaultFormat),
    autosaveMinutes: minutes,
    defaultLocale: locale,
    deleteKeyDialog: bool(o.deleteKeyDialog, DEFAULT_PREFS.deleteKeyDialog),
    pasteOverwriteWarning: bool(o.pasteOverwriteWarning, DEFAULT_PREFS.pasteOverwriteWarning),
    detectiveAuto: bool(o.detectiveAuto, DEFAULT_PREFS.detectiveAuto),
    openDocumentsIn: oneOf(o.openDocumentsIn, ["tabs", "windows"] as const, DEFAULT_PREFS.openDocumentsIn),
    chrome: parseChrome(o.chrome),
    autoFormats: parseUserAutoFormats(o.autoFormats),
    twoDigitYearStart: isTwoDigitYearStart(o.twoDigitYearStart) ? o.twoDigitYearStart : DEFAULT_PREFS.twoDigitYearStart,
  };
}

/** Serialises prefs for `set_prefs` (stable key order, compact). */
export function serializePrefs(p: AppPrefs): string {
  const clean = parsePrefs(p);
  return JSON.stringify({ ...clean, autoFormats: serializeUserAutoFormats(clean.autoFormats) });
}

const STORAGE_KEY = "tally.prefs";

/** The browser build's fallback store (per viewer). */
export function readLocalPrefs(): AppPrefs {
  try {
    const raw = typeof localStorage === "undefined" ? null : localStorage.getItem(STORAGE_KEY);
    return parsePrefs(raw ? JSON.parse(raw) : {});
  } catch {
    return DEFAULT_PREFS;
  }
}

export function writeLocalPrefs(p: AppPrefs): void {
  try {
    if (typeof localStorage !== "undefined") localStorage.setItem(STORAGE_KEY, serializePrefs(p));
  } catch {
    // Storage unavailable: prefs live for the session only.
  }
}
