import type { GridGeometryInfo } from "@tally/grid";

/** Screen rect of an object node (CSS px from the grid's top-left) and whether it intersects the viewport. */
export function nodeVisible(rect: { x: number; y: number; w: number; h: number }, geo: GridGeometryInfo): boolean {
  const frozenW = geo.frozen.cols > 0 ? geo.colLeft(geo.frozen.cols) - geo.headerW : 0;
  const frozenH = geo.frozen.rows > 0 ? geo.rowTop(geo.frozen.rows) - geo.headerH : 0;
  const sx = geo.headerW + rect.x - geo.scrollX;
  const sy = geo.headerH + rect.y - geo.scrollY;
  return sx + rect.w > geo.headerW + frozenW && sx < geo.width && sy + rect.h > geo.headerH + frozenH && sy < geo.height;
}
