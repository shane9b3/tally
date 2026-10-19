# Engine API: Tauri commands ↔ `DocumentService`

The UI talks to the engine only through `EngineClient` (`packages/bridge/src/client.ts`).
In the desktop app, `TauriClient` (WP7) implements it by invoking the Tauri commands
below; each command is a thin async wrapper over one method of the Rust
`DocumentService` trait (`crates/tally-api/src/service.rs`). `MockClient`
(`packages/bridge/src/mock.ts`) implements the same interface in memory.

Types come from `crates/tally-api/src/dto.rs` and are generated into
`packages/bridge/src/generated/` by ts-rs. Regenerate with `cargo xtask gen-bindings`
(CI runs `cargo xtask gen-bindings --check`).

## Conventions

- Command names are `snake_case`; their JS arguments are `camelCase` (Tauri's default).
- Errors reject the promise with `ApiError` `{ code: string, message: string }`.
  Codes: `not_found`, `invalid_argument`, `io`, `bad_file`, `unsupported`, `internal`,
  and (shell only) `cancelled` when the user dismisses a native dialog (`ApiError::CANCELLED`, `CANCELLED` in TS; the UI ignores it).
- `DocId` and `SheetId` are numbers. A `SheetId` is stable and is not the tab index.
- Optional values are `T | null` in both directions.
- Enums with data are discriminated unions on `type` (`{ type: "setCellInput", ... }`);
  plain enums are camelCase strings (`"ods"`, `"down"`).

## Commands

| Tauri command | Arguments | Returns | `DocumentService` method | `EngineClient` method |
|---|---|---|---|---|
| `wb_new` | `template?: string` (id: `invoices`, `budget`, `stock`) | `WorkbookInfo` | `new_workbook(template)` (id passed through; the engine builds it) | `newWorkbook(template?)` |
| `wb_open_dialog` | none | `WorkbookInfo \| null` | shell only: native dialog, then `open(path)` | `ShellClient.openDialog()` |
| `wb_open` | `path: string` | `WorkbookInfo` | `open(path)` | `open(path)` |
| `wb_save` | `doc, saveAs: boolean, format?: FileFormat` | `WorkbookInfo` | shell shows a dialog when `saveAs` or no path, then `save(doc, path, format)` | `save(doc, path \| null, format)`; `ShellClient.saveWithDialog(doc, saveAs, format)` |
| `wb_close` | `doc` | `void` | `close(doc)` | `close(doc)` |
| `sheet_layout` | `doc, sheet` | `SheetLayout` | `sheet_layout(doc, sheet)` | `sheetLayout(doc, sheet)` |
| `get_cells` | `doc, sheet, ranges: RangeDto[]` | `CellBlock[]` | `cells(doc, sheet, ranges)` | `cells(doc, sheet, ranges)` |
| `get_styles` | `doc, ids: number[]` | `StyleDto[]` | `styles(doc, ids)` | `styles(doc, ids)` |
| `cell_edit_info` | `doc, sheet, r, c` | `CellEditInfo` | `cell_edit_info(doc, sheet, r, c)` | `cellEditInfo(doc, sheet, r, c)` |
| `apply` | `doc, tx: Transaction` | `ApplyResult` | `apply(doc, tx)` | `apply(doc, tx)` |
| `undo` | `doc` | `ApplyResult` | `undo(doc)` | `undo(doc)` |
| `redo` | `doc` | `ApplyResult` | `redo(doc)` | `redo(doc)` |
| `selection_stats` | `doc, sel: SelectionDto` | `SelectionStats` | `selection_stats(doc, sel)` | `selectionStats(doc, sel)` |
| `clipboard_copy` | `doc, sel: SelectionDto` | `ClipboardOut` | `copy(doc, sel)` | `copy(doc, sel)` |
| `navigate_edge` | `doc, sheet, from: CellPos, dir: Direction` | `CellPos` | `navigate_edge(doc, sheet, from, dir)` | `navigateEdge(doc, sheet, from, dir)` |
| `resolve_name_box` | `doc, text` | `SelectionDto` | `resolve_name_box(doc, text)` | `resolveNameBox(doc, text)` |
| `list_functions` | none | `FunctionInfo[]` | `list_functions()` | `listFunctions()` |
| `assist_suggest` | `doc, query, sel: SelectionDto` | `Suggestion[]` | `assist_suggest(doc, query, sel)` | `assistSuggest(doc, query, sel)` |
| `recent_files` | none | `string[]` | shell only (app data) | `ShellClient.recentFiles()` |
| `list_templates` | none | `TemplateInfo[]` (`{ id, title, description }`) | shell only (the engine's `templates::CATALOG`) | `ShellClient.listTemplates()` |

### v0.2 commands

These extend `EngineClient` as `EngineClientV2` (`packages/bridge/src/client.ts`;
`MockClient` implements it with stubs). The command functions exist in
`apps/desktop/src-tauri/src/commands.rs`; registering them in `lib.rs`'s
`generate_handler!` and adding them to `TauriClient` is pending (WP7).
The trait methods have default implementations that return `unsupported`.

| Tauri command | Arguments | Returns | `DocumentService` method | `EngineClientV2` method |
|---|---|---|---|---|
| `names` | `doc` | `NameInfo[]` (`{ name, scope, refersTo, sheetName }`; workbook names first, then by sheet tab, each sorted by name) | `names(doc)` | `names(doc)` |
| `filter_info` | `doc, sheet` | `AutoFilterInfo` (`{ range, columns: FilterColumnInfo[] }`, `range: null` without a filter) | `filter_info(doc, sheet)` | `filterInfo(doc, sheet)` |
| `find` | `doc, query: FindQuery` | `FindResult` (`{ hits: FindHit[], truncated }`, ≤ 10,000 hits) | `find(doc, &query)` | `find(doc, query)` |
| `detect_header` | `doc, sheet, range: RangeDto` | `boolean` | `detect_header(doc, sheet, range)` | `detectHeader(doc, sheet, range)` |
| `recalc_all` | `doc` | `ApplyResult` | `recalc_all(doc)` (hard recalc, ⇧⌘F9; clears "recalculation incomplete") | `recalcAll(doc)` |

`wb_open_dialog`, `recent_files`, `list_templates` and the dialog half of `wb_save`
belong to the shell, not the engine, so they have no `DocumentService` method. They
form the `ShellClient` interface (`packages/bridge/src/shell.ts`), implemented by
`TauriClient` (`@tally/bridge/tauri`) and `MockClient` (in-memory: no Open dialog,
"save" renames the document, the three templates are built in memory).

## Ops (`Transaction.ops`)

Phase A ops: `setCellInput`, `setRangeInputs`, `clear`, `applyStyle`, `setNumberFormat`,
`insertRows`, `deleteRows`, `insertCols`, `deleteCols`, `setColWidth`, `setRowHeight`,
`addSheet`, `renameSheet`, `deleteSheet`, `moveSheet`, `paste`. Every op is undoable.

v0.2 ops (all undoable; refusals come back in `ApplyResult.errors` as `OpError`):

| `type` | Fields | Notes |
|---|---|---|
| `defineName` | `name, scope: SheetId \| null, refersTo` | `refersTo` is CalcA1 without `=` (`$Sheet1.$A$1:$B$5`, `0.07`). Names match `[A-Za-z_][A-Za-z0-9_.]*`, are not cell references, R1C1-like or TRUE/FALSE, and are unique per scope case-insensitively (redefining replaces). Users of the name recalculate. |
| `deleteName` | `name, scope` | Users show `#NAME?` and keep printing the name. |
| `setArrayFormula` | `sheet, range, input` | `input` starts with `=`; evaluated in array context, the result spread over `range` (outside a larger result `#N/A`; a single row/column repeats; a scalar fills the block). |
| `moveRange` | `sheet, from: RangeDto, to: CellPos` | Cut/paste. References into the block follow it (any sheet, names too); moved formulas keep outside targets; references to overwritten destination cells become `#REF!`. Merges and whole array blocks move with it. |
| `mergeCells` | `sheet, range` | Keeps only the top-left content (**the UI should warn** when other cells are non-empty); merges inside are absorbed; partial overlap → `merged`. |
| `unmergeCells` | `sheet, range` | Removes every merge intersecting `range`. |
| `setFrozen` | `sheet, rows, cols` | `0, 0` unfreezes. `SheetLayout.frozen` is `{ r: rows, c: cols }`. |
| `sortRange` | `sheet, range, keys: SortKey[] ({ col, ascending }), hasHeader, caseSensitive` | Stable; numbers < text < errors, empties last; see "Sort" below. Merges inside → `merged`; arrays → `array_block`. |
| `setAutoFilter` | `sheet, range: RangeDto \| null` | `null` removes the filter (and shows its rows). |
| `setColumnFilter` | `sheet, col, filter: ColumnFilter \| null` | `ColumnFilter = { values: string[] \| null, condition: { op: FilterOp, value } \| null }`; `FilterOp` = `eq ne gt ge lt le contains beginsWith endsWith empty notEmpty`. |
| `setRowsHidden` / `setColsHidden` | `sheet, start, end, hidden` | User-hidden, independent of filters; sizes are kept. |
| `replaceAll` | `sheet: SheetId \| null, query: FindQuery, replace` | Regex mode: `$1`, `&` = whole match, `\&` = `&`. Replacements that would not parse are reported (`invalid_argument` OpErrors) and skipped; array cells are skipped (`array_block` OpError). |
| `pasteSpecial` | `sheet, target, payload, options: PasteOptions` | `{ valuesOnly, formatsOnly, transpose, skipEmpty, operation: none \| add \| subtract \| multiply \| divide }`. TSV supports `valuesOnly` (`=…` stays text), `transpose`, `skipEmpty`. |
| `fillRange` | `sheet, source, target, mode: FillMode` | `target` contains `source` and extends it in one direction; `mode` = `auto copy linearSeries growthSeries date`. |

`applyStyle`'s `StylePatch` has an optional `borders: BordersPatch | null` with
`top bottom left right innerH innerV: { line: BorderLineDto | null } | null`
(`line: null` removes that edge; inner edges apply between the range's rows/columns).

Error codes added in v0.2: `array_block` (the op would change part of an array
formula), `merged` (part of a merged area).

Other v0.2 DTO additions (all optional, so older clients keep working):
`DimRunPx.filtered?: boolean | null` (rows hidden by an AutoFilter; also `hidden`),
`CellEditInfo.array?: boolean | null` and `arrayRange?: RangeDto | null` (array cells
show `{=…}` in `input`), `CellView.flags & 4` for array cells, `SheetLayout.merges`
and `SheetLayout.frozen` are now filled.

### Sort semantics

Rows move whole (values, formulas, styles). A formula reference whose target lies
entirely inside the sorted rows keeps its relative offsets, so a per-row formula
`=A2*B2` keeps reading its own record after the move (AOO behaviour); every other
reference keeps its absolute target. Use `detect_header` to pre-fill "range contains
column labels".

### Find semantics

`FindQuery = { text, sheet, inFormulas, matchCase, wholeCell, mode: literal \| wildcards \| regex, from }`.
Values mode matches the displayed text of non-empty cells; formulas mode matches
formula text (`=…`) and the edit text of constants. Wildcards: `*`, `?`, `~` escapes.
Regexes are size-limited (1 MiB compiled). Hits are row-major per sheet in tab order;
`from` (on `sheet`, or the active sheet) rotates them so the first hit follows it.

## Suggestions

`Suggestion` (⌘K) has `title`, `formula`, `explanation`, `preview` (computed by the
engine at the active cell without committing), `confidence`, `action` and
`actionArgs: ActionArg[]` (`{ key, value }`, empty for formula suggestions). Action ids
and their args from the rule provider:

| `action` | `actionArgs` | UI (Phase A) |
|---|---|---|
| `format.currency` | `column` (`F`), `range` (column body, `F2:F15`) | applies the currency format to `range` |
| `filter.byColumn` | `column`, `op` (`=`, `<>`, `>` …), `value` | toast "Filtering is coming in v0.2" |
| `chart.byColumn` | `value`, `by` (column letters) | toast "Charts are coming in v0.2" |

## Events

| Event | Payload | `EngineClient` |
|---|---|---|
| `doc-changed` | `DocChangedEvent { doc, result: ApplyResult }` | `onDocChanged(cb)` |
| `menu-action` | `MenuActionEvent { id: string }` | `onMenuAction(cb)` |

`doc-changed` carries only changes the UI did not initiate (volatile recalc, async
load). Results of the UI's own `apply`/`undo`/`redo` come back as return values.
