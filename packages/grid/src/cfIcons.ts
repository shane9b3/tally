/**
 * Conditional-format decorations drawn on the canvas: icon-set glyphs
 * (arrows, traffic lights, flags, signs, symbols, stars, triangles, rating
 * bars, quarters, boxes) and data bars. Simple vector shapes, no image
 * assets; `index` 0 is the lowest icon of the set (already reversed by the
 * engine when the rule says so).
 */
import type { DataBarView, IconSetKind, IconView } from "@tally/bridge";

/** Icon colours (the same family as the status chips). */
export const ICON_COLORS = {
  green: "#16a34a",
  amber: "#f59e0b",
  red: "#dc2626",
  grey: "#9ca3af",
  dark: "#374151",
  gold: "#f5b301",
  blue: "#2563eb",
  pink: "#f9a8b4",
} as const;

/** Number of icons in a set. */
export function iconCount(set: IconSetKind): number {
  if (set.startsWith("three")) return 3;
  if (set.startsWith("four")) return 4;
  return 5;
}

/** 0 (lowest) … 1 (highest) for an icon of a set. */
function level(icon: IconView): number {
  const n = iconCount(icon.set);
  return n <= 1 ? 1 : Math.max(0, Math.min(1, icon.index / (n - 1)));
}

/** Red → amber → green for low → high. */
function traffic(icon: IconView): string {
  const t = level(icon);
  return t < 0.34 ? ICON_COLORS.red : t < 0.67 ? ICON_COLORS.amber : ICON_COLORS.green;
}

/** The arrow angle: up for the highest, down for the lowest (radians, 0 = right). */
function arrowAngle(icon: IconView): number {
  // 3 arrows: down, right, up; 4/5: also the diagonals.
  return -Math.PI / 2 + Math.PI * (1 - level(icon));
}

function arrow(ctx: CanvasRenderingContext2D, cx: number, cy: number, s: number, angle: number, color: string): void {
  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(angle);
  ctx.fillStyle = color;
  const h = s / 2;
  ctx.beginPath();
  ctx.moveTo(h, 0);
  ctx.lineTo(0, -h * 0.85);
  ctx.lineTo(0, -h * 0.32);
  ctx.lineTo(-h, -h * 0.32);
  ctx.lineTo(-h, h * 0.32);
  ctx.lineTo(0, h * 0.32);
  ctx.lineTo(0, h * 0.85);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function circle(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, color: string, rim?: string): void {
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();
  if (rim) {
    ctx.lineWidth = Math.max(1, r * 0.22);
    ctx.strokeStyle = rim;
    ctx.stroke();
  }
}

function star(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, fill: number): void {
  const path = (): void => {
    ctx.beginPath();
    for (let i = 0; i < 10; i++) {
      const a = -Math.PI / 2 + (i * Math.PI) / 5;
      const rr = i % 2 === 0 ? r : r * 0.45;
      const x = cx + rr * Math.cos(a);
      const y = cy + rr * Math.sin(a);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.closePath();
  };
  path();
  ctx.fillStyle = "#e5e7eb";
  ctx.fill();
  if (fill > 0) {
    ctx.save();
    path();
    ctx.clip();
    ctx.fillStyle = ICON_COLORS.gold;
    ctx.fillRect(cx - r, cy - r, 2 * r * fill, 2 * r);
    ctx.restore();
  }
  path();
  ctx.lineWidth = Math.max(0.75, r * 0.12);
  ctx.strokeStyle = fill > 0 ? "#d69e00" : ICON_COLORS.grey;
  ctx.stroke();
}

function triangle(ctx: CanvasRenderingContext2D, cx: number, cy: number, s: number, up: boolean, color: string): void {
  const h = s / 2;
  ctx.beginPath();
  if (up) {
    ctx.moveTo(cx, cy - h * 0.8);
    ctx.lineTo(cx + h, cy + h * 0.7);
    ctx.lineTo(cx - h, cy + h * 0.7);
  } else {
    ctx.moveTo(cx, cy + h * 0.8);
    ctx.lineTo(cx + h, cy - h * 0.7);
    ctx.lineTo(cx - h, cy - h * 0.7);
  }
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
}

function flag(ctx: CanvasRenderingContext2D, cx: number, cy: number, s: number, color: string): void {
  const h = s / 2;
  ctx.fillStyle = ICON_COLORS.dark;
  ctx.fillRect(cx - h * 0.7, cy - h, Math.max(1, s * 0.1), s);
  ctx.beginPath();
  ctx.moveTo(cx - h * 0.6, cy - h);
  ctx.lineTo(cx + h * 0.85, cy - h * 0.45);
  ctx.lineTo(cx - h * 0.6, cy + h * 0.1);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
}

/** ✕, ! or ✓ strokes. */
function symbol(ctx: CanvasRenderingContext2D, cx: number, cy: number, s: number, kind: "x" | "!" | "v", color: string): void {
  const h = s * 0.28;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = Math.max(1.2, s * 0.16);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  if (kind === "x") {
    ctx.moveTo(cx - h, cy - h);
    ctx.lineTo(cx + h, cy + h);
    ctx.moveTo(cx + h, cy - h);
    ctx.lineTo(cx - h, cy + h);
    ctx.stroke();
  } else if (kind === "v") {
    ctx.moveTo(cx - h * 1.1, cy + h * 0.05);
    ctx.lineTo(cx - h * 0.25, cy + h * 0.85);
    ctx.lineTo(cx + h * 1.15, cy - h * 0.9);
    ctx.stroke();
  } else {
    ctx.moveTo(cx, cy - h * 1.15);
    ctx.lineTo(cx, cy + h * 0.25);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(cx, cy + h * 1.05, ctx.lineWidth * 0.55, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

/** Signal-strength bars: `filled` of `n` bars, rising left to right. */
function bars(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, n: number, filled: number): void {
  const gap = Math.max(1, s * 0.08);
  const w = (s - gap * (n - 1)) / n;
  for (let i = 0; i < n; i++) {
    const h = (s * (i + 1)) / n;
    ctx.fillStyle = i < filled ? ICON_COLORS.blue : "#d1d5db";
    ctx.fillRect(x + i * (w + gap), y + s - h, w, h);
  }
}

/** A pie with `quarters` of 4 filled. */
function quarters(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, q: number): void {
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.fillStyle = "#ffffff";
  ctx.fill();
  if (q > 0) {
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.arc(cx, cy, r, -Math.PI / 2, -Math.PI / 2 + (Math.PI * 2 * q) / 4);
    ctx.closePath();
    ctx.fillStyle = ICON_COLORS.dark;
    ctx.fill();
  }
  ctx.beginPath();
  ctx.arc(cx, cy, r, 0, Math.PI * 2);
  ctx.lineWidth = Math.max(1, r * 0.18);
  ctx.strokeStyle = ICON_COLORS.dark;
  ctx.stroke();
}

/** A 2 × 2 grid with `filled` of 4 boxes. */
function boxes(ctx: CanvasRenderingContext2D, x: number, y: number, s: number, filled: number): void {
  const gap = Math.max(1, s * 0.1);
  const w = (s - gap) / 2;
  const order: [number, number][] = [
    [0, 1],
    [1, 1],
    [0, 0],
    [1, 0],
  ];
  order.forEach(([i, j], k) => {
    ctx.fillStyle = k < filled ? ICON_COLORS.blue : "#d1d5db";
    ctx.fillRect(x + i * (w + gap), y + j * (w + gap), w, w);
  });
}

/**
 * Draws `icon` in the square at (x, y) with side `size` (screen px).
 */
export function drawCfIcon(ctx: CanvasRenderingContext2D, icon: IconView, x: number, y: number, size: number): void {
  const cx = x + size / 2;
  const cy = y + size / 2;
  const r = size / 2;
  const t = level(icon);
  const n = iconCount(icon.set);
  ctx.save();
  switch (icon.set) {
    case "threeArrows":
    case "fourArrows":
    case "fiveArrows":
      arrow(ctx, cx, cy, size * 0.92, arrowAngle(icon), traffic(icon));
      break;
    case "threeArrowsGray":
    case "fourArrowsGray":
    case "fiveArrowsGray":
      arrow(ctx, cx, cy, size * 0.92, arrowAngle(icon), ICON_COLORS.grey);
      break;
    case "threeFlags":
      flag(ctx, cx, cy, size * 0.9, traffic(icon));
      break;
    case "threeTrafficLights":
    case "fourTrafficLights": {
      const color = n === 4 && icon.index === 0 ? ICON_COLORS.dark : traffic(icon);
      circle(ctx, cx, cy, r * 0.82, color);
      break;
    }
    case "threeTrafficLightsRimmed":
      circle(ctx, cx, cy, r * 0.72, traffic(icon), ICON_COLORS.dark);
      break;
    case "fourRedToBlack": {
      const colors = [ICON_COLORS.dark, ICON_COLORS.grey, ICON_COLORS.pink, ICON_COLORS.red];
      circle(ctx, cx, cy, r * 0.82, colors[icon.index] ?? ICON_COLORS.grey);
      break;
    }
    case "threeSigns": {
      const color = traffic(icon);
      if (t < 0.34) {
        // Red diamond.
        ctx.beginPath();
        ctx.moveTo(cx, cy - r * 0.85);
        ctx.lineTo(cx + r * 0.85, cy);
        ctx.lineTo(cx, cy + r * 0.85);
        ctx.lineTo(cx - r * 0.85, cy);
        ctx.closePath();
        ctx.fillStyle = color;
        ctx.fill();
      } else if (t < 0.67) triangle(ctx, cx, cy, size * 0.88, true, color);
      else circle(ctx, cx, cy, r * 0.8, color);
      break;
    }
    case "threeSymbols":
      circle(ctx, cx, cy, r * 0.86, traffic(icon));
      symbol(ctx, cx, cy, size, t < 0.34 ? "x" : t < 0.67 ? "!" : "v", "#ffffff");
      break;
    case "threeSymbolsUncircled":
      symbol(ctx, cx, cy, size * 1.25, t < 0.34 ? "x" : t < 0.67 ? "!" : "v", traffic(icon));
      break;
    case "threeStars":
      star(ctx, cx, cy, r * 0.95, t);
      break;
    case "threeTriangles":
      if (t < 0.34) triangle(ctx, cx, cy, size * 0.8, false, ICON_COLORS.red);
      else if (t < 0.67) {
        ctx.fillStyle = ICON_COLORS.amber;
        ctx.fillRect(x + size * 0.18, cy - size * 0.1, size * 0.64, size * 0.2);
      } else triangle(ctx, cx, cy, size * 0.8, true, ICON_COLORS.green);
      break;
    case "fourRating":
      bars(ctx, x + size * 0.06, y + size * 0.06, size * 0.88, 4, icon.index + 1);
      break;
    case "fiveRating":
      bars(ctx, x + size * 0.06, y + size * 0.06, size * 0.88, 4, icon.index);
      break;
    case "fiveQuarters":
      quarters(ctx, cx, cy, r * 0.8, icon.index);
      break;
    case "fiveBoxes":
      boxes(ctx, x + size * 0.08, y + size * 0.08, size * 0.84, icon.index);
      break;
  }
  ctx.restore();
}

/** `0xRRGGBBAA` → `rgba()` with an alpha override. */
function rgbaWith(rgba: number, alpha: number): string {
  const n = rgba >>> 0;
  return `rgba(${(n >>> 24) & 255},${(n >>> 16) & 255},${(n >>> 8) & 255},${alpha})`;
}

/**
 * A data bar inside the cell box (x, y, w, h): from the left edge, or from
 * the axis when the range has negatives (negative bars grow leftwards).
 */
export function drawDataBar(ctx: CanvasRenderingContext2D, bar: DataBarView, x: number, y: number, w: number, h: number, zoom: number): void {
  const padX = 2 * zoom;
  const padY = Math.max(2, 3 * zoom);
  const innerW = Math.max(0, w - 2 * padX);
  const top = y + padY;
  const height = Math.max(1, h - 2 * padY - 1);
  const fraction = Math.max(0, Math.min(1, bar.fraction));
  const axis = bar.axis === null ? null : Math.max(0, Math.min(1, bar.axis));
  let x0: number;
  let len: number;
  if (axis === null) {
    x0 = x + padX;
    len = innerW * fraction;
  } else if (bar.negative) {
    len = innerW * axis * fraction;
    x0 = x + padX + innerW * axis - len;
  } else {
    x0 = x + padX + innerW * axis;
    len = innerW * (1 - axis) * fraction;
  }
  if (len > 0.5) {
    ctx.save();
    if (bar.gradient) {
      const g = bar.negative ? ctx.createLinearGradient(x0 + len, 0, x0, 0) : ctx.createLinearGradient(x0, 0, x0 + len, 0);
      g.addColorStop(0, rgbaWith(bar.color, 0.85));
      g.addColorStop(1, rgbaWith(bar.color, 0.12));
      ctx.fillStyle = g;
    } else {
      ctx.fillStyle = rgbaWith(bar.color, 0.55);
    }
    ctx.fillRect(x0, top, len, height);
    ctx.strokeStyle = rgbaWith(bar.color, 0.7);
    ctx.lineWidth = 1;
    ctx.strokeRect(x0 + 0.5, top + 0.5, Math.max(0, len - 1), Math.max(0, height - 1));
    ctx.restore();
  }
  if (axis !== null) {
    ctx.save();
    ctx.strokeStyle = "rgba(55,65,81,0.6)";
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 2]);
    const ax = Math.round(x + padX + innerW * axis) + 0.5;
    ctx.beginPath();
    ctx.moveTo(ax, top);
    ctx.lineTo(ax, top + height);
    ctx.stroke();
    ctx.restore();
  }
}
