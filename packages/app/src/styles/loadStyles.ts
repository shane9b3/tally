/**
 * Load Styles helpers: when the dialog may load, and the browser mock's
 * stand-in for the engine's `loadstyles.rs` (`ScDocShell::LoadStylesArgs`):
 * which cell styles to define, parents first, each re-based onto this
 * document's version of its parent.
 */
import type { CellStyleDto, LoadStylesOptions, PageSetupDto, StyleDto } from "@tally/bridge";

/** Something would be loaded: a source, and cell styles or (with Overwrite) the page style. */
export function loadStylesValid(opts: LoadStylesOptions, source: string | null): boolean {
  return source !== null && (opts.cellStyles || (opts.pageStyles && opts.overwrite));
}

const MAX_DEPTH = 64;

function find(list: readonly CellStyleDto[], name: string): CellStyleDto | undefined {
  return list.find((s) => s.name === name);
}

function depth(list: readonly CellStyleDto[], name: string): number {
  let d = 0;
  let cur = find(list, name)?.parent ?? null;
  while (cur !== null && d < MAX_DEPTH) {
    d++;
    if (cur === name) break;
    cur = find(list, cur)?.parent ?? null;
  }
  return d;
}

function loops(list: readonly CellStyleDto[], name: string, parent: string): boolean {
  let cur: string | null = parent;
  for (let i = 0; i < MAX_DEPTH && cur !== null; i++) {
    if (cur === name) return true;
    cur = find(list, cur)?.parent ?? null;
  }
  return cur !== null;
}

/** `cell` moved from `oldBase` onto `newBase`: what it shares with the old base follows the new one. */
export function rebaseStyle(oldBase: StyleDto, cell: StyleDto, newBase: StyleDto): StyleDto {
  const out: Record<string, unknown> = { ...cell };
  const keys = new Set([...Object.keys(cell), ...Object.keys(newBase)]);
  for (const k of keys) {
    if (k === "id" || k === "styleName") continue;
    const c = (cell as Record<string, unknown>)[k];
    const o = (oldBase as Record<string, unknown>)[k];
    if (JSON.stringify(c) === JSON.stringify(o)) out[k] = (newBase as Record<string, unknown>)[k];
  }
  return out as StyleDto;
}

/**
 * The styles to (re)define for Load Styles: `current` and `source` are full
 * tables (built-ins included, as `cellStyles` lists them). A source style
 * is taken when its name is new here, or with `overwrite`; this document's
 * other styles whose parent was replaced follow it. Parents come first.
 */
export function planLoadStyles(current: readonly CellStyleDto[], source: readonly CellStyleDto[], overwrite: boolean): CellStyleDto[] {
  const exists = (n: string) => current.some((s) => s.name === n);
  const picked = source.filter((s) => s.name.trim() === s.name && s.name.length > 0 && (overwrite || !exists(s.name))).sort((a, b) => depth(source, a.name) - depth(source, b.name));
  const work: CellStyleDto[] = current.map((s) => structuredClone(s));
  const put = (s: CellStyleDto) => {
    const i = work.findIndex((x) => x.name === s.name);
    if (i >= 0) work[i] = s;
    else work.push(s);
  };
  const out: CellStyleDto[] = [];
  for (const s of picked) {
    let parent: string | null = null;
    if (s.name !== "Default") {
      const p = s.parent !== null && s.parent !== s.name && (find(source, s.parent) || find(work, s.parent)) ? s.parent : "Default";
      parent = loops(work, s.name, p) ? "Default" : p;
    }
    const theirs = parent === null ? undefined : find(source, parent);
    const ours = parent === null ? undefined : find(work, parent);
    const style = theirs && ours ? rebaseStyle(theirs.style, s.style, ours.style) : structuredClone(s.style);
    const next: CellStyleDto = { name: s.name, parent, builtin: exists(s.name) ? (find(current, s.name)?.builtin ?? false) : false, style };
    put(next);
    out.push(next);
  }
  const taken = new Set(picked.map((s) => s.name));
  const rest = work.filter((s) => !taken.has(s.name)).sort((a, b) => depth(work, a.name) - depth(work, b.name));
  for (const child of rest) {
    if (child.parent === null) continue;
    const was = find(current, child.parent);
    const now = find(work, child.parent);
    if (!was || !now || JSON.stringify(was.style) === JSON.stringify(now.style)) continue;
    const next = { ...child, style: rebaseStyle(was.style, child.style, now.style) };
    put(next);
    out.push(next);
  }
  return out;
}

/** The source page style on a sheet whose setup is `cur`: print ranges, repeats and breaks stay. */
export function pageStyleOn(src: PageSetupDto, cur: PageSetupDto): PageSetupDto {
  return { ...structuredClone(src), printRange: cur.printRange, repeatRows: cur.repeatRows, repeatCols: cur.repeatCols, rowBreaks: cur.rowBreaks, colBreaks: cur.colBreaks };
}
