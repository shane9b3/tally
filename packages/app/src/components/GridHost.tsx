/**
 * Mounts @tally/grid over the current document's EngineClientSource and
 * feeds it the formula-editing state: point mode, coloured reference
 * outlines and editor spans (shared with the formula bar).
 */
import { readTheme } from "@tally/tokens";
import { Grid } from "@tally/grid/react";
import { useMemo } from "react";
import { filterButtonsFor } from "../dataOps";
import { useDataStore } from "../stores/dataStore";
import { useController } from "../context";
import { formulaHighlights, isPointMode } from "../formulaEdit";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
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

  if (!source) return <div className={styles.host} data-testid="grid-loading" />;
  return (
    <div className={styles.host} data-testid="grid" data-point-mode={pointMode ? "true" : undefined}>
      <Grid
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
    </div>
  );
}
