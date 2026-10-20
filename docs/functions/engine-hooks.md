# Engine hooks: GETPIVOTDATA and MULTIPLE.OPERATIONS

Two spreadsheet functions need the workbook, not just cell values. Each
validates and normalizes its arguments in `tally-functions` and makes one
call on an `EvalContext` method whose default returns `None`; the engine
overrides it (plan W5-2 for pivots, W5-3 for the what-if sandbox). This
page is the contract between the two sides.

## GETPIVOTDATA → `EvalContext::pivot_data(at, field, filters)`

`crates/tally-functions/src/categories/spreadsheet/pivot.rs`

Accepted forms (AOO `ScGetPivotData`, `sc/source/core/tool/interpr2.cxx`):

| Form | Call |
|---|---|
| AOO: `GETPIVOTDATA(data_field; pivot_table [; field; item]…)` | `pivot_data(top_left(pivot_table), data_field, [(field, item), …])` |
| Excel 2000: `GETPIVOTDATA(pivot_table; "constraints")` | `pivot_data(top_left(pivot_table), <bare words of constraints>, [<bracketed pairs>])` |

- `at` is the **top-left cell of the reference**. AOO looks for the
  DataPilot table whose output contains the reference; the engine should
  find the pivot whose output range contains `at`.
- `field` is the data field text as typed (`"Sales"`, `"Sum - Sales"`).
  In the Excel form it is every word outside `Field[Item]` groups, joined
  by single spaces: AOO (`ScDPObject::ParseFilters`) matches that text
  greedily against the data field names and, for any remaining words,
  the item names of all fields; a single data field may be omitted. The
  engine resolves `field` the same way, which also handles the AOO form
  (an exact data field name matches first).
- `filters` are `(field name, item text)` pairs. Numeric items arrive in
  General format (`2026`), booleans as `TRUE`/`FALSE`; the engine compares
  against the item's display text.
- Return `Some(value)` for a hit, `Some(Value::Error(..))` to surface a
  specific error, `None` for "no pivot here / unknown field or item",
  which the function reports as `#REF!`.
- Before the call the function already answered: odd argument counts
  (`#REF!`), a non-reference table argument (`#REF!`), unbalanced
  brackets in the constraint string (`#REF!`), argument errors (propagated),
  and arity (`Err:511` below 2, `Err:504` above 30).

Mock: `MockEvalContext::set_pivot_data(|at, field, filters| …)`; every
request is also recorded in `pivot_calls`.

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

## Conformance

`tests/conformance/spreadsheet/{GETPIVOTDATA,MULTIPLE.OPERATIONS}.toml`
pin the argument handling only (every case is an error or an
`IFERROR` of one), because the engine runner has no pivots and no
sandbox. Value cases belong to the engine's DataPilot tests and
`multiple_operations_one_and_two_variable`.
