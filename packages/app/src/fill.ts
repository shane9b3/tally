/**
 * Fill handle (AutoFill), ⌘D and ⌘R: one engine `fillRange` op. The fill
 * handle uses mode `auto` (the engine continues numbers, dates, sort-list
 * entries and "Item 1" text, and copies formulas re-anchored); ⌘D / ⌘R use
 * mode `copy` (AOO Edit ▸ Fill ▸ Down / Right is `FillSimple`: a copy, with
 * relative references adjusted). The browser demo engine emulates the op
 * (dev/mockFill.ts).
 */
import type { CellView, FillMode, Op, RangeDto } from "@tally/bridge";
import type { FillDir } from "@tally/grid";

export interface FillContext {
  sheet: number;
}

/** The cells added by a fill: `target` minus `source` (one rectangle). */
export function extensionOf(source: RangeDto, target: RangeDto, dir: FillDir): RangeDto {
  switch (dir) {
    case "down":
      return { ...target, r0: source.r1 + 1 };
    case "up":
      return { ...target, r1: source.r0 - 1 };
    case "right":
      return { ...target, c0: source.c1 + 1 };
    case "left":
      return { ...target, c1: source.c0 - 1 };
  }
}

/**
 * The mode of a fill-handle drag: AutoFill, or with the copy modifier
 * (⌘ / Ctrl held on release; AOO `KEY_MOD1`) a copy that, like AutoFill,
 * leaves filtered rows alone.
 */
export function fillHandleMode(copy: boolean): FillMode {
  return copy ? "autoCopy" : "auto";
}

/** The op that fills `target` (which contains `source` and extends it along one axis). */
export function fillRange(ctx: FillContext, source: RangeDto, target: RangeDto, mode: FillMode = "auto"): Op[] {
  if (source.r0 === target.r0 && source.c0 === target.c0 && source.r1 === target.r1 && source.c1 === target.c1) return [];
  return [{ type: "fillRange", sheet: ctx.sheet, source: { ...source }, target: { ...target }, mode }];
}

/**
 * Double-click on the fill handle: how far down to fill. Scans the column
 * left of the source (else right of it) from the source's top for its
 * contiguous data; returns the last row, or `null` if that would not
 * extend the source. `get(r, c)` is the fetched cell (undefined = empty).
 */
export function fillDownEnd(source: RangeDto, get: (r: number, c: number) => CellView | undefined, maxRows: number): number | null {
  const filled = (r: number, c: number): boolean => {
    const v = get(r, c);
    return v !== undefined && v.kind !== "empty" && v.text !== "";
  };
  for (const c of [source.c0 - 1, source.c1 + 1]) {
    if (c < 0) continue;
    if (!filled(source.r1, c) && !filled(source.r1 + 1, c)) continue;
    let r = source.r1;
    while (r + 1 < source.r0 + maxRows && filled(r + 1, c)) r++;
    // Stop above existing data in the source column itself.
    for (let k = source.r1 + 1; k <= r; k++) {
      if (filled(k, source.c0)) {
        r = k - 1;
        break;
      }
    }
    return r > source.r1 ? r : null;
  }
  return null;
}
