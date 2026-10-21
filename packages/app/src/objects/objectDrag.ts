/**
 * Pure drag/resize maths for cell-anchored objects (W5-5): a pointer drag
 * from one of the layer's 8 handles (or the body) turns the object's start
 * rect into a new rect; arrow keys nudge an anchor by millimetres.
 */
import { anchorToPx, pxToAnchor, type AnchorAxes, type ObjectAnchor, type ObjectHandle } from "@tally/grid";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Smallest object size while resizing (CSS px). */
export const MIN_OBJECT_PX = 24;

/** The rect after dragging `part` by `(dx, dy)` px from `start`. */
export function dragRect(start: Rect, part: "body" | ObjectHandle, dx: number, dy: number, minPx = MIN_OBJECT_PX): Rect {
  if (part === "body") return { x: Math.max(0, start.x + dx), y: Math.max(0, start.y + dy), w: start.w, h: start.h };
  let { x, y, w, h } = start;
  if (part.includes("e")) w = Math.max(minPx, start.w + dx);
  if (part.includes("s")) h = Math.max(minPx, start.h + dy);
  if (part.includes("w")) {
    const nw = Math.max(minPx, start.w - dx);
    x = Math.max(0, start.x + (start.w - nw));
    w = nw;
  }
  if (part.includes("n")) {
    const nh = Math.max(minPx, start.h - dy);
    y = Math.max(0, start.y + (start.h - nh));
    h = nh;
  }
  return { x, y, w, h };
}

/** Nudges an anchor by an arrow key: 1 mm, or 10 mm with Shift. */
export function nudgeAnchor(anchor: ObjectAnchor, key: string, shift: boolean, axes: AnchorAxes): ObjectAnchor | null {
  const step = shift ? 10 : 1;
  const d = key === "ArrowLeft" ? [-step, 0] : key === "ArrowRight" ? [step, 0] : key === "ArrowUp" ? [0, -step] : key === "ArrowDown" ? [0, step] : null;
  if (!d) return null;
  const k = (96 / 25.4) * axes.zoom;
  const rect = anchorToPx(anchor, axes);
  const next = pxToAnchor({ ...rect, x: Math.max(0, rect.x + (d[0] ?? 0) * k), y: Math.max(0, rect.y + (d[1] ?? 0) * k) }, axes);
  return { ...next, widthMm: anchor.widthMm, heightMm: anchor.heightMm };
}

/** Rounds an anchor's millimetre fields (what we send to the engine). */
export function roundAnchor(a: ObjectAnchor): ObjectAnchor {
  const r = (v: number) => Math.round(v * 100) / 100;
  return { r: a.r, c: a.c, dxMm: r(a.dxMm), dyMm: r(a.dyMm), widthMm: r(a.widthMm), heightMm: r(a.heightMm) };
}

/** True when two anchors differ in any field (after rounding). */
export function anchorChanged(a: ObjectAnchor, b: ObjectAnchor): boolean {
  const ra = roundAnchor(a);
  const rb = roundAnchor(b);
  return ra.r !== rb.r || ra.c !== rb.c || ra.dxMm !== rb.dxMm || ra.dyMm !== rb.dyMm || ra.widthMm !== rb.widthMm || ra.heightMm !== rb.heightMm;
}
