/**
 * Header/footer area layout, the TypeScript mirror of the engine's
 * `print::body_of` (AOO semantics): an area sits inside the page margins,
 * `margins.headerMm` high with its spacing towards the body included, grown
 * with AutoFit height to its lines (4.94 mm each, the most of its three
 * regions) plus the spacing; the body starts below it. The page setup
 * dialog's preview uses it; Print Preview and PDF are drawn by the engine.
 */
import type { HeaderFooter, HeaderFooterArea, PageSetupDto } from "@tally/bridge";

/** One printed header/footer line (10 pt at 1.4 line spacing), mm. */
export const HF_LINE_MM = 4.94;
/** Most lines of one region that are laid out. */
export const HF_MAX_LINES = 50;

/** Smallest body height (engine `MIN_BODY_HMM`), mm. */
export const MIN_BODY_MM = 10;
/** Printed column header height (14 pt) and row header width (28 pt), mm. */
export const COL_HEADER_H_MM = (14 * 25.4) / 72;
export const ROW_HEADER_W_MM = (28 * 25.4) / 72;

/** AOO's defaults: on, 2.5 mm spacing, AutoFit height, no side margins. */
export const DEFAULT_HF_AREA: HeaderFooterArea = { on: true, spacingMm: 2.5, autofit: true, leftMm: 0, rightMm: 0 };

export type HfWhich = "header" | "footer";

/** The area of the header or footer (older setups without one: on when a text is set). */
export function hfArea(setup: PageSetupDto, which: HfWhich): HeaderFooterArea {
  const a = which === "header" ? setup.headerArea : setup.footerArea;
  const text = which === "header" ? setup.header : setup.footer;
  return a ?? { ...DEFAULT_HF_AREA, on: text !== null };
}

/** The header or footer when it is printed (set and switched on). */
export function hfShown(setup: PageSetupDto, which: HfWhich): HeaderFooter | null {
  const text = which === "header" ? setup.header : setup.footer;
  return text && hfArea(setup, which).on ? text : null;
}

/** Printed lines of a header/footer: the most of its regions (1…50). */
export function hfLines(hf: HeaderFooter): number {
  const n = Math.max(...[hf.left, hf.center, hf.right].map((t) => t.split("\n").length));
  return Math.min(HF_MAX_LINES, Math.max(1, n));
}

/** Printed height of an area, spacing included (mm). */
export function hfAreaHeightMm(hf: HeaderFooter, heightMm: number, area: HeaderFooterArea): number {
  return area.autofit ? Math.max(heightMm, hfLines(hf) * HF_LINE_MM + area.spacingMm) : heightMm;
}

/** A laid-out text box of a header or footer, mm from the paper's top-left. */
export interface HfBox {
  top: number;
  height: number;
  left: number;
  width: number;
}

/** The body box and the header/footer text boxes of a page setup (mm). */
export interface PageBoxes {
  paper: [number, number];
  /** Body top and height (without printed row/column headers). */
  bodyTop: number;
  bodyHeight: number;
  header: HfBox | null;
  footer: HfBox | null;
  /** The areas fit with the body keeping its minimum (else they were cut down). */
  fits: boolean;
}

export function pageBoxes(setup: PageSetupDto, paper: [number, number]): PageBoxes {
  const m = setup.margins;
  const innerW = Math.max(0, paper[0] - m.leftMm - m.rightMm);
  const box = (a: HeaderFooterArea, top: number, total: number): HfBox => ({
    top,
    height: Math.max(0, total - a.spacingMm),
    left: m.leftMm + Math.min(a.leftMm, innerW),
    width: Math.max(10, innerW - a.leftMm - a.rightMm),
  });
  const head = hfShown(setup, "header");
  const foot = hfShown(setup, "footer");
  const ha = hfArea(setup, "header");
  const fa = hfArea(setup, "footer");
  // Like the engine, the areas never squeeze the body below its minimum
  // (header first, then footer).
  const colHeader = setup.printHeaders ? COL_HEADER_H_MM : 0;
  let room = Math.max(0, paper[1] - m.topMm - m.bottomMm - colHeader - MIN_BODY_MM);
  const wantHead = head ? hfAreaHeightMm(head, m.headerMm, ha) : 0;
  const wantFoot = foot ? hfAreaHeightMm(foot, m.footerMm, fa) : 0;
  const headRoom = Math.min(wantHead, room);
  room -= headRoom;
  const footRoom = Math.min(wantFoot, room);
  return {
    paper,
    fits: headRoom === wantHead && footRoom === wantFoot,
    bodyTop: m.topMm + headRoom,
    bodyHeight: Math.max(MIN_BODY_MM, paper[1] - m.topMm - m.bottomMm - headRoom - footRoom),
    header: head ? box(ha, m.topMm, headRoom) : null,
    footer: foot ? box(fa, paper[1] - m.bottomMm - footRoom + Math.min(fa.spacingMm, footRoom), footRoom) : null,
  };
}
