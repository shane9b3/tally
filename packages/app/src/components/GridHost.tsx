/** Mounts @tally/grid over the current document's EngineClientSource. */
import { readTheme } from "@tally/tokens";
import { Grid } from "@tally/grid/react";
import { useMemo } from "react";
import { useController } from "../context";
import { useDocStore } from "../stores/docStore";
import { useEditStore } from "../stores/editStore";
import { useSelectionStore } from "../stores/selectionStore";
import { useUiStore } from "../stores/uiStore";
import styles from "./GridHost.module.css";

export function GridHost() {
  const ctl = useController();
  const source = useDocStore((s) => s.source);
  const sheet = useDocStore((s) => s.activeSheet);
  const selection = useSelectionStore((s) => s.selection);
  const edit = useEditStore((s) => s.edit);
  const zoom = useUiStore((s) => s.zoom);
  const theme = useMemo(() => readTheme(), []);

  if (!source) return <div className={styles.host} data-testid="grid-loading" />;
  return (
    <div className={styles.host} data-testid="grid">
      <Grid
        gridRef={ctl.setGrid}
        className={styles.grid}
        source={source}
        sheet={sheet}
        theme={theme}
        zoom={zoom}
        selection={selection}
        edit={edit}
        onSelectionChange={ctl.onSelectionChange}
        onStartEdit={ctl.onGridStartEdit}
        onEditChange={ctl.onEditChange}
        onCommitEdit={ctl.onGridCommitEdit}
        onCancelEdit={ctl.cancelEdit}
        onResize={ctl.resize}
        onNavigateEdge={ctl.navigateEdge}
        onDeleteContents={ctl.deleteContents}
        onContextMenu={() => undefined}
        onCopy={ctl.onGridCopy}
        onCut={ctl.onGridCut}
        onPaste={ctl.onGridPaste}
      />
    </div>
  );
}
