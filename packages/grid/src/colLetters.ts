/** Column index ↔ letters ("A" = 0, "XFD" = 16383). Pure, no DOM. */

/** 0-based column index to letters (0 → "A", 26 → "AA"). */
export function colToLetters(col: number): string {
  let n = Math.floor(col) + 1;
  let s = "";
  while (n > 0) {
    const m = (n - 1) % 26;
    s = String.fromCharCode(65 + m) + s;
    n = Math.floor((n - 1) / 26);
  }
  return s;
}

/** Letters to a 0-based column index (case-insensitive); -1 if invalid. */
export function lettersToCol(letters: string): number {
  if (!/^[A-Za-z]+$/.test(letters)) return -1;
  let n = 0;
  for (const ch of letters.toUpperCase()) n = n * 26 + (ch.charCodeAt(0) - 64);
  return n - 1;
}

/** "B3" style address of a cell. */
export function cellAddress(r: number, c: number): string {
  return `${colToLetters(c)}${r + 1}`;
}
