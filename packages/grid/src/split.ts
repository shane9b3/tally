/**
 * Split window (wave 5 seam, W5-0b; §4 item 30 fills it in): two
 * independent {@link Grid} viewports over the same {@link GridDataSource},
 * side by side (`axis: "vertical"`, a vertical divider) or stacked
 * (`axis: "horizontal"`). Each pane scrolls on its own; props pushed through
 * {@link SplitView.update} reach both. The divider drags to move the split.
 *
 *     const split = new SplitView(container, props, { axis: "horizontal", ratio: 0.5 });
 *     split.update({ selection });
 *     split.destroy();
 *
 * Persisting the split (`setSplit { xPx, yPx }`) and the View ▸ Split Window
 * command are W5-5's; this is only the layout.
 */
import { Grid, type GridOptions } from "./grid";
import type { GridProps } from "./types";

export type SplitAxis = "vertical" | "horizontal";

export interface SplitOptions {
  /** `"vertical"`: panes left/right; `"horizontal"`: panes top/bottom. */
  axis: SplitAxis;
  /** Share of the first pane, 0.1–0.9 (default 0.5). */
  ratio?: number;
  /** Divider thickness in CSS px (default 5). */
  dividerPx?: number;
  /** Grid limits for both panes. */
  grid?: GridOptions;
  /** The split moved (after a divider drag). */
  onRatioChange?(ratio: number): void;
}

const MIN_RATIO = 0.1;
const MAX_RATIO = 0.9;

export function clampRatio(r: number): number {
  return Number.isFinite(r) ? Math.min(MAX_RATIO, Math.max(MIN_RATIO, r)) : 0.5;
}

export class SplitView {
  readonly root: HTMLDivElement;
  readonly first: HTMLDivElement;
  readonly second: HTMLDivElement;
  readonly divider: HTMLDivElement;
  readonly primary: Grid;
  readonly secondary: Grid;
  private axisValue: SplitAxis;
  private ratioValue: number;
  private readonly dividerPx: number;
  private readonly onRatioChange: ((ratio: number) => void) | undefined;
  private dragging = false;
  private destroyed = false;

  constructor(container: HTMLElement, props: GridProps, opts: SplitOptions) {
    const doc = container.ownerDocument;
    this.axisValue = opts.axis;
    this.ratioValue = clampRatio(opts.ratio ?? 0.5);
    this.dividerPx = opts.dividerPx ?? 5;
    this.onRatioChange = opts.onRatioChange;

    const root = doc.createElement("div");
    root.className = "tally-split";
    root.dataset.axis = opts.axis;
    Object.assign(root.style, { position: "relative", width: "100%", height: "100%", display: "flex", overflow: "hidden" } satisfies Partial<CSSStyleDeclaration>);
    const pane = (name: string): HTMLDivElement => {
      const d = doc.createElement("div");
      d.className = `tally-split-pane tally-split-${name}`;
      Object.assign(d.style, { position: "relative", minWidth: "0", minHeight: "0", overflow: "hidden" } satisfies Partial<CSSStyleDeclaration>);
      return d;
    };
    const first = pane("first");
    const second = pane("second");
    const divider = doc.createElement("div");
    divider.className = "tally-split-divider";
    divider.setAttribute("role", "separator");
    divider.setAttribute("aria-orientation", opts.axis);
    divider.tabIndex = 0;
    Object.assign(divider.style, { flex: "none", background: props.theme.colors.frozenLine, touchAction: "none", userSelect: "none" } satisfies Partial<CSSStyleDeclaration>);
    root.append(first, divider, second);
    container.appendChild(root);
    this.root = root;
    this.first = first;
    this.second = second;
    this.divider = divider;
    this.layout();

    this.primary = new Grid(first, props, opts.grid);
    this.secondary = new Grid(second, props, opts.grid);

    divider.addEventListener("pointerdown", this.onDown);
    divider.addEventListener("pointermove", this.onMove);
    divider.addEventListener("pointerup", this.onUp);
    divider.addEventListener("pointercancel", this.onUp);
    divider.addEventListener("keydown", this.onKey);
  }

  get axis(): SplitAxis {
    return this.axisValue;
  }

  get ratio(): number {
    return this.ratioValue;
  }

  /** Pushes prop changes to both panes. */
  update(partial: Partial<GridProps>): void {
    this.primary.update(partial);
    this.secondary.update(partial);
  }

  setRatio(ratio: number): void {
    const r = clampRatio(ratio);
    if (r === this.ratioValue) return;
    this.ratioValue = r;
    this.layout();
    this.onRatioChange?.(r);
  }

  setAxis(axis: SplitAxis): void {
    if (axis === this.axisValue) return;
    this.axisValue = axis;
    this.root.dataset.axis = axis;
    this.divider.setAttribute("aria-orientation", axis);
    this.layout();
  }

  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.primary.destroy();
    this.secondary.destroy();
    this.root.remove();
  }

  private layout(): void {
    const vertical = this.axisValue === "vertical";
    this.root.style.flexDirection = vertical ? "row" : "column";
    const pct = `${(this.ratioValue * 100).toFixed(3)}%`;
    this.first.style.flex = `0 0 calc(${pct} - ${this.dividerPx / 2}px)`;
    this.second.style.flex = "1 1 0";
    const d = this.divider.style;
    d.width = vertical ? `${this.dividerPx}px` : "100%";
    d.height = vertical ? "100%" : `${this.dividerPx}px`;
    d.cursor = vertical ? "col-resize" : "row-resize";
    this.divider.setAttribute("aria-valuenow", String(Math.round(this.ratioValue * 100)));
  }

  private ratioFromEvent(e: PointerEvent): number {
    const r = this.root.getBoundingClientRect();
    return this.axisValue === "vertical" ? (e.clientX - r.left) / Math.max(1, r.width) : (e.clientY - r.top) / Math.max(1, r.height);
  }

  private readonly onDown = (e: PointerEvent): void => {
    if (e.button !== 0) return;
    this.dragging = true;
    try {
      this.divider.setPointerCapture(e.pointerId);
    } catch {
      // Synthetic events in tests have no active pointer.
    }
    e.preventDefault();
  };

  private readonly onMove = (e: PointerEvent): void => {
    if (!this.dragging) return;
    this.setRatio(this.ratioFromEvent(e));
  };

  private readonly onUp = (e: PointerEvent): void => {
    if (!this.dragging) return;
    this.dragging = false;
    try {
      this.divider.releasePointerCapture(e.pointerId);
    } catch {
      // ignore
    }
  };

  private readonly onKey = (e: KeyboardEvent): void => {
    const vertical = this.axisValue === "vertical";
    const dec = vertical ? "ArrowLeft" : "ArrowUp";
    const inc = vertical ? "ArrowRight" : "ArrowDown";
    if (e.key === dec || e.key === inc) {
      e.preventDefault();
      this.setRatio(this.ratioValue + (e.key === inc ? 0.05 : -0.05));
    }
  };
}
