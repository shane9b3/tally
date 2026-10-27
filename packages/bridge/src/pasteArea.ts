/**
 * The cells a paste writes, as the engine works it out (`clipboard::paste_area`):
 * external TSV text sits at the top-left and is never tiled; an internal
 * block is tiled over a target that is a whole multiple of its size.
 */
import type { PasteOptions } from "./generated/PasteOptions";
import type { PastePayload } from "./generated/PastePayload";
import type { RangeDto } from "./generated/RangeDto";

/** Rows and columns of a TSV text (every line is a row, the widest line sets the columns). */
export function tsvDims(text: string): { rows: number; cols: number } | null {
  if (text === "") return null;
  const lines = text.replace(/\r?\n$/, "").split(/\r?\n/);
  return { rows: lines.length, cols: Math.max(1, ...lines.map((l) => l.split("\t").length)) };
}

/** `clip` is the size of the internal block the payload's token names (`null`: token unknown). */
export function pasteArea(target: RangeDto, payload: PastePayload, options: Pick<PasteOptions, "transpose">, clip: { rows: number; cols: number } | null): RangeDto | null {
  const swap = <T extends { rows: number; cols: number }>(d: T) => (options.transpose ? { rows: d.cols, cols: d.rows } : { rows: d.rows, cols: d.cols });
  if (payload.type === "tsv") {
    const d = tsvDims(payload.text);
    if (!d) return null;
    const { rows, cols } = swap(d);
    return { r0: target.r0, c0: target.c0, r1: target.r0 + rows - 1, c1: target.c0 + cols - 1 };
  }
  if (!clip) return null;
  const { rows, cols } = swap({ rows: Math.max(1, clip.rows), cols: Math.max(1, clip.cols) });
  const th = target.r1 - target.r0 + 1;
  const tw = target.c1 - target.c0 + 1;
  const repsR = th % rows === 0 ? th / rows : 1;
  const repsC = tw % cols === 0 ? tw / cols : 1;
  return { r0: target.r0, c0: target.c0, r1: target.r0 + rows * repsR - 1, c1: target.c0 + cols * repsC - 1 };
}
