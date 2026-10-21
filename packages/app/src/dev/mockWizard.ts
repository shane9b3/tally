/**
 * Function Wizard emulation for the browser mock (e2e): a realistic
 * `list_functions` subset with the engine's `params` / `example`
 * metadata, and `preview_formula` for SUM, AVERAGE, MIN, MAX, COUNT,
 * SUMIF, COUNTIF, SUMIFS and IF over the mock's cell values.
 */
import type { FunctionInfo, RangeDto } from "@tally/bridge";
import { parseCall } from "../wizard/wizardArgs";

type P = FunctionInfo["params"][number];
const req = (name: string, kind = "number"): P => ({ name, kind, optional: false, repeat: false });
const opt = (name: string, kind = "number"): P => ({ name, kind, optional: true, repeat: false });
const rep = (name: string, kind = "numberSeq"): P => ({ name, kind, optional: false, repeat: true });

const fn = (name: string, category: string, params: P[], description: string, example: string, signature?: string): FunctionInfo => {
  const parts = params.map((p) => (p.optional ? `[${p.name}]` : p.name));
  if (params.some((p) => p.repeat)) parts.push("...");
  return { name, category, signature: signature ?? `${name}(${parts.join(", ")})`, description, params, example };
};

/** The functions the browser mock lists (sorted by name, like the engine). */
export const WIZARD_FUNCTIONS: FunctionInfo[] = [
  fn("AVERAGE", "Statistical", [rep("number")], "Returns the average of the arguments.", "=AVERAGE(A1:A10)"),
  fn("COUNT", "Statistical", [rep("value", "valueSeq")], "Counts how many numbers are in the list of arguments.", "=COUNT(A1:A10)"),
  fn("COUNTIF", "Mathematical", [req("range", "reference"), req("criterion", "value")], "Counts the cells in a range that meet a condition.", '=COUNTIF(A1:A10;">5")'),
  fn("IF", "Logical", [req("test", "logical"), opt("then_value", "value"), opt("otherwise_value", "value")], "Specifies a logical test to be performed.", '=IF(A1>5;"big";"small")'),
  fn("MAX", "Statistical", [rep("number")], "Returns the maximum value in a list of arguments.", "=MAX(A1:A10)"),
  fn("MIN", "Statistical", [rep("number")], "Returns the minimum value in a list of arguments.", "=MIN(A1:A10)"),
  // The formula-bar signature tests (v02.spec) read SUM's numbered form.
  fn("SUM", "Mathematical", [rep("number")], "Returns the sum of all arguments.", "=SUM(A1:A10)", "SUM(number1, [number2], …)"),
  fn("SUMIF", "Mathematical", [req("range", "reference"), req("criterion", "value"), opt("sum_range", "reference")], "Totals the arguments that meet the condition.", '=SUMIF(A1:A10;">5";B1:B10)'),
  fn(
    "SUMIFS",
    "Mathematical",
    [req("sum_range", "reference"), rep("range", "reference"), rep("criterion", "value")],
    "Totals the values of cells in a range that meet multiple criteria in multiple ranges.",
    '=SUMIFS(C1:C10;A1:A10;">5";B1:B10;"<10")',
  ),
  fn("VLOOKUP", "Spreadsheet", [req("search_criterion", "value"), req("array", "reference"), req("index"), opt("sorted", "logical")], "Vertical search and reference to indicated cells.", "=VLOOKUP(A1;B1:C10;2;0)"),
];

/** A cell's value: number, text or empty. */
export type MockValue = number | string | null;

/** Reads the values of a range (row-major). */
export type RangeReader = (range: RangeDto) => Promise<MockValue[][]>;

function col(letters: string): number {
  let c = 0;
  for (const ch of letters.toUpperCase()) c = c * 26 + (ch.charCodeAt(0) - 64);
  return c - 1;
}

function parseRef(text: string): RangeDto | null {
  const m = /^\$?([A-Za-z]{1,3})\$?(\d+)(?::\$?([A-Za-z]{1,3})\$?(\d+))?$/.exec(text.trim());
  if (!m?.[1] || !m[2]) return null;
  const c0 = col(m[1]);
  const r0 = Number(m[2]) - 1;
  const c1 = m[3] ? col(m[3]) : c0;
  const r1 = m[4] ? Number(m[4]) - 1 : r0;
  return { r0: Math.min(r0, r1), c0: Math.min(c0, c1), r1: Math.max(r0, r1), c1: Math.max(c0, c1) };
}

function literal(text: string): MockValue {
  const t = text.trim();
  if (/^".*"$/s.test(t)) return t.slice(1, -1).replace(/""/g, '"');
  if (t !== "" && Number.isFinite(Number(t))) return Number(t);
  return t === "" ? null : t;
}

/** AOO criterion matching (`>3`, `<>x`, `=5`, plain value; text case-insensitive). */
function matches(v: MockValue, crit: MockValue): boolean {
  const c = crit === null ? "" : String(crit);
  const m = /^(<=|>=|<>|=|<|>)?(.*)$/s.exec(c);
  const op = m?.[1] ?? "=";
  const rhs = m?.[2] ?? "";
  const n = Number(rhs);
  if (rhs.trim() !== "" && Number.isFinite(n)) {
    if (typeof v !== "number") return op === "<>";
    switch (op) {
      case "<":
        return v < n;
      case ">":
        return v > n;
      case "<=":
        return v <= n;
      case ">=":
        return v >= n;
      case "<>":
        return v !== n;
      default:
        return v === n;
    }
  }
  const eq = String(v ?? "").toLowerCase() === rhs.toLowerCase();
  return op === "<>" ? !eq : eq;
}

const flat = (rows: MockValue[][]): MockValue[] => rows.flat();

function display(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 1e10) / 1e10);
}

/** Evaluates a wizard call (`=SUMIF(A1:A5;">3")`); `null` when it is not one the mock knows. */
export async function previewCall(text: string, read: RangeReader): Promise<string | null> {
  const call = parseCall(text);
  if (!call) return null;
  const values = async (arg: string): Promise<MockValue[]> => {
    const r = parseRef(arg);
    if (r) return flat(await read(r));
    return [literal(arg)];
  };
  const nums = async (args: string[]): Promise<number[]> => (await Promise.all(args.map(values))).flat().filter((v): v is number => typeof v === "number");
  const args = call.args.filter((a, i) => a !== "" || i < call.args.length - 1);
  switch (call.name) {
    case "SUM":
      return display((await nums(args)).reduce((a, b) => a + b, 0));
    case "AVERAGE": {
      const ns = await nums(args);
      return ns.length === 0 ? "#DIV/0!" : display(ns.reduce((a, b) => a + b, 0) / ns.length);
    }
    case "MIN": {
      const ns = await nums(args);
      return display(ns.length === 0 ? 0 : Math.min(...ns));
    }
    case "MAX": {
      const ns = await nums(args);
      return display(ns.length === 0 ? 0 : Math.max(...ns));
    }
    case "COUNT":
      return display((await nums(args)).length);
    case "SUMIF":
    case "COUNTIF": {
      if (args.length < 2) return null;
      const range = await values(args[0] ?? "");
      const crit = literal(args[1] ?? "");
      const sums = args[2] ? await values(args[2]) : range;
      let total = 0;
      let count = 0;
      range.forEach((v, i) => {
        if (!matches(v, crit)) return;
        count++;
        const s = sums[i];
        if (typeof s === "number") total += s;
      });
      return display(call.name === "SUMIF" ? total : count);
    }
    case "SUMIFS": {
      if (args.length < 3) return null;
      const sums = await values(args[0] ?? "");
      const ok = sums.map(() => true);
      for (let k = 1; k + 1 < args.length; k += 2) {
        const range = await values(args[k] ?? "");
        const crit = literal(args[k + 1] ?? "");
        range.forEach((v, i) => {
          if (!matches(v, crit)) ok[i] = false;
        });
      }
      return display(sums.reduce<number>((a, s, i) => (ok[i] && typeof s === "number" ? a + s : a), 0));
    }
    case "IF": {
      const [test, then, otherwise] = args;
      const m = /^(.+?)(<=|>=|<>|=|<|>)(.+)$/.exec(test ?? "");
      let truth: boolean;
      if (m?.[1] && m[2] && m[3]) {
        const lhs = (await values(m[1]))[0] ?? null;
        truth = matches(lhs, `${m[2]}${String(literal(m[3]) ?? "")}`);
      } else {
        const v = (await values(test ?? ""))[0] ?? null;
        truth = typeof v === "number" ? v !== 0 : v !== null && v !== "";
      }
      const pick = truth ? then : otherwise;
      if (pick === undefined) return truth ? "TRUE" : "FALSE";
      const v = (await values(pick))[0] ?? null;
      return v === null ? "0" : typeof v === "number" ? display(v) : v;
    }
    default:
      return null;
  }
}
