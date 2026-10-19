/**
 * Display-only prettification of formula text for the formula bar:
 * `*` → ` × ` and `<>` → ` ≠ `, outside string literals. The raw text is
 * always what gets edited and stored; never feed `pretty()` output back to
 * the engine (`≠` is not valid criteria syntax).
 */
const OPS: ReadonlyArray<readonly [string, string]> = [
  ["<>", "≠"],
  ["*", "×"],
];

export function pretty(input: string): string {
  if (!input.startsWith("=")) return input;
  let out = "";
  let i = 0;
  while (i < input.length) {
    const ch = input[i] ?? "";
    if (ch === '"') {
      // String literal; "" is an escaped quote.
      let j = i + 1;
      while (j < input.length) {
        if (input[j] === '"') {
          if (input[j + 1] === '"') j += 2;
          else break;
        } else j++;
      }
      out += input.slice(i, j + 1);
      i = j + 1;
      continue;
    }
    if (ch === "'") {
      // Quoted sheet name.
      const j = input.indexOf("'", i + 1);
      const end = j < 0 ? input.length : j + 1;
      out += input.slice(i, end);
      i = end;
      continue;
    }
    const op = OPS.find(([raw]) => input.startsWith(raw, i));
    if (op) {
      out = out.replace(/ +$/, "") + ` ${op[1]} `;
      i += op[0].length;
      while (input[i] === " ") i++;
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}
