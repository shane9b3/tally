// Scans packages/app/src for user-visible English literals that bypass t():
// JSX text, string/template values of aria-label, title, placeholder, label,
// alt, aria-description, aria-placeholder, aria-roledescription, and
// setAttribute("aria-label" | "title" | "placeholder", "...").
// Also: pushToast messages and action labels, reason/message/hint/detail/tooltip props, applyOps/apply/applyPatch
// undo labels, and user-facing `throw new Error(...)` messages.
// Usage: node scripts/scan-literals.mjs [--summary | --json]
import ts from "typescript";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SRC = fileURLToPath(new URL("../src", import.meta.url));
const ATTRS = new Set(["aria-label", "title", "placeholder", "label", "alt", "aria-description", "aria-placeholder", "aria-roledescription", "aria-valuetext"]);
const files = [];
(function walk(d) {
  for (const n of readdirSync(d)) {
    const p = join(d, n);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.tsx$/.test(n) || /\.ts$/.test(n)) files.push(p);
  }
})(SRC);

// Text that is deliberately not translated: key caps, glyphs, brand, formula/range examples, sample data.
const ALLOW = new Set([
  "Tally", "esc", "Tab", "Esc", "fx", "Aa", "AaBbCc", "mm)", "SUMIF, VLOOKUP…", "$G2=\"Overdue\"", "Paid Due Overdue",
  "$Lists.$A$1:$A$10", "AND(D2>0,D2<100)", "${} Esc", "01/01/1900 (StarCalc 1.0)", "; (OpenOffice Calc A1)", ", (Excel A1)",
]);
// Object-literal props that carry user-visible prose. `label`/`title` are not here: those are data-table
// labels (English fallback beside a key, translated where used) or command titles (localizeTitles).
const PROPS = new Set(["reason", "message", "hint", "detail", "tooltip"]);
// Thrown messages for programmer errors (never shown to users).
const INTERNAL_THROW = /outside <|^unreachable|^invariant/i;
// Prose = starts with a capital letter and has a space or sentence punctuation, or is a single capitalised word.
const looksLikeProse = (s) => /^[A-Z“"(]/.test(s) && /[a-z]/.test(s);
const calleeName = (c) => (ts.isPropertyAccessExpression(c.expression) ? c.expression.name.text : ts.isIdentifier(c.expression) ? c.expression.text : "");
// Every string/template literal that can become the value of e (descends through ?: and ??/||/&& branches and parentheses).
function literalsOf(e) {
  const out = [];
  (function go(x) {
    if (!x) return;
    if (ts.isParenthesizedExpression(x)) return go(x.expression);
    if (ts.isConditionalExpression(x)) { go(x.whenTrue); go(x.whenFalse); return; }
    if (ts.isBinaryExpression(x) && [ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.AmpersandAmpersandToken].includes(x.operatorToken.kind)) { go(x.left); go(x.right); return; }
    if (ts.isStringLiteral(x) || ts.isNoSubstitutionTemplateLiteral(x)) out.push([x.text, x]);
    else if (ts.isTemplateExpression(x)) out.push([x.head.text + x.templateSpans.map((sp) => "${}" + sp.literal.text).join(""), x]);
  })(e);
  return out;
}
const hasWords = (s) => /[A-Za-z]{2,}/.test(s);
const hits = [];
for (const file of files) {
  const text = readFileSync(file, "utf8");
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const add = (node, kind, s) => {
    if (ALLOW.has(s.replace(/\s+/g, " ").trim())) return;
    const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
    hits.push({ file: relative(SRC, file), line: line + 1, kind, text: s.replace(/\s+/g, " ").trim(), start: node.getStart(), end: node.getEnd(), template: ts.isTemplateExpression(node) });
  };
  const literalOf = (e) => {
    if (!e) return null;
    if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e)) return e.text;
    if (ts.isTemplateExpression(e)) return e.head.text + e.templateSpans.map((s) => "${}" + s.literal.text).join("");
    return null;
  };
  const visit = (n) => {
    if (ts.isJsxText(n)) {
      const s = n.text.replace(/\s+/g, " ").trim();
      if (s && hasWords(s) && !ALLOW.has(s)) add(n, "jsx-text", s);
    } else if (ts.isJsxAttribute(n) && ATTRS.has(n.name.getText())) {
      const init = n.initializer;
      let s = null;
      if (init && ts.isStringLiteral(init)) s = init.text;
      else if (init && ts.isJsxExpression(init)) s = literalOf(init.expression);
      if (s && hasWords(s)) add(n, "attr:" + n.name.getText(), s);
    } else if (ts.isJsxExpression(n) && n.parent && (ts.isJsxElement(n.parent) || ts.isJsxFragment(n.parent))) {
      const s = literalOf(n.expression);
      if (s && hasWords(s)) add(n, "jsx-expr", s);
    } else if (ts.isCallExpression(n) && calleeName(n) === "pushToast") {
      // pushToast(kind, message, actions?): message may be a literal, template or conditional of them.
      for (const [s, node] of literalsOf(n.arguments[1])) if (hasWords(s)) add(node, "toast", s);
      // action buttons: pushToast(kind, msg, [{ label: "Undo", ... }])
      (function actions(x) {
        if (!x) return;
        if (ts.isPropertyAssignment(x) && x.name.getText() === "label") for (const [s, node] of literalsOf(x.initializer)) if (hasWords(s)) add(node, "toast-action", s);
        ts.forEachChild(x, actions);
      })(n.arguments[2]);
    } else if (ts.isCallExpression(n) && (["apply", "applyPatch", "applyOps", "undoLabel"].includes(calleeName(n)))) {
      for (const [s, node] of literalsOf(n.arguments[0])) if (hasWords(s)) add(node, "undo-label", s);
    } else if (ts.isThrowStatement(n) && n.expression && ts.isNewExpression(n.expression) && n.expression.expression.getText() === "Error") {
      for (const [s, node] of literalsOf(n.expression.arguments?.[0])) if (hasWords(s) && !INTERNAL_THROW.test(s)) add(node, "throw", s);
    } else if (ts.isPropertyAssignment(n) && PROPS.has(n.name.getText())) {
      // Object-literal text props (toast actions, command reasons, hints...); not under t(...) because that is a call.
      for (const [s, node] of literalsOf(n.initializer)) if (hasWords(s) && looksLikeProse(s)) add(node, "prop:" + n.name.getText(), s);
    } else if (ts.isCallExpression(n) && ts.isPropertyAccessExpression(n.expression) && n.expression.name.text === "setAttribute") {
      const [a, b] = n.arguments;
      if (a && ts.isStringLiteral(a) && ATTRS.has(a.text)) {
        const s = literalOf(b);
        if (s && hasWords(s)) add(n, "setAttribute:" + a.text, s);
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
if (process.argv.includes("--json")) {
  console.log(JSON.stringify(hits));
} else if (process.argv.includes("--summary")) {
  const by = new Map();
  for (const h of hits) by.set(h.file, (by.get(h.file) ?? 0) + 1);
  for (const [f, c] of [...by].sort((a, b) => b[1] - a[1])) console.log(String(c).padStart(4), f);
  console.log(String(hits.length).padStart(4), "total");
} else {
  for (const h of hits) console.log(`${h.file}:${h.line} [${h.kind}] ${h.text}`);
  console.log(`${hits.length} literal(s)`);
}
process.exitCode = hits.length ? 1 : 0;
