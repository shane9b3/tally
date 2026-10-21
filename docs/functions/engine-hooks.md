# Engine hooks: GETPIVOTDATA, MULTIPLE.OPERATIONS, the document locale and reference unions

Some functions need the workbook, not just cell values. Each validates and
normalizes its arguments in `tally-functions` and relies on an `EvalContext`
method with a default (`None`, or en-US), which the engine overrides (plan
W5-2 for pivots, W5-3 for the what-if sandbox; correctness review 3 for the
locale and unions). This page is the contract between the two sides.

## GETPIVOTDATA → `EvalContext::pivot_data(at, field, filters)`

`crates/tally-functions/src/categories/spreadsheet/pivot.rs`

Accepted forms (AOO `ScGetPivotData`, `sc/source/core/tool/interpr2.cxx`):

| Form | Call |
|---|---|
| AOO: `GETPIVOTDATA(data_field; pivot_table [; field; item]…)` | `pivot_data(top_left(pivot_table), data_field, [(field, item), …])` |
| Excel 2000: `GETPIVOTDATA(pivot_table; "constraints")` | `pivot_data_excel(top_left(pivot_table), <bare words of constraints>, [<bracketed pairs>])` |

- `at` is the **top-left cell of the reference**. AOO looks for the
  DataPilot table whose output contains the reference; the engine finds
  the pivot whose output range contains `at`.
- AOO form: `field` must name a data field, by its caption (`Sum -
  Sales`) or its source name (`Sales`), case-insensitively (AOO
  `lcl_IsNamedDataField`, `dpoutput.cxx:1370`); an item name there is
  `#REF!`.
- Excel form (`pivot_data_excel`, which defaults to `pivot_data`):
  `words` is every word outside `Field[Item]` groups, joined by single
  spaces. As AOO `ScDPObject::ParseFilters` (`dpobject.cxx:1320-1530`),
  from left to right the longest word run that names a data field is
  taken, else the longest run naming an item of exactly one field (it
  becomes a filter; the same name in several fields is `#REF!`); a
  single data field may be omitted (`GETPIVOTDATA(K1;"East")`).
- `filters` are `(field name, item text)` pairs, matched
  case-insensitively against the field's display or source name and the
  item's caption. Numeric items arrive in General format (`2026`),
  booleans as `TRUE`/`FALSE`. A caption shared by two shown items
  (`1.2` and `1.4` formatted `0`) is ambiguous: `#REF!`.
- A page field may be filtered only by its selected item; at `- all -`
  any filter on it is `#REF!` (`lcl_CheckPageField`,
  `dpoutput.cxx:1388-1410`).
- The value is the **displayed** result ("show as" applied,
  `dpoutput.cxx:1820-1828`); an empty result is 0; an error result is
  `#REF!` (`:1820`).
- Return `Some(value)` for a hit, `None` for "no pivot here / unknown
  field or item / no such cell / error result", which the function
  reports as `#REF!`.
- Before the call the function already answered: odd argument counts
  (`#REF!`), a non-reference table argument (`#REF!`), unbalanced
  brackets in the constraint string (`#REF!`), argument errors (propagated),
  and arity (`Err:511` below 2, `Err:504` above 30).
- The engine answers from an index built with the output (item caption
  hash → item, item path → output line), so a call costs O(fields).

Mock: `MockEvalContext::set_pivot_data(|at, field, filters| …)`; every
request (both forms, through the default `pivot_data_excel`) is also
recorded in `pivot_calls`.

## MULTIPLE.OPERATIONS → `EvalContext::table_op(formula, subs)`

`crates/tally-functions/src/categories/spreadsheet/tableop.rs`

AOO `ScTableOp` (`sc/source/core/tool/interpr4.cxx`): 3 arguments (one
input cell) or 5 (two input cells); 4 is `Err:504`.

```
MULTIPLE.OPERATIONS(formula; column_cell; column_replacement [; row_cell; row_replacement])
→ table_op(formula_cell, [(column_cell, value), (row_cell, value)])
```

- `formula` and each input cell must be **single-cell references**
  (AOO `PopSingleRef`; anything else is `Err:504`).
- A replacement is the **value** of a single-cell reference (what the
  Data ▸ Multiple Operations dialog writes) or, more leniently than AOO, a
  constant or expression. Errors propagate.
- The engine re-evaluates `formula` with the substitutions applied to the
  precedent chain (generalized goal-seek sandbox, capped at 10,000 calls
  per recalc) and returns the formula cell's value; `None` means no
  sandbox or `formula` is not a formula cell, reported as `Err:504`.

Mock: `MockEvalContext::set_table_op(|formula, subs| …)`; requests are
recorded in `table_op_calls`.

## Document locale → `EvalContext::locale_tag()` (review 3, H2/M6/M7)

`crates/tally-functions/src/context.rs`, `coerce.rs` (`locale_of`,
`text_to_number_in`, `recognize_number`), `criteria.rs`
(`Criterion::parse_in`), `crates/tally-numfmt/src/parse_format.rs`
(`parse_format_with_locale`).

AOO reads criteria operands, `VALUE`, `DATEVALUE`, `TIMEVALUE` and the
format codes of `TEXT` with the document's number formatter
(`pFormatter->IsNumberFormat`, `GetPreviewStringGuess` with the cell
language), and `FIXED`/`DOLLAR` generate their code from the locale's
standard number/currency format. The function side therefore asks the
context for the locale:

```rust
fn locale_tag(&self) -> &str { "en-US" }   // default on the trait
```

**Engine contract.** Return the BCP 47 tag of the workbook locale
(`self.wb.locale.tag`, the same `Locale` that `recognize` uses for typed
input). The tag is resolved with `tally_numfmt::locale_by_tag`; a tag it
does not know falls back to en-US inside the functions, so the engine must
only ever hand out tags of built-in locales (it does: `Workbook::locale`
is a `&'static Locale`).

**`locale_by_tag` is strict now (M7).** An unknown region (`de-CH`,
`es-MX`, `pt-BR`) is `None`; a language-only tag (`de`, `fr`) maps to the
language's default region (`de-DE`, `fr-FR`); `_` and case are tolerated.
`Workbook::set_locale` already returns `false` on `None`; the ODS reader's
`default_locale` (`fo:language`/`fo:country`) and `Workbook::from_data_opts`
should surface the fallback as a `WorkbookInfo` warning ("locale de-CH not
supported, using en-US") instead of silently keeping en-US, and the
service's `set_locale` should keep rejecting the tag.

What changes on the function side under a non-en-US locale (all tested
with `MockEvalContext::set_locale`):

| Function | en-US | de-DE |
|---|---|---|
| `COUNTIF(A1:A3;">1,5")` | text operand | number 1.5 (`">1.5"` is 1 May) |
| `SUMIF(A4:A5;"<19.10.2026")` | text | date |
| criteria `WAHR`/`FALSCH` | text | booleans (`TRUE`/`FALSE` always work) |
| `VALUE("1,7")`, `VALUE("1.234,5")` | `#VALUE!` | 1.7, 1234.5 |
| `DATEVALUE("19.10.2026")` | `#VALUE!` | 46314 |
| `TEXT(1.7;"0,00")` | `002` (`,` groups) | `1,70` |
| `TEXT(d;"TT.MM.JJJJ")` | literal letters | `19.10.2026` (English keywords also accepted) |
| `TEXT(1.7;"0.00")` | `1.70` | `002` (`.` groups, `,` is the point, as AOO) |
| `FIXED(1234.567)` | `1,234.57` | `1.234,57` |
| `DOLLAR(1234.567)` | `$1,234.57` | `1.234,57 €` |
| `NUMBERVALUE("1.234,5")` | `#VALUE!` | 1234.5 (defaults are the locale's separators) |
| `ConvertLocale` text-to-number | en-US recognizer | locale recognizer |

The conformance runner (`crates/tally-engine/tests/conformance.rs`) has no
`locale` setting yet; adding `settings = { locale = "de-DE" }` there (and
to the allowed settings in
`crates/tally-functions/tests/conformance_files.rs`) would let the table
above be pinned as vectors. Until then the cases live in the unit tests of
`value_fn.rs`, `datevalue.rs`, `time_fn.rs`, `text_fn.rs`, `fixed.rs`,
`numbervalue.rs`, `conditional.rs` and `tests/{coercion,criteria}.rs`.

## Reference unions → `EvalValue::Union` (review 3, M1)

`crates/tally-functions/src/context.rs` (`EvalValue::{union, areas,
is_reference}`), `args.rs` (`Args::ranges`), `coerce.rs`, `eval.rs`.

AOO keeps `A1:A2~B1:B2` as one `svRefList` operand and lets each function
decide: `ScSum`/`IterateParameters`, `ScCountIf`, `ScCountEmptyCells`,
`IterateParametersIf` (no sum range), `ScAreas`, `ScIndex` (the `range`
argument picks the area), `ScIsRef` and the statistics iterators accept it;
`ScRows`/`ScColumns` answer `Err:504`; `IterateParametersIf` with a sum
range answers `Err:504`; everything that needs one rectangle gives
`#VALUE!`. The engine today instead splits a union into separate
arguments, but only for `NumberSeq`/`ValueSeq` parameters
(`crates/tally-engine/src/eval.rs`, `flatten_unions`), so SUBTOTAL, AREAS,
COUNTIF, INDEX and ROWS saw `#VALUE!`.

The function side is implemented against a new variant:

```rust
pub enum EvalValue {
    …,
    #[cfg(feature = "ref-union")]
    Union(Arc<[CellRange]>),   // two or more areas, formula order
}
EvalValue::union(Vec<CellRange>)  // 0 → #REF!, 1 → Range, n → Union
v.areas() -> Option<&[CellRange]> // one area for Range, all for Union
v.is_reference()
Args::ranges(i) -> Result<Vec<CellRange>, ErrorKind>
```

Semantics on the function side (all tested with the mock, with and
without the feature; `MockEvalContext::union(sheet, &["A1:A2", "B1:B2"])`
builds one):

- sequence helpers (`number_seq`, `values_seq`, `SparseData::read`) visit
  every area in order, first error wins: SUM, COUNT, MAX, MEDIAN, … work;
- scalar parameters: `#VALUE!` (no implicit intersection possible), also in
  array context (a union cannot be materialized);
- `SUBTOTAL(f; r1; r2; …)` is variadic (M2) and takes unions;
- `AREAS` counts the members; `ROWS`/`COLUMNS` give `Err:504`;
- `COUNTIF`, `COUNTBLANK`, `SUMIF`/`AVERAGEIF` without a sum range iterate
  the areas; with a sum range `Err:504`; the `*IFS` functions give `#VALUE!`;
- `INDEX(union; row; col; area)`: `area` selects the member, past the last
  member `#REF!`;
- `ISREF` is `TRUE`, `TYPE` is 64; `OFFSET`, lookups, matrix functions and
  SUMPRODUCT give `#VALUE!`.

**Why a cargo feature.** The engine's `operand()` in `eval.rs` matches
`EvalValue` exhaustively, so adding the variant unconditionally would break
the engine build mid-wave. The variant exists only with the `ref-union`
feature of `tally-functions`; the whole function crate compiles and is
tested both ways (`cargo test -p tally-functions --features ref-union`
runs the union tests). CI's `cargo test --workspace` stays on the
no-feature build until the engine switches.

**Engine contract (the switch).**

1. In `crates/tally-engine/Cargo.toml`: `tally-functions = { workspace =
   true, features = ["ref-union"] }`; then make the feature the default
   (or drop the `cfg`s) in `tally-functions`.
2. `eval.rs`: give `operand()` an arm for `EvalValue::Union` (treat it like
   a non-intersectable range: `Operand::Scalar(scalar_value(&v, ctx))`,
   which yields `#VALUE!`), and check `cell_result`/`array_result` for
   the same (`#VALUE!`).
3. `BinOp::Union` in `binary()` builds `EvalValue::union(left areas ++
   right areas)` when both operands are references (nested unions
   concatenate; `Range ~ Range` → `Union`); any other operand is `#VALUE!`
   as today. The result also feeds `cell_result` → `#VALUE!` when a union
   is a cell's final value, as in AOO (`=A1:A2~B1:B2` alone is `#VALUE!`).
4. Remove `flatten_unions`/`union_leaves` (the function side flattens
   sequence parameters itself) and the `is_union` name resolution; `Range`
   named expressions that are unions then evaluate to `Union` naturally.
5. Dependencies: record every area of a union in the formula's precedents
   (the current splitting already does so per leaf).
6. Conformance vectors to add once (1)–(4) are in (the runner cannot
   express them before): `=SUBTOTAL(9;A1:A2~B1:B2)` = 33, `=AREAS(A1:A2~B1:B2)`
   = 2, `=COUNTIF(A1:A2~B1:B2;">1")` = 3, `=INDEX(A1:A2~B1:B2;2;1;2)` = 20,
   `=ROWS(A1:A2~B1:B2)` = `Err:504`, `=SUM(A1:A2~B1:B2)` = 33 (unchanged),
   `=ISREF(A1~B1)` = TRUE, `=COUNTBLANK(A1:A2~C1:C2)`.

## Conformance

`tests/conformance/spreadsheet/{GETPIVOTDATA,MULTIPLE.OPERATIONS}.toml`
pin the argument handling only (every case is an error or an
`IFERROR` of one), because the engine runner has no pivots and no
sandbox. Value cases belong to the engine's DataPilot tests and
`multiple_operations_one_and_two_variable`.

`tests/conformance/mathematical/SUBTOTAL.toml` pins the variadic form
(`SUBTOTAL(9;A1:A2;A3)`), `tests/conformance/spreadsheet/INDIRECT.toml`
and `ERRORTYPE.toml` pin `Err:502` for unparsable INDIRECT text (AOO
`ScIndirect`: `PushIllegalArgument`, review 3 L).
