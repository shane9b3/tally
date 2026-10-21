/**
 * Mounts @tally/grid over the current document's EngineClientSource and
 * feeds it the formula-editing state: point mode, coloured reference
 * outlines and editor spans (shared with the formula bar).
 */
import { readTheme } from "@tally/tokens";
import { Grid, SplitGrid } from "@tally/grid/react";
import { useMemo } from "react";
import { ChartLayer } from "../charts/ChartLayer";
import { objectsFor, useChartStore } from "../charts/chartStore";
import { toGridBreaks, useW5Store } from "../objects/w5Store";
import { useW56Store } from "../data/store";
import { filterButtonsFor } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { useController } from "../context";
import { formulaHighlights, isPointMode } from "../formulaEdit";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import { useV3Store } from "../stores/v3Store";
import styles from "./GridHost.module.css";

export function GridHost() {
  const ctl = useController();
  const source = useDocStore((s) => s.source);
  const sheet = useDocStore((s) => s.activeSheet);
  const sheetName = useDocStore((s) => s.info?.sheets.find((x) => x.id === s.activeSheet)?.name ?? null);
  const selection = useSelectionStore((s) => s.selection);
  const edit = useEditStore((s) => s.edit);
  const pointSpan = useEditStore((s) => s.pointSpan);
  const zoom = useUiStore((s) => s.zoom);
  const resolved = useUiStore((s) => s.theme);
  // The canvas cannot use CSS variables: re-read the tokens when the theme flips.
  const theme = useMemo(() => {
    void resolved;
    return readTheme();
  }, [resolved]);
  const text = edit?.text ?? "";
  const hl = useMemo(() => formulaHighlights(text, sheetName, resolved), [text, sheetName, resolved]);
  const pointMode = edit !== null && isPointMode(edit.text, edit.caret, pointSpan);
  const active = useEditStore((s) => s.active);
  const cell = selection.active;
  const arrayRange = active && active.sheet === sheet && active.cell.r === cell.r && active.cell.c === cell.c ? (active.arrayRange ?? null) : null;
  const filter = useDataStore((s) => s.filter);
  const filterButtons = useMemo(() => filterButtonsFor(filter), [filter]);
  const clip = useDataStore((s) => s.clip);
  const clipRange = clip && clip.sheet === sheet ? clip.range : null;
  const pageBreaksOn = useV3Store((s) => s.pageBreaksOn);
  const pageBreaks = useV3Store((s) => s.pageBreaks);
  const printRange = useV3Store((s) => s.pageSetup?.printRange ?? null);
  // ARIA mirror (wave 5, §4 item 46): the sheet name and the active cell's formula.
  const activeInput = useEditStore((s) => (s.active && s.active.sheet === sheet ? s.active.input : null));
  const aria = useMemo(() => ({ sheetName, activeFormula: activeInput && activeInput.startsWith("=") ? activeInput : null }), [sheetName, activeInput]);
  // W5-5: objects (charts, pictures), detective arrows, Page Break Preview and the split window.
  const charts = useChartStore((s) => s.charts);
  const images = useChartStore((s) => s.images);
  const objects = useMemo(() => objectsFor(charts, images), [charts, images]);
  const traces = useW5Store((s) => s.traces);
  const invalidCells = useW5Store((s) => s.invalidCells);
  const pageBreakPreview = useW5Store((s) => s.pageBreakPreview);
  const w5Breaks = useW5Store((s) => s.pageBreaks);
  const previewBreaks = useMemo(() => (w5Breaks ? toGridBreaks(w5Breaks) : null), [w5Breaks]);
  const split = useW5Store((s) => s.split);
  const w5 = ctl.w5;
  const w5Props = {
    objects,
    traces: traces.length > 0 ? traces : null,
    invalidCells: invalidCells.length > 0 ? invalidCells : null,
    pageBreakPreview,
    pageBreaks: pageBreakPreview ? (previewBreaks ?? pageBreaks) : pageBreaksOn ? pageBreaks : null,
    printRange: pageBreakPreview || pageBreaksOn ? printRange : null,
    onObjectSelect: w5.objects.onObjectSelect,
    onObjectPointerDown: w5.objects.onObjectPointerDown,
    onObjectDoubleClick: w5.objects.onObjectDoubleClick,
    onObjectKey: w5.objects.onObjectKey,
    onPageBreakDrag: w5.view.onPageBreakDrag,
    onViewportChange: () => useChartStore.getState().bumpViewport(),
  };
  const outlineGroups = useW56Store((s) => s.outline);
  const outline = outlineGroups.rows.length > 0 || outlineGroups.cols.length > 0 ? outlineGroups : null;
  const GridComponent = split ? SplitGrid : Grid;
  const splitProps = split ? { axis: split.axis, ratio: split.ratio, onRatioChange: w5.view.onSplitRatio } : { axis: "horizontal" as const, ratio: 0.5 };

  if (!source) return <div className={styles.host} data-testid="grid-loading" />;
  return (
    <div className={styles.host} data-testid="grid" data-point-mode={pointMode ? "true" : undefined} data-split={split ? split.axis : undefined} data-page-break-preview={pageBreakPreview ? "true" : undefined}>
      <GridComponent
        {...w5Props}
        {...splitProps}
        gridRef={ctl.setGrid}
        className={styles.grid}
        source={source}
        sheet={sheet}
        theme={theme}
        zoom={zoom}
        selection={selection}
        edit={edit}
        pointMode={pointMode}
        refHighlights={hl.refs}
        editHighlights={hl.spans}
        arrayRange={arrayRange}
        clipRange={clipRange}
        filterButtons={filterButtons}
        onFilterButton={ctl.onFilterButton}
        aria={aria}
        outline={outline}
        onOutlineToggle={ctl.w56.onOutlineToggle}
        onOutlineLevel={ctl.w56.onOutlineLevel}
        onCellButton={ctl.v3.onCellButton}
        onOpenLink={ctl.v3.onOpenLink}
        onNoteHover={ctl.v3.onNoteHover}
        onSelectionChange={ctl.onSelectionChange}
        onStartEdit={ctl.onGridStartEdit}
        onEditChange={ctl.onEditChange}
        onCommitEdit={ctl.onGridCommitEdit}
        onCancelEdit={ctl.cancelEdit}
        onResize={ctl.resize}
        onAutoFit={(axis, start, end) => void ctl.autoFit(axis, start, end)}
        onFill={(src, target) => void ctl.onFill(src, target)}
        onFillHandleDoubleClick={(src) => void ctl.onFillHandleDoubleClick(src)}
        onPointRange={ctl.onPointRange}
        onEditorKeyDown={ctl.handleEditorKey}
        onNavigateEdge={ctl.navigateEdge}
        onDeleteContents={ctl.deleteContents}
        onContextMenu={(cell, x, y, area) => useUiStore.getState().openContextMenu({ cell, x, y, area })}
        onCopy={ctl.onGridCopy}
        onCut={ctl.onGridCut}
        onPaste={ctl.onGridPaste}
      />
      <ChartLayer />
    </div>
  );
}
