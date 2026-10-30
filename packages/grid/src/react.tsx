/**
 * React 19 wrapper. The `Grid` class is created once on mount; later prop
 * changes are pushed through `grid.update()` (only the keys that changed).
 * Scrolling never re-renders React.
 */
import { useEffect, useLayoutEffect, useRef, type CSSProperties, type Ref } from "react";
import { Grid as GridCore, type GridOptions } from "./grid";
import { SplitView, type SplitAxis } from "./split";
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
  "editElsewhere",
  "refHighlights",
  "editHighlights",
  "arrayRange",
  "clipRange",
  "filterButtons",
  "pageBreaks",
  "printRange",
  "protectedNav",
  "onNextUnlocked",
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
  // wave 5 seams
  "rangePick",
  "outline",
  "pageBreakPreview",
  "traces",
  "aria",
  "objects",
  "onViewportChange",
  "onOutlineToggle",
  "onOutlineLevel",
  "onObjectSelect",
  "onObjectPointerDown",
  "onObjectDoubleClick",
  "onObjectKey",
  "invalidCells",
  "onPageBreakDrag",
  "showGridlines",
  "showHeaders",
  "decimalSep",

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

// ---------------------------------------------------------------------------
// Split window (W5-5, §4 item 30)
// ---------------------------------------------------------------------------


export interface SplitGridProps extends GridComponentProps {
  /** `"vertical"`: panes left/right; `"horizontal"`: panes top/bottom. */
  axis: SplitAxis;
  /** Share of the first pane (0.1–0.9). */
  ratio: number;
  /** The divider was dragged. */
  onRatioChange?: (ratio: number) => void;
  /** Receives the {@link SplitView} (or `null` on unmount). */
  splitRef?: Ref<SplitView | null>;
}

/**
 * Two grids over the same source with a draggable divider ({@link SplitView}).
 * `gridRef` receives the primary (first) pane; prop changes reach both panes.
 */
export function SplitGrid(props: SplitGridProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<SplitView | null>(null);
  const lastRef = useRef<GridProps | null>(null);
  const ratioCb = useRef(props.onRatioChange);
  ratioCb.current = props.onRatioChange;
  const { gridRef: externalRef, splitRef } = props;

  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const initial = pickProps(props);
    const view = new SplitView(host, initial, {
      axis: props.axis,
      ratio: props.ratio,
      grid: props.options,
      onRatioChange: (r) => ratioCb.current?.(r),
    });
    viewRef.current = view;
    lastRef.current = initial;
    setRef(externalRef, view.primary);
    setRef(splitRef, view);
    return () => {
      view.destroy();
      viewRef.current = null;
      lastRef.current = null;
      setRef(externalRef, null);
      setRef(splitRef, null);
    };
    // Created once; later changes go through update().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    const last = lastRef.current;
    if (!view || !last) return;
    view.setAxis(props.axis);
    view.setRatio(props.ratio);
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
      view.update(changed);
    }
  });

  return <div ref={hostRef} className={props.className} style={{ position: "relative", ...props.style }} />;
}
