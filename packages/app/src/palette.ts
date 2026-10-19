/**
 * Colours as the engine stores them (`0xRRGGBBAA` numbers): the swatches
 * of the font/fill colour pickers and the chip palette of "Colour by
 * value". Chip pairs are light fills with dark text, so they stay legible
 * on both the light and the dark theme.
 */

/** `#rrggbb` → `0xRRGGBBff`; `null` if not a 6-digit hex colour. */
export function hexToRgba(hex: string): number | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  return ((parseInt(m[1] ?? "0", 16) << 8) | 0xff) >>> 0;
}

/** `0xRRGGBBAA` → `#rrggbb`. */
export function rgbaToHex(rgba: number): string {
  return `#${((rgba >>> 8) & 0xffffff).toString(16).padStart(6, "0")}`;
}

/** Fully transparent: the "No fill" swatch (the patch cannot reset a fill to none). */
export const TRANSPARENT = 0xffffff00;

/** 24 swatches: a grey ramp, then saturated and light rows. */
export const SWATCHES: readonly string[] = [
  "#000000", "#374151", "#6b7280", "#9ca3af", "#d1d5db", "#f3f4f6", "#ffffff", "#7c2d12",
  "#dc2626", "#ea580c", "#d97706", "#16a34a", "#0d9488", "#2563eb", "#7c3aed", "#db2777",
  "#fee2e2", "#ffedd5", "#fef3c7", "#dcfce7", "#ccfbf1", "#dbeafe", "#ede9fe", "#fce7f3",
];

export interface ChipColors {
  name: string;
  bg: number;
  fg: number;
}

/** Status-chip colours (same family as the theme's chip tokens). */
export const CHIPS: readonly ChipColors[] = [
  { name: "green", bg: 0xdcfce7ff, fg: 0x166534ff },
  { name: "amber", bg: 0xfef3c7ff, fg: 0x92400eff },
  { name: "red", bg: 0xfee2e2ff, fg: 0x991b1bff },
  { name: "blue", bg: 0xdbeafeff, fg: 0x1e40afff },
  { name: "purple", bg: 0xede9feff, fg: 0x5b21b6ff },
  { name: "grey", bg: 0xf3f4f6ff, fg: 0x374151ff },
];

const SEMANTIC: readonly [RegExp, string][] = [
  [/^(paid|done|ok|yes|complete(d)?|active|approved|in stock|on track|won|pass(ed)?)$/i, "green"],
  [/^(due|pending|in progress|waiting|maybe|low|review|draft|partial|at risk)$/i, "amber"],
  [/^(overdue|late|failed|no|blocked|rejected|lost|out of stock|cancel(l)?ed|error)$/i, "red"],
];

/**
 * Assigns chip colours to distinct values: well-known status words get
 * their usual colour (Paid → green, Due → amber, Overdue → red); the rest
 * cycle through the palette in order of appearance, skipping colours
 * already taken while any are left.
 */
export function chipColorsFor(values: readonly string[]): Map<string, ChipColors> {
  const out = new Map<string, ChipColors>();
  const used = new Set<string>();
  const byName = (n: string): ChipColors => CHIPS.find((c) => c.name === n) ?? CHIPS[0]!;
  for (const v of values) {
    const hit = SEMANTIC.find(([re]) => re.test(v.trim()));
    if (hit && !used.has(hit[1])) {
      out.set(v, byName(hit[1]));
      used.add(hit[1]);
    }
  }
  let k = 0;
  for (const v of values) {
    if (out.has(v)) continue;
    const free = CHIPS.filter((c) => !used.has(c.name));
    const chip = free.length > 0 ? free[0]! : CHIPS[k++ % CHIPS.length]!;
    out.set(v, chip);
    used.add(chip.name);
  }
  return out;
}
