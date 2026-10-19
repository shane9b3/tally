/**
 * WP6 demo: the grid over a synthetic 1,048,576-row source (sync or with
 * simulated latency) and over MockClient through EngineClientSource. The
 * small controller here plays the role of @tally/app's stores.
 */
import "@tally/tokens/fonts";
import "@tally/tokens/tokens.css";
import { MockClient, type CellPos, type EngineClient, type Op, type StylePatch } from "@tally/bridge";
import { readTheme } from "@tally/tokens";
import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { runFling, type PerfResult } from "../scripts/perf";
import { cellAddress } from "../src/colLetters";
import { EngineClientSource } from "../src/engineSource";
import type { Grid as GridCore } from "../src/grid";
import { Grid } from "../src/react";
import { cellSelection } from "../src/selection";
import { SyntheticSource } from "../src/synthetic";
import type { CommitMove, Dir, EditState, GridDataSource, Selection } from "../src/types";

type Mode = "synthetic" | "async" | "mock";

interface Backend {
  source: GridDataSource;
  sheet: number;
  inputOf(cell: CellPos): Promise<string>;
  commit(cell: CellPos, text: string): void;
  clear(sel: Selection): void;
  resize(start: number, end: number, px: number): void;
  edge(from: CellPos, dir: Dir): Promise<CellPos>;
  copy(sel: Selection): Promise<string>;
  paste(sel: Selection, text: string): void;
}

function syntheticBackend(latencyMs: number): Backend {
  const src = new SyntheticSource(1_048_576, 100, { latencyMs });
  return {
    source: src,
    sheet: 1,
    inputOf: (c) => Promise.resolve(src.inputOf(c.r, c.c)),
    commit: (c, text) => src.setInput(c.r, c.c, text),
    clear: (sel) => src.clear(sel.range),
    resize: () => undefined,
    edge: (from, dir) => Promise.resolve(src.navigateEdge(from, dir)),
    copy: (sel) => {
      const rows: string[] = [];
      for (let r = sel.range.r0; r <= Math.min(sel.range.r1, sel.range.r0 + 999); r++) {
        const row: string[] = [];
        for (let c = sel.range.c0; c <= Math.min(sel.range.c1, 99); c++) row.push(src.inputOf(r, c));
        rows.push(row.join("\t"));
      }
      return Promise.resolve(rows.join("\n"));
    },
    paste: (sel, text) => {
      text.split(/\r?\n/).forEach((line, i) =>
        line.split("\t").forEach((v, j) => src.setInput(sel.active.r + i, sel.active.c + j, v)),
      );
    },
  };
}

const emptyPatch: StylePatch = {
  bold: null,
  italic: null,
  underline: null,
  strike: null,
  fontFamily: null,
  fontSizePt: null,
  color: null,
  fill: null,
  hAlign: null,
  vAlign: null,
  wrap: null,
  numberFormat: null,
};

async function mockBackend(): Promise<Backend> {
  const client: EngineClient = new MockClient();
  const info = await client.newWorkbook(null);
  const doc = info.doc;
  const sheet = info.activeSheet;
  const rows = [
    ["Invoice", "Customer", "Qty", "Price", "Total", "Paid", "Note"],
    ["1001", "Acme", "4", "2.5", "=C2*D2", "TRUE", "Long note that overflows into the empty cells to the right"],
    ["1002", "Globex", "10", "1.25", "=C3*D3", "FALSE", ""],
    ["1003", "Initech", "-3", "9.99", "=C4*D4", "TRUE", ""],
    ["1004", "Umbrella", "7", "3.14159265358979", "=C5*D5", "FALSE", ""],
    ["", "", "", "Sum", "=SUM(E2:E5)", "", ""],
  ];
  const ops: Op[] = [
    { type: "setRangeInputs", sheet, r: 0, c: 0, rows },
    { type: "applyStyle", sheet, ranges: [{ r0: 0, c0: 0, r1: 0, c1: 6 }], patch: { ...emptyPatch, bold: true, fill: 0xf3f4f6ff } },
    { type: "applyStyle", sheet, ranges: [{ r0: 5, c0: 3, r1: 5, c1: 4 }], patch: { ...emptyPatch, bold: true } },
    { type: "applyStyle", sheet, ranges: [{ r0: 1, c0: 6, r1: 1, c1: 6 }], patch: { ...emptyPatch, italic: true } },
    { type: "setColWidth", sheet, start: 1, end: 1, px: 130 },
  ];
  await client.apply(doc, { ops, label: "Seed", selectionBefore: null, selectionAfter: null });
  const source = new EngineClientSource(client, doc, sheet);
  const run = (op: Op, label: string): void => {
    client.apply(doc, { ops: [op], label, selectionBefore: null, selectionAfter: null }).then(
      (res) => source.applyResult(res),
      (e: unknown) => console.warn(e),
    );
  };
  return {
    source,
    sheet,
    inputOf: (c) => client.cellEditInfo(doc, sheet, c.r, c.c).then((i) => i.input),
    commit: (c, text) => run({ type: "setCellInput", sheet, r: c.r, c: c.c, input: text }, "Typing"),
    clear: (sel) => run({ type: "clear", sheet, ranges: [sel.range], what: "contents" }, "Delete"),
    resize: (start, end, px) => run({ type: "setColWidth", sheet, start, end, px }, "Column width"),
    edge: (from, dir) => client.navigateEdge(doc, sheet, from, dir),
    copy: (sel) => client.copy(doc, { sheet, ranges: [sel.range], active: sel.active }).then((o) => o.tsv),
    paste: (sel, text) => run({ type: "paste", sheet, target: sel.range, payload: { type: "tsv", text } }, "Paste"),
  };
}

function useBackend(mode: Mode): Backend | null {
  const [backend, setBackend] = useState<Backend | null>(null);
  useEffect(() => {
    let live = true;
    setBackend(null);
    if (mode === "mock") void mockBackend().then((b) => live && setBackend(b));
    else setBackend(syntheticBackend(mode === "async" ? 40 : 0));
    return () => {
      live = false;
    };
  }, [mode]);
  return backend;
}

function App() {
  const [mode, setMode] = useState<Mode>("synthetic");
  const backend = useBackend(mode);
  const theme = useMemo(() => readTheme(), []);
  const [zoom, setZoom] = useState(1);
  const [selection, setSelection] = useState<Selection>(cellSelection({ r: 0, c: 0 }));
  const [edit, setEdit] = useState<EditState | null>(null);
  const [perf, setPerf] = useState<PerfResult | null>(null);
  const [menu, setMenu] = useState<string>("");
  const gridRef = useRef<GridCore | null>(null);
  const editRef = useRef<EditState | null>(null);
  editRef.current = edit;

  useEffect(() => {
    setSelection(cellSelection({ r: 0, c: 0 }));
    setEdit(null);
  }, [mode]);

  const fling = useCallback(async () => {
    const g = gridRef.current;
    if (!g) return;
    const res = await runFling(g);
    setPerf(res);
    (window as unknown as { __tallyPerf?: PerfResult }).__tallyPerf = res;
    console.info("fling", res);
  }, []);

  useEffect(() => {
    if (backend && new URLSearchParams(location.search).has("autorun")) {
      const t = setTimeout(() => void fling(), 1500);
      return () => clearTimeout(t);
    }
    return undefined;
  }, [backend, fling]);

  const onStartEdit = useCallback(
    (cell: CellPos, seed?: string) => {
      if (seed !== undefined) {
        setEdit({ cell, text: seed, caret: seed.length });
        return;
      }
      setEdit({ cell, text: "", caret: 0 });
      void backend?.inputOf(cell).then((text) => {
        const cur = editRef.current;
        if (cur && cur.cell.r === cell.r && cur.cell.c === cell.c && cur.text === "") setEdit({ cell, text, caret: text.length });
      });
    },
    [backend],
  );
  const onCommitEdit = useCallback(
    (_move: CommitMove) => {
      const cur = editRef.current;
      if (cur && backend) backend.commit(cur.cell, cur.text);
      setEdit(null);
    },
    [backend],
  );

  if (!backend) return <div style={{ padding: 16 }}>Loading…</div>;
  const r = selection.range;
  const addr =
    r.r0 === r.r1 && r.c0 === r.c1 ? cellAddress(r.r0, r.c0) : `${cellAddress(r.r0, r.c0)}:${cellAddress(r.r1, r.c1)}`;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", gap: 12, alignItems: "center", padding: "8px 12px", fontSize: 13, borderBottom: "1px solid var(--tally-border-subtle)", background: "var(--tally-bg)" }}>
        <strong>Tally grid</strong>
        {(["synthetic", "async", "mock"] as const).map((m) => (
          <button key={m} onClick={() => setMode(m)} style={{ fontWeight: m === mode ? 600 : 400 }}>
            {m === "synthetic" ? "Synthetic 1M rows" : m === "async" ? "Synthetic, 40 ms latency" : "MockClient"}
          </button>
        ))}
        <label>
          Zoom{" "}
          <select value={zoom} onChange={(e) => setZoom(Number(e.target.value))}>
            {[0.75, 1, 1.25, 1.5, 2].map((z) => (
              <option key={z} value={z}>
                {z * 100}%
              </option>
            ))}
          </select>
        </label>
        <button onClick={() => void fling()}>Fling test</button>
        <span style={{ fontVariantNumeric: "tabular-nums", color: "var(--tally-text-muted)" }}>
          {addr}
          {edit ? ` · editing "${edit.text}"` : ""}
          {perf ? ` · ${perf.frames} frames, p50 ${perf.p50.toFixed(1)} ms, p95 ${perf.p95.toFixed(1)} ms, max ${perf.max.toFixed(1)} ms, draw avg ${perf.drawAvg.toFixed(2)} ms` : ""}
          {menu ? ` · ${menu}` : ""}
        </span>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Grid
          key={mode}
          gridRef={gridRef}
          style={{ width: "100%", height: "100%" }}
          source={backend.source}
          sheet={backend.sheet}
          theme={theme}
          zoom={zoom}
          selection={selection}
          edit={edit}
          onSelectionChange={setSelection}
          onStartEdit={onStartEdit}
          onEditChange={(text, caret) => setEdit((e) => (e ? { ...e, text, caret } : e))}
          onCommitEdit={onCommitEdit}
          onCancelEdit={() => setEdit(null)}
          onResize={(_axis, start, end, px) => backend.resize(start, end, px)}
          onNavigateEdge={(from, dir) => backend.edge(from, dir)}
          onContextMenu={(cell, x, y) => setMenu(`context menu at ${cellAddress(cell.r, cell.c)} (${x}, ${y})`)}
          onDeleteContents={() => backend.clear(selection)}
          onCopy={() => void backend.copy(selection).then((t) => navigator.clipboard?.writeText(t))}
          onCut={() => {
            void backend.copy(selection).then((t) => navigator.clipboard?.writeText(t));
            backend.clear(selection);
          }}
          onPaste={(data) => {
            const text = data?.getData("text/plain") ?? "";
            if (text) backend.paste(selection, text);
          }}
        />
      </div>
    </div>
  );
}

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
