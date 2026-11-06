/**
 * `W5Features`: the W5-5 half of the controller (charts and pictures on the
 * object layer, printing and PDF, the split window, Page Break Preview, the
 * detective, zoom presets and full screen). One instance per controller
 * (`ctl.w5`); it talks to the engine only through the controller.
 */
import type { ApplyResult, Op } from "@tally/bridge";
import type { Grid as GridCore } from "@tally/grid";
import { ChartFeatures } from "../charts/chartFeatures";
import { useChartStore } from "../charts/chartStore";
import type { AppController } from "../controller";
import { Detective } from "../detective/detective";
import { PrintFeatures } from "../print/printFeatures";
import { ObjectFeatures } from "./objectFeatures";
import { ViewFeatures } from "./viewFeatures";
import { useW5Store } from "./w5Store";

export class W5Features {
  readonly objects: ObjectFeatures;
  readonly charts: ChartFeatures;
  readonly print: PrintFeatures;
  readonly view: ViewFeatures;
  readonly detective: Detective;
  /** The second pane of Window ▸ Split (`SplitGrid`'s secondary grid), or `null`; `ctl.grid` is the first. */
  secondaryGrid: GridCore | null = null;

  constructor(ctl: AppController) {
    this.objects = new ObjectFeatures(ctl);
    this.charts = new ChartFeatures(ctl, this.objects);
    this.print = new PrintFeatures(ctl, this.charts);
    this.view = new ViewFeatures(ctl);
    this.detective = new Detective(ctl);
  }

  /** `SplitGrid`'s `splitRef`: remembers the second pane so charts mount there too. */
  setSplitView = (view: { secondary: GridCore } | null): void => {
    this.secondaryGrid = view?.secondary ?? null;
    useChartStore.getState().bumpViewport();
  };

  /** A document was loaded or the visible sheet changed. */
  refreshSheet(): void {
    useW5Store.getState().resetSheet();
    useChartStore.getState().reset();
    void this.objects.refreshObjects();
    void this.view.refreshPageBreaks();
    // The sheet's detective arrows (they live in the engine and the file).
    this.detective.onSheet();
  }

  /** After an engine result (`AppController.handleResult`). */
  afterResult(res: ApplyResult, ops: readonly Op[] | null): void {
    this.objects.afterResult(res, ops);
    const w5 = useW5Store.getState();
    if (w5.pageBreakPreview && (ops === null || ops.some((o) => o.type === "setPageBreak" || o.type === "setPageSetup") || res.layoutChanged.length > 0 || res.changed.length > 0)) {
      void this.view.refreshPageBreaks();
    }
    // AutoRefresh redrew the arrows, or rows/columns moved them (the engine keeps them).
    this.detective.afterResult(res.detectiveChanged);
    // "Mark invalid data" circles point at cells that may have moved: structural edits clear them.
    if (ops?.some((o) => ["insertRows", "deleteRows", "insertCols", "deleteCols", "moveRange", "sortRange"].includes(o.type))) useW5Store.getState().setInvalidCells([]);
  }

  /** A snapshot of the view state (e2e hook, status readouts). */
  state(): { traces: number; invalidCells: number; split: string | null; pageBreakPreview: boolean; detectiveFill: boolean } {
    const w5 = useW5Store.getState();
    return { traces: w5.traces.length, invalidCells: w5.invalidCells.length, split: w5.split?.axis ?? null, pageBreakPreview: w5.pageBreakPreview, detectiveFill: w5.detectiveFill };
  }

  /** ⌘C/⌘X while a chart is selected go to the chart; returns true when handled. */
  async clipboard(action: "copy" | "cut"): Promise<boolean> {
    if (useChartStore.getState().selected === null) return false;
    return action === "copy" ? this.charts.copySelected() : this.charts.cutSelected();
  }

  /**
   * ⌘V pastes the copied chart (at the active cell) when a chart was copied
   * and either a chart is selected or the system clipboard carries no text
   * (the PNG we wrote has none).
   */
  wantsChartPaste(text: string): boolean {
    const st = useChartStore.getState();
    return st.clipboard !== null && (st.selected !== null || text === "");
  }
}
