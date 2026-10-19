import type { ApplyResult, WorkbookInfo } from "@tally/bridge";
import { beforeEach, describe, expect, it } from "vitest";
import { clampZoom, nextZoom } from "../src/formats";
import { NO_UNDO, reduceResult, useDocStore } from "../src/stores/docStore";
import { useEditStore } from "../src/stores/editStore";
import { fromSelectionDto, toSelectionDto } from "../src/stores/selectionStore";
import { MAX_TOASTS, useUiStore } from "../src/stores/uiStore";

const info = (over: Partial<WorkbookInfo> = {}): WorkbookInfo => ({
  doc: 1,
  title: "Book",
  path: null,
  sheets: [{ id: 1, name: "Sheet1", tabColor: null, hidden: false }],
  activeSheet: 1,
  dirty: false,
  version: 0,
  ...over,
});

const result = (over: Partial<ApplyResult> = {}): ApplyResult => ({
  version: 1,
  changed: [],
  layoutChanged: [],
  sheetsChanged: false,
  errors: [],
  undo: { canUndo: true, canRedo: false, undoLabel: "Typing", redoLabel: null },
  selection: null,
  info: null,
  ...over,
});

describe("docStore", () => {
  beforeEach(() => useDocStore.getState().reset());

  it("marks the document edited on cell changes", () => {
    const s = reduceResult({ info: info(), activeSheet: 1, undo: NO_UNDO }, result({ changed: [{ sheet: 1, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 0 }] }] }));
    expect(s.info?.dirty).toBe(true);
    expect(s.info?.version).toBe(1);
    expect(s.undo.canUndo).toBe(true);
  });

  it("follows the engine to a newly added sheet and away from a deleted one", () => {
    const two = info({ sheets: [...info().sheets, { id: 2, name: "Sheet2", tabColor: null, hidden: false }], activeSheet: 2 });
    let s = reduceResult({ info: info(), activeSheet: 1, undo: NO_UNDO }, result({ sheetsChanged: true, info: two }));
    expect(s.activeSheet).toBe(2);
    s = reduceResult(s, result({ sheetsChanged: true, info: info({ activeSheet: 1 }) }));
    expect(s.activeSheet).toBe(1);
  });

  it("keeps a locally chosen sheet when the engine's active sheet is unchanged", () => {
    const two = info({ sheets: [...info().sheets, { id: 2, name: "S2", tabColor: null, hidden: false }] });
    const s = reduceResult({ info: two, activeSheet: 2, undo: NO_UNDO }, result({ sheetsChanged: true, info: { ...two, title: "x" } }));
    expect(s.activeSheet).toBe(2);
  });

  it("bumps rev on every result and resets undo for a new document", () => {
    const st = useDocStore.getState();
    st.setInfo(info());
    st.applyResult(result());
    expect(useDocStore.getState().rev).toBe(2);
    expect(useDocStore.getState().undo.canUndo).toBe(true);
    useDocStore.getState().setInfo(info({ doc: 2 }));
    expect(useDocStore.getState().undo).toEqual(NO_UNDO);
  });
});

describe("editStore", () => {
  it("shares one session between grid and formula bar", () => {
    const st = useEditStore.getState();
    st.start({ r: 1, c: 2 }, "=A", "grid");
    useEditStore.getState().change("=A1*B1");
    expect(useEditStore.getState().edit).toEqual({ cell: { r: 1, c: 2 }, text: "=A1*B1", caret: 6 });
    expect(useEditStore.getState().origin).toBe("grid");
    useEditStore.getState().end();
    expect(useEditStore.getState().edit).toBeNull();
    useEditStore.getState().change("ignored");
    expect(useEditStore.getState().edit).toBeNull();
  });
});

describe("uiStore", () => {
  beforeEach(() => useUiStore.getState().reset());

  it("toggles the sidebar and clamps zoom", () => {
    useUiStore.getState().toggleSidebar();
    expect(useUiStore.getState().sidebarOpen).toBe(false);
    useUiStore.getState().setZoom(5);
    expect(useUiStore.getState().zoom).toBe(2);
    useUiStore.getState().zoomBy(-1);
    expect(useUiStore.getState().zoom).toBe(1.75);
    expect(clampZoom(0.1)).toBe(0.5);
    expect(nextZoom(1, 1)).toBe(1.1);
    expect(nextZoom(0.5, -1)).toBe(0.5);
  });

  it("caps and de-duplicates toasts", () => {
    const ui = useUiStore.getState();
    ui.pushToast("error", "a");
    ui.pushToast("error", "a");
    ui.pushToast("error", "b");
    ui.pushToast("error", "c");
    const id = ui.pushToast("info", "d");
    const toasts = useUiStore.getState().toasts;
    expect(toasts.length).toBe(MAX_TOASTS);
    expect(toasts.map((t) => t.message)).toEqual(["b", "c", "d"]);
    useUiStore.getState().dismissToast(id);
    expect(useUiStore.getState().toasts.length).toBe(2);
  });
});

describe("selection conversion and CSV", () => {
  it("round-trips the selection DTO", () => {
    const sel = { range: { r0: 0, c0: 0, r1: 0, c1: 2 }, active: { r: 0, c: 0 }, anchor: { r: 0, c: 0 } };
    const dto = toSelectionDto(sel, 3);
    expect(dto).toEqual({ sheet: 3, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 2 }], active: { r: 0, c: 0 } });
    expect(fromSelectionDto(dto)).toEqual(sel);
  });
});
