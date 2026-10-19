/**
 * React 19 wrapper. The `Grid` class is created once on mount; later prop
 * changes are pushed through `grid.update()` (only the keys that changed).
 * Scrolling never re-renders React.
 */
import { useEffect, useLayoutEffect, useRef, type CSSProperties, type Ref } from "react";
import { Grid as GridCore, type GridOptions } from "./grid";
import type { GridProps } from "./types";

export interface GridComponentProps extends GridProps {
  className?: string;
  style?: CSSProperties;
  /** Receives the underlying {@link GridCore} instance (or `null` on unmount). */
  gridRef?: Ref<GridCore | null>;
  /** Only read on mount. */
  options?: GridOptions;
}

const PROP_KEYS = [
  "source",
  "sheet",
  "theme",
  "zoom",
  "selection",
  "edit",
  "pointMode",
  "refHighlights",
  "editHighlights",
  "arrayRange",
  "clipRange",
  "filterButtons",
  "pageBreaks",
  "printRange",
  "onSelectionChange",
  "onStartEdit",
  "onEditChange",
  "onCommitEdit",
  "onCancelEdit",
  "onResize",
  "onAutoFit",
  "onFill",
  "onFillHandleDoubleClick",
  "onPointRange",
  "onEditorKeyDown",
  "onNavigateEdge",
  "onContextMenu",
  "onDeleteContents",
  "onCopy",
  "onCut",
  "onPaste",
  "onFilterButton",
  "onCellButton",
  "onOpenLink",
  "onNoteHover",
] as const satisfies readonly (keyof GridProps)[];

function pickProps(p: GridComponentProps): GridProps {
  const out: Partial<GridProps> = {};
  for (const k of PROP_KEYS) {
    if (p[k] !== undefined) (out as Record<string, unknown>)[k] = p[k];
  }
  return out as GridProps;
}

function setRef<T>(ref: Ref<T> | undefined, value: T): void {
  if (typeof ref === "function") ref(value);
  else if (ref) ref.current = value;
}

export function Grid(props: GridComponentProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const gridRef = useRef<GridCore | null>(null);
  const lastRef = useRef<GridProps | null>(null);
  const { gridRef: externalRef } = props;

  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const initial = pickProps(props);
    const grid = new GridCore(host, initial, props.options);
    gridRef.current = grid;
    lastRef.current = initial;
    setRef(externalRef, grid);
    return () => {
      grid.destroy();
      gridRef.current = null;
      lastRef.current = null;
      setRef(externalRef, null);
    };
    // Created once; later changes go through update().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const grid = gridRef.current;
    const last = lastRef.current;
    if (!grid || !last) return;
    const next = pickProps(props);
    const changed: Partial<GridProps> = {};
    let any = false;
    for (const k of PROP_KEYS) {
      if (next[k] !== last[k]) {
        (changed as Record<string, unknown>)[k] = next[k];
        any = true;
      }
    }
    if (any) {
      lastRef.current = next;
      grid.update(changed);
    }
  });

  return <div ref={hostRef} className={props.className} style={{ position: "relative", ...props.style }} />;
}

export { GridCore };
export type { GridProps };
