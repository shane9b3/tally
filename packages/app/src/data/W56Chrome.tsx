/**
 * The W5-6 chrome mounted by `App.tsx`: the dialog dispatcher, the extra
 * shortcuts (window keydown in the capture phase: Delete → Delete Contents
 * dialog, ⌥↩ fill selection, ⌃D selection list, F8 / ⇧F8 selection modes,
 * ⌘* data area, ⌘/ array formula, ⌥ arrows resize, ⌃⌘+ / ⌃⌘− cells), the
 * AutoInput ghost text over the in-cell editor, the pivot output header
 * band and the selection-mode badge. `W56Panels` mounts the Styles and
 * Navigator panels next to the grid.
 */
import { extendTo, MAX_COLS, MAX_ROWS } from "@tally/grid";
import { useEffect, useState } from "react";
import { useChartStore } from "../charts/chartStore";
import { useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { isTypingTarget } from "../useKeymap";
import { ghostSuffix } from "../autocorrect/autoinput";
import { NavigatorPanel } from "../navigator/NavigatorPanel";
import { OptionsDialog } from "../options/OptionsDialog";
import { DataPilotDialog } from "../pivot/DataPilotDialog";
import { pivotHeaderRanges } from "../pivot/pivotModel";
import { AutoFormatDialog, FormatCellsDialog, StyleEditorDialog, StylesPanel } from "../styles/StylesUi";
import {
  ConsolidateDialog,
  CopySheetDialog,
  CreateNamesDialog,
  DefineDatabaseRangeDialog,
  DeleteCellsDialog,
  DeleteContentsDialog,
  DocumentStatsDialog,
  FillSeriesDialog,
  InsertCellsDialog,
  MergePromptDialog,
  MultipleOperationsDialog,
  RecoveryDialog,
  RemoveDuplicatesDialog,
  SaveAsTemplateDialog,
  ScenariosDialog,
  SelectionListPopover,
  ShowOutlineLevelDialog,
  ShowSheetDialog,
  SolverDialog,
  SortOptionsDialog,
  StandardFilterDialog,
  SubtotalsDialog,
  TabColorDialog,
  TextToColumnsDialog,
} from "./DataDialogs";
import { useW56Store, type W56Dialog } from "./store";
import styles from "./W56.module.css";

function DialogFor({ d }: { d: W56Dialog }) {
  switch (d.kind) {
    case "dataPilot":
      return <DataPilotDialog d={d} />;
    case "standardFilter":
      return <StandardFilterDialog d={d} />;
    case "subtotals":
      return <SubtotalsDialog d={d} />;
    case "consolidate":
      return <ConsolidateDialog />;
    case "textToColumns":
      return <TextToColumnsDialog d={d} />;
    case "removeDuplicates":
      return <RemoveDuplicatesDialog d={d} />;
    case "scenarios":
      return <ScenariosDialog d={d} />;
    case "defineDatabaseRange":
      return <DefineDatabaseRangeDialog d={d} />;
    case "createNamesFromLabels":
      return <CreateNamesDialog d={d} />;
    case "fillSeries":
      return <FillSeriesDialog d={d} />;
    case "sortOptions":
      return <SortOptionsDialog d={d} />;
    case "deleteContents":
      return <DeleteContentsDialog d={d} />;
    case "multipleOperations":
      return <MultipleOperationsDialog d={d} />;
    case "insertCells":
      return <InsertCellsDialog d={d} />;
    case "deleteCells":
      return <DeleteCellsDialog d={d} />;
    case "copySheet":
      return <CopySheetDialog d={d} />;
    case "showSheet":
      return <ShowSheetDialog d={d} />;
    case "tabColor":
      return <TabColorDialog d={d} />;
    case "showOutlineLevel":
      return <ShowOutlineLevelDialog d={d} />;
    case "solver":
      return <SolverDialog d={d} />;
    case "mergePrompt":
      return <MergePromptDialog d={d} />;
    case "autoFormat":
      return <AutoFormatDialog d={d} />;
    case "formatCells":
      return <FormatCellsDialog d={d} />;
    case "styleEditor":
      return <StyleEditorDialog d={d} />;
    case "options":
      return <OptionsDialog d={d} />;
    case "documentStats":
      return <DocumentStatsDialog d={d} />;
    case "recovery":
      return <RecoveryDialog d={d} />;
    case "saveAsTemplate":
      return <SaveAsTemplateDialog d={d} />;
    case "selectionList":
      return <SelectionListPopover d={d} />;
  }
}

/** Dialogs, ghost text, pivot bands, selection-mode badge and the key hook. */
export function W56Chrome() {
  const dialog = useW56Store((s) => s.dialog);
  const extend = useW56Store((s) => s.extendMode);
  const add = useW56Store((s) => s.addMode);
  useW56Keys();
  return (
    <>
      {dialog ? <DialogFor d={dialog} /> : null}
      <AutoInputGhost />
      <PivotBands />
      {extend || add ? <div className={styles.modeBadge} data-testid="selection-mode">{extend ? "Extend selection (F8)" : "Add to selection (⇧F8)"}</div> : null}
    </>
  );
}

/** The right-side panels (inside the grid's `.work` row). */
export function W56Panels() {
  const panel = useW56Store((s) => s.panel);
  if (panel === "styles") return <StylesPanel />;
  if (panel === "navigator") return <NavigatorPanel />;
  return null;
}

/** Ghost text of the AutoInput completion, drawn over the in-cell editor. */
function AutoInputGhost() {
  const ctl = useController();
  const ghost = useW56Store((s) => s.ghost);
  const edit = useEditStore((s) => s.edit);
  const zoom = useUiStore((s) => s.zoom);
  const [rect, setRect] = useState<DOMRect | null>(null);
  useEffect(() => {
    if (!ghost || !edit) {
      setRect(null);
      return;
    }
    setRect(ctl.grid?.editorRect() ?? null);
  }, [ctl, ghost, edit, zoom]);
  if (!ghost || !edit || !rect) return null;
  const suffix = ghostSuffix(edit.text, ghost);
  if (suffix === "") return null;
  return (
    <div className={styles.ghost} style={{ left: rect.left, top: rect.top, height: rect.height, fontSize: 13 * zoom }} aria-hidden="true" data-testid="autoinput-ghost">
      <span className={styles.ghostTyped}>{edit.text}</span>
      <span>{suffix}</span>
    </div>
  );
}

/** A tinted band over each pivot table's header rows/columns (the engine styles the cells; this marks the region). */
function PivotBands() {
  const ctl = useController();
  const pivots = useW56Store((s) => s.pivots);
  const sheet = useDocStore((s) => s.activeSheet);
  const rev = useDocStore((s) => s.rev);
  const tick = useChartStore((s) => s.viewportTick);
  const zoom = useUiStore((s) => s.zoom);
  const [boxes, setBoxes] = useState<{ left: number; top: number; width: number; height: number }[]>([]);
  useEffect(() => {
    const grid = ctl.grid;
    if (!grid) return;
    const out: { left: number; top: number; width: number; height: number }[] = [];
    for (const p of pivots) {
      if (p.targetSheet !== sheet) continue;
      for (const r of pivotHeaderRanges(p)) {
        const a = grid.cellClientRect({ r: r.r0, c: r.c0 });
        const b = grid.cellClientRect({ r: r.r1, c: r.c1 });
        if (!a || !b) continue;
        out.push({ left: a.left, top: a.top, width: Math.max(0, b.right - a.left), height: Math.max(0, b.bottom - a.top) });
      }
    }
    setBoxes(out);
  }, [ctl, pivots, sheet, rev, tick, zoom]);
  return (
    <>
      {boxes.map((b, i) => (
        <div key={i} className={styles.pivotBand} style={b} data-testid="pivot-band" />
      ))}
    </>
  );
}


/** Keys the keymap table cannot express (they need context or collide with grid keys). */
export function useW56Keys(): void {
  const ctl = useController();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      const st = useW56Store.getState();
      const editing = useEditStore.getState().edit !== null;
      const typing = isTypingTarget(e.target) || editing;
      const inGrid = e.target instanceof HTMLElement && e.target.closest(".tally-grid") !== null;
      const mod = e.metaKey || e.ctrlKey;
      const consume = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      // Esc leaves paint-can and selection modes.
      if (e.key === "Escape" && !typing && (st.paintStyle || st.extendMode || st.addMode)) {
        st.setPaintStyle(null);
        st.setExtendMode(false);
        st.setAddMode(false);
        return;
      }
      // ⌃D (Control, not Cmd): the selection list — while editing or on the cell.
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "d" && (inGrid || editing)) {
        consume();
        void ctl.w56.openSelectionList();
        return;
      }
      if (typing || st.dialog || useUiStore.getState().dialog) return;
      // A selected chart/picture owns Delete, Enter and the arrows (W5-5's object layer).
      if (!inGrid || useChartStore.getState().selected !== null) return;
      // Delete → Delete Contents dialog (Backspace stays immediate).
      if (e.key === "Delete" && !mod && !e.altKey && !e.shiftKey) {
        if (!st.prefs.deleteKeyDialog) return;
        consume();
        ctl.w56.openDeleteContents();
        return;
      }
      if (e.key === "Enter" && e.altKey && !mod && !e.shiftKey) {
        consume();
        void ctl.w56.fillSelection();
        return;
      }
      if (e.key === "F8" && !mod && !e.altKey) {
        consume();
        if (e.shiftKey) st.setAddMode(!st.addMode);
        else st.setExtendMode(!st.extendMode);
        return;
      }
      if (mod && !e.altKey && (e.code === "NumpadMultiply" || (e.shiftKey && e.code === "Digit8"))) {
        consume();
        void ctl.w56.selectDataArea();
        return;
      }
      if (mod && !e.altKey && e.code === "NumpadDivide") {
        consume();
        ctl.w56.selectArrayFormula();
        return;
      }
      if (mod && !e.altKey && (e.code === "NumpadAdd" || (e.ctrlKey && e.metaKey && e.code === "Equal"))) {
        consume();
        ctl.w56.openInsertCells();
        return;
      }
      if (mod && !e.altKey && (e.code === "NumpadSubtract" || (e.ctrlKey && e.metaKey && e.code === "Minus"))) {
        consume();
        ctl.w56.openDeleteCells();
        return;
      }
      const arrows: Record<string, [number, number]> = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
      const delta = arrows[e.key];
      if (delta && e.altKey && !mod) {
        consume();
        const [dr, dc] = delta;
        if (dr !== 0) ctl.w56.resizeByKey("row", dr * 4, e.shiftKey);
        else ctl.w56.resizeByKey("col", dc * 8, e.shiftKey);
        return;
      }
      if (delta && st.extendMode && !mod && !e.altKey) {
        consume();
        const [dr, dc] = delta;
        const sel = useSelectionStore.getState().selection;
        const corner = { r: sel.anchor.r === sel.range.r0 ? sel.range.r1 : sel.range.r0, c: sel.anchor.c === sel.range.c0 ? sel.range.c1 : sel.range.c0 };
        const next = extendTo(sel, { r: Math.max(0, Math.min(MAX_ROWS - 1, corner.r + dr)), c: Math.max(0, Math.min(MAX_COLS - 1, corner.c + dc)) });
        ctl.grid?.setSelection(next, true);
        useSelectionStore.getState().setSelection(next);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [ctl]);

  // ⇧F8 add mode: a plain click adds a range instead of replacing the selection.
  useEffect(() => {
    const off = useSelectionStore.subscribe((s, prev) => {
      const st = useW56Store.getState();
      if (!st.addMode || s.selection === prev.selection) return;
      const cur = s.selection;
      const before = prev.selection;
      if ((cur.others?.length ?? 0) > 0 || useEditStore.getState().edit) return;
      const same = cur.range.r0 === before.range.r0 && cur.range.c0 === before.range.c0 && cur.range.r1 === before.range.r1 && cur.range.c1 === before.range.c1;
      if (same) return;
      const inside = cur.active.r >= before.range.r0 && cur.active.r <= before.range.r1 && cur.active.c >= before.range.c0 && cur.active.c <= before.range.c1;
      if (inside && cur.range.r0 === cur.range.r1 && cur.range.c0 === cur.range.c1) return;
      const merged = { ...cur, others: [...(before.others ?? []), before.range] };
      useSelectionStore.getState().setSelection(merged);
      ctl.grid?.setSelection(merged);
    });
    return off;
  }, [ctl]);
}
