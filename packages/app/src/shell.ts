/**
 * Shell services that are not part of the engine: native file dialogs,
 * recent files and templates. The desktop app adapts `TauriClient`; in a
 * plain browser {@link createBrowserShell} provides in-memory stand-ins.
 */
import type { DocId, EngineClient, FileFormat, Op, StylePatch, WorkbookInfo } from "@tally/bridge";

export interface TemplateInfo {
  id: string;
  title: string;
  description: string;
}

export interface ShellApi {
  /** True when real files and native dialogs are available (desktop). */
  readonly native: boolean;
  /** Native Open dialog, then open; `null` when cancelled. */
  openDialog(): Promise<WorkbookInfo | null>;
  /** Opens a file from the recent list. */
  openRecent(path: string): Promise<WorkbookInfo>;
  /** Saves (dialog when `saveAs` or the document has no file); rejects with code `cancelled` if dismissed. */
  save(doc: DocId, saveAs: boolean, format: FileFormat | null, title: string): Promise<WorkbookInfo>;
  recentFiles(): Promise<string[]>;
  listTemplates(): Promise<TemplateInfo[]>;
  /** A new untitled workbook from a template. */
  newFromTemplate(id: string): Promise<WorkbookInfo>;
}

/** A template available in the browser build (CSV text). */
export interface BrowserTemplate extends TemplateInfo {
  csv: string;
}

/** Minimal RFC 4180 parser for bundled templates. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i] ?? "";
    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else quoted = false;
      } else field += ch;
    } else if (ch === '"') quoted = true;
    else if (ch === ",") {
      row.push(field);
      field = "";
    } else if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
    } else field += ch;
  }
  if (field !== "" || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

export const EMPTY_PATCH: StylePatch = {
  bold: null,
  italic: null,
  underline: null,
  strike: null,
  fontFamily: null,
  fontSizePt: null,
  color: null,
  fill: null,
  hAlign: null,
  vAlign: null,
  wrap: null,
  numberFormat: null,
};

/** In-browser shell over any EngineClient (dev server, e2e). */
export function createBrowserShell(client: EngineClient, templates: BrowserTemplate[] = []): ShellApi {
  return {
    native: false,
    openDialog: () => Promise.reject({ code: "unsupported", message: "Opening files needs the desktop app." }),
    openRecent: (path) => client.open(path),
    save: (doc, _saveAs, format, title) => {
      const fmt = format ?? "ods";
      const name = /\.[a-z]+$/i.test(title) ? title.replace(/\.[a-z]+$/i, `.${fmt}`) : `${title}.${fmt}`;
      return client.save(doc, name, fmt);
    },
    recentFiles: () => Promise.resolve([]),
    listTemplates: () => Promise.resolve(templates.map(({ id, title, description }) => ({ id, title, description }))),
    newFromTemplate: async (id) => {
      const t = templates.find((x) => x.id === id);
      const info = await client.newWorkbook(id);
      if (!t) return info;
      const rows = parseCsv(t.csv.trim());
      const width = Math.max(0, ...rows.map((r) => r.length));
      const sheet = info.activeSheet;
      const ops: Op[] = [{ type: "setRangeInputs", sheet, r: 0, c: 0, rows }];
      if (width > 0) {
        ops.push({ type: "applyStyle", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: width - 1 }], patch: { ...EMPTY_PATCH, bold: true } });
      }
      await client.apply(info.doc, { ops, label: null, selectionBefore: null, selectionAfter: null });
      return { ...info, title: t.title };
    },
  };
}
