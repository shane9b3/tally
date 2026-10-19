# Adding a spreadsheet function

Functions live in `crates/tally-functions`. They depend only on `tally-types`
and `tally-numfmt`, never on the engine. The engine
reaches them through `tally_functions::eval_call`.

## Where things go

```
crates/tally-functions/src/
  registry.rs            FunctionDef, Param, ParamKind, FnFlags, Registry, FuncId
  context.rs             EvalContext / ArgList traits (implemented by tally-engine), EvalValue
  eval.rs                eval_call: arity, argument preparation, intersection, array lifting
  coerce.rs              every coercion rule (do not re-implement them)
  criteria.rs            COUNTIF-style criteria
  args.rs                Args helpers, run / run_ref, check_arity
  numeric.rs             approx_add, ApproxSum
  stats_util.rs          THE erf/erfc, normal density/CDF/inverse (AOO ports) and the
                         SparseData column-major argument reader; reuse, never copy
  mock.rs                MockEvalContext, MockArgs, call() (feature `test-support`)
  categories/<category>/mod.rs     one `register` list per category (owned by that category's engineer)
  categories/<category>/<family>.rs  one file per function family
  categories/ooxml.rs    OOXML (Excel) names, one table for every function
tests/conformance/<category>/<FUNC>.toml   end-to-end vectors (repo root; run by tally-engine)
docs/functions/master-list.toml            generated: python3 -I docs/functions/gen-master-list.py
docs/functions/coverage.md                 generated: cargo xtask function-coverage --doc
```

Category directories: `mathematical statistical logical text datetime
spreadsheet information array addin database financial`. Use the AOO
category from `docs/functions/master-list.toml` (for example AVERAGEIF is
Mathematical, RANDBETWEEN is Add-in).

Group related functions in one family file (`mathematical/rounding.rs` holds
ROUND, ROUNDUP, ROUNDDOWN, TRUNC, INT, …). Adding a family means one new
file plus one `family::register(r);` line in the category `mod.rs`; nothing
else is edited centrally.

## Worked example: ABS

`crates/tally-functions/src/categories/mathematical/abs.rs` (about 30 lines with tests):

```rust
//! ABS.

use tally_types::Value;

use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const ABS_PARAMS: &[Param] = &[Param::required("number", ParamKind::Number)];

/// `ABS(number)`.
pub const ABS: FunctionDef = FunctionDef::new("ABS", Category::Mathematical, ABS_PARAMS, abs)
    .help("Absolute value of a number.");

fn abs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.num(0)?.abs())))
}

pub(super) fn register(r: &mut Registry) {
    r.register(ABS);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    #[test]
    fn abs() {
        let mut c = MockEvalContext::new();
        assert_eq!(call(&mut c, "ABS", vec![EvalValue::number(-2.0)]), Value::Number(2.0));
        assert_eq!(call(&mut c, "ABS", vec![EvalValue::text("x")]), Value::Error(ErrorKind::Value));
    }
}
```

Then add `mod abs;` and `abs::register(r);` to `categories/mathematical/mod.rs`,
and write `tests/conformance/mathematical/ABS.toml` (below).

`FunctionDef::new` derives `min_args` (required params) and `max_args`
(`params.len()`, or 255 when any param has `repeat: true`). Override with
`.arity(min, max)` when the AOO signature differs, set `.odf("COM.MICROSOFT.IFNA")`,
`.ooxml("_xlfn.IFNA")`, `.extension()` (not in AOO 4.1), `.flags(..)` and
`.returns(..)` as needed. The registry test checks name, category, ODF name,
arity and `extension` against `docs/functions/master-list.toml`, and
`conformance_files.rs` fails until the TOML vector file exists.

## The author API

```rust
let mut a = Args::new(ctx, args);
run(|| {                              // Err(e) -> Value::Error(e); NaN/inf -> #NUM!
    let x = a.num(0)?;                // number (missing -> 0)
    let d = a.num_opt(1, 0.0)?;       // optional with default
    let k = a.int_opt(2, 1)?;         // integer: snapped to 15 digits, truncated
    let s = a.text(3)?;               // Arc<str> (numbers in General format)
    let flag = a.bool_opt(4, true)?;  // TRUE/FALSE text accepted, other text #VALUE!
    let r = a.range(5)?;              // must be a reference, else #VALUE!
    let c = a.criterion(6)?;          // criteria::Criterion; c.matches(&value)
    let mut sum = 0.0;
    a.for_each_number(7, |v| sum += v)?;   // SUM semantics
    a.for_each_value(8, |v, src| { /* COUNTA-style: every value, no propagation */ });
    let raw = a.any(9);               // EvalValue (Missing / Scalar / Range / Array)
    let n = a.count();                // number of written arguments
    let gap = a.is_missing(10);       // omitted or past the end
    Ok(Value::Number(x + d))
})
```

Functions that may return a reference (IF, INDEX, OFFSET, CHOOSE) use
`run_ref(|| Ok(EvalValue::Range(r)))` and set `FnFlags::RETURNS_REF`. Use
`a.ctx()` for `cell_value`, `for_each_in_range`, `now_serial`, `rand`, etc.
Never `unwrap()`.

### Parameter kinds (what `eval_call` does before your code runs)

| ParamKind | Preparation |
|---|---|
| `Number`, `Integer`, `Text`, `Bool` | evaluated; a range is reduced by implicit intersection (same row for a column, same column for a row, else `#VALUE!`); in array context an array/multi-cell range is lifted elementwise and the function returns a matrix |
| `Any`, `Criteria` | evaluated; implicit intersection; never lifted |
| `NumberSeq`, `ValueSeq`, `Reference`, `Array` | evaluated; references and arrays passed through unchanged (`Array` params are evaluated by the engine in array context) |
| `Lazy` | not evaluated; `a.any(i)` evaluates on demand (requires `FnFlags::LAZY_ARGS`; such functions are never lifted) |

Too few arguments is `Err:511`, too many `Err:504`, checked before any argument is evaluated.

### Coercion rules (all in `coerce.rs`)

- Scalar: empty/missing → 0, `""`, FALSE; bool → 1/0; text → number per
  `settings.text_to_number` (default `ConvertUnambiguous`: plain decimals and
  ISO 8601 dates/times only, `""` is `#VALUE!`; `ConvertLocale` uses
  tally-numfmt's en-US input recognizer); errors propagate; arrays give
  their top-left element. Numbers become text through tally-numfmt's
  General formatter (`coerce::number_to_text`).
- Sequences (`number_seq`): direct arguments are coerced as scalars (so
  `SUM("3")` is 3); inside references and arrays numbers **and booleans**
  count (AOO stores booleans as numbers), text and empties are skipped, the
  first error in column-major order propagates. `number_seq_with_text_as_zero`
  (AVERAGEA) counts text in references as 0; `values_seq` (COUNTA) visits
  everything without propagating errors.
- Text rendering of numbers: 15 significant digits, decimal for
  `1e-4 <= |x| < 1e15`, else `1E+20` style.

### Criteria (`criteria.rs`)

`Criterion::parse(&value, settings)` handles `=x <>x <x <=x >x >=x` and bare
values. Numbers compare with `approx_equal` and also match numeric text for
`=`; ordering operators never match text or blanks. Text uses the document's
criteria mode (Wildcards `* ? ~` by default, Regex, Literal), `whole_cell_match`
and `case_sensitive` (default off). `""` matches blank cells and empty text,
`"="` only blank cells, `"<>"` any non-blank cell.

## Conformance vectors

One file per function: `tests/conformance/<category>/<FUNC>.toml`, at least
8 cases, covering error propagation, empty cells, text in ranges, booleans,
missing optional arguments and (for scalar params) implicit intersection.

```toml
[[case]]
name = "text skipped, bool counted"         # optional, short
cells = { A1 = 1, A2 = "x", A3 = true, A4 = "=1/0" }
formula = "=SUM(A1:A3)"
expect = 2
note = "AOO stores booleans as numbers."    # optional free text

[[case]]
formula = "=ROUND(-2.5;0)"
expect = -3

[[case]]
formula = "=STDEV(1;2;3)"
expect = 1.0
tol = 1e-12
settings = { criteria_mode = "wildcards", case_sensitive = false }
```

| key | meaning |
|---|---|
| `formula` (required) | formula in the en-US UI dialect (CalcA1); `;` is preferred as the argument separator (`,` is also accepted); inline arrays use `{1;2|3;4}` |
| `expect` (required) | number, boolean, string, error string (`"#DIV/0!"`, `"Err:502"`; any string that `ErrorKind::parse` accepts is an error), `{ text = "#N/A" }` for literal text that looks like an error, or an array of rows `[[1, 2], [3, 4]]` for array results |
| `cells` | input cells on the first sheet; keys are A1 addresses, or `"Sheet2.A1"` for other sheets (created on demand). Strings starting with `=` are formulas; `{ text = "=x" }` is literal text. Omitted cells are empty |
| `origin` | the cell holding the formula (default `Z100`, which intersects nothing used here) |
| `tol` | absolute tolerance for numbers; default is `approx_equal`; `tol = 0` means bit-exact |
| `settings` | `criteria_mode` (`wildcards`/`regex`/`literal`), `text_to_number` (`generate_error`/`treat_as_zero`/`convert_unambiguous`/`convert_locale`), `date_epoch` (`"1899"`/`"1900"`/`"1904"`), `case_sensitive`, `whole_cell_match`, `precision_as_shown` (booleans) |
| `aoo_expect`, `lo_expect` | recorded results from AOO 4.1 / LibreOffice when they differ from `expect` (filled in by `xtask oracle`) |
| `note` | why the case exists or why it deviates |

Example for ABS (`tests/conformance/mathematical/ABS.toml`):

```toml
[[case]]
formula = "=ABS(-2)"
expect = 2
[[case]]
formula = "=ABS(\"-3\")"
expect = 3
[[case]]
formula = "=ABS(\"x\")"
expect = "#VALUE!"
[[case]]
formula = "=ABS(A5)"
expect = 0
[[case]]
cells = { A1 = true }
formula = "=ABS(A1)"
expect = 1
[[case]]
cells = { A4 = "=1/0" }
formula = "=ABS(A4)"
expect = "#DIV/0!"
[[case]]
cells = { A1 = -1, A2 = -2 }
origin = "C2"
formula = "=ABS(A1:A2)"
expect = 2
[[case]]
formula = "=ABS()"
expect = "Err:511"
```

`crates/tally-functions/tests/conformance_files.rs` validates every file
against this schema (allowed keys, settings values, at least 8 cases, file in
the right category directory, one file per registered function). The
end-to-end runner is `crates/tally-engine/tests/conformance.rs` (WP1).

## Names: the master list is the source of truth

`docs/functions/master-list.toml` is generated from `docs/calc-functions.json`
by `docs/functions/gen-master-list.py`; never edit it by hand. Lead-approved
departures from the AOO JSON live in explicit tables at the top of the script,
each checked against the JSON so a stale entry fails the generator:

- `ARITY_OVERRIDES` (TRUNC's optional count, NUMBERVALUE's 1..3 arguments);
- `ODF_OVERRIDES` (ZGZ is stored as `RRI`);
- `ADDIN_LOSES_PLAIN_NAME` / `ADDIN_KEEPS_PLAIN_NAME`: AOO's add-in clash rule.
  Plain ISEVEN/ISODD/GCD/LCM/EFFECT/CUMPRINC/CUMIPMT/NOMINAL mean the
  built-in, so those `_ADD` add-ins are stored under their UNO programmatic
  name (`COM.SUN.STAR.SHEET.ADDIN.ANALYSIS.GETGCD`); plain WEEKNUM/CONVERT/
  DURATION mean the add-in, so the built-ins keep `ISOWEEKNUM`,
  `ORG.OPENOFFICE.CONVERT` and `PDURATION`;
- `EXTENSION_OVERRIDES` (NUMBERVALUE: listed but never implemented by AOO);
- `XLFN_2010_PLUS` / `OOXML_RENAMED`: `ooxml_name`s. Mirror any change in
  `crates/tally-functions/src/categories/ooxml.rs`.

Register functions with exactly the master list's ODF name (`.odf(..)`),
arity, category and `.extension()`; `registry_matches_master_list` reports
every mismatch at once, and `addin_name_clashes_follow_aoo` pins the clash
rule. To change a name, change the generator table, regenerate, then the
registration.

## Lessons from the parallel batches

- **Quotes in TOML formulas**: write formulas that contain `"` as TOML
  literal strings, `formula = '=LEFT("abc";2)'`. Literal strings cannot
  contain `'` and have no escapes (`''` is *not* an escaped quote); for a
  formula with both quote kinds use a basic string with `\"`.
- **Per-agent target directories**: when several people or agents build at
  once, give each its own `CARGO_TARGET_DIR` (`.targets/<name>`) and `-j 4`;
  a shared target directory serialises on the build lock and rebuilds
  everything when feature sets differ.
- **Check vector data against its note**: a case documented as
  "y = 1 + 2·x1 + 3·x2" must have data on that plane (LINEST/TREND had
  off-plane rows and the function was blamed).
- **Before changing an expectation**, decide which side is wrong from the
  AOO source; never delete a case to make it pass, and record the reason in
  `note` when an expectation changes.
- **No private copies of shared helpers**: erf, the normal distribution and
  sparse range readers are in `stats_util.rs`, coercion in `coerce.rs`.

## Checklist

```sh
cargo fmt -p tally-functions
cargo clippy -p tally-functions --all-targets -- -D warnings
cargo test -p tally-functions
cargo test -p tally-engine --test conformance --features tally-engine/io
cargo xtask function-coverage --check
```

- Port AOO algorithms only with a comment naming the source file (Apache-2.0);
  never copy LibreOffice, IronCalc or formualizer code.
- Document every public item; no `unsafe`, no `unwrap()`/`expect()` outside tests.
- If the AOO and Excel results differ, follow AOO (unless it is clearly a bug)
  and add a `note` to the vector.
