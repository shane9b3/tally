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
| `wb_open_dialog` | none | `WorkbookInfo \| null` | shell only: native dialog (filters: ODS, XLSX, CSV), then `open(path)` | `ShellClient.openDialog()` |
| `wb_open` | `path: string` | `WorkbookInfo` | `open(path)` | `open(path)` |
| `wb_save` | `doc, saveAs: boolean, format?: FileFormat` | `WorkbookInfo` | shell shows a dialog when `saveAs`, no path or a format change (Save As offers ODS, XLSX and CSV; without `format` the chosen extension decides), then `save(doc, path, format)` | `save(doc, path \| null, format)`; `ShellClient.saveWithDialog(doc, saveAs, format)` |
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

These extend `EngineClient` as `EngineClientV2` (`packages/bridge/src/client.ts`).
They are registered in `apps/desktop/src-tauri/src/lib.rs` and implemented by
`TauriClient` (`COMMANDS.names` … `COMMANDS.recalcAll`); `MockClient` has stubs and
the browser build's `WebMockClient` (`packages/app/src/dev/webMock.ts`) emulates them.
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

### v0.3 queries

`EngineClientV3` (`packages/bridge/src/client.ts`) adds the queries below; the
mutations are ops (next section). The `DocumentService` methods have default
implementations that return `unsupported`; `EngineService` implements all of them and
`MockClient` has in-memory stubs (stored, not evaluated). The commands are registered
in `apps/desktop/src-tauri/src/lib.rs` and implemented by `TauriClient`
(`COMMANDS.conditionalFormats` … `COMMANDS.resolveNameBoxIn`); the browser build's
`WebMockClient` emulates them (`packages/app/src/dev/mockV3.ts`, `mockCf.ts`).

| Tauri command | Arguments | Returns | `DocumentService` method | `EngineClientV3` method |
|---|---|---|---|---|
| `conditional_formats` | `doc, sheet` | `ConditionalFormatDto[]` (priority order) | `conditional_formats(doc, sheet)` | `conditionalFormats(doc, sheet)` |
| `validations` | `doc, sheet` | `ValidationDto[]` | `validations(doc, sheet)` | `validations(doc, sheet)` |
| `validation_at` | `doc, sheet, r, c` | `ValidationInfo \| null` (list items resolved, ≤ 10,000) | `validation_at(doc, sheet, r, c)` | `validationAt(doc, sheet, r, c)` |
| `note` | `doc, sheet, r, c` | `NoteDto \| null` | `note(doc, sheet, r, c)` | `note(doc, sheet, r, c)` |
| `notes` | `doc, sheet` | `NoteDto[]` (row-major) | `notes(doc, sheet)` | `notes(doc, sheet)` |
| `protection_info` | `doc, sheet` | `ProtectionInfo` | `protection_info(doc, sheet)` | `protectionInfo(doc, sheet)` |
| `page_setup` | `doc, sheet` | `PageSetupDto` (with `rowBreaks`/`colBreaks`) | `page_setup(doc, sheet)` | `pageSetup(doc, sheet)` |
| `document_properties` | `doc` | `DocPropertiesDto` | `document_properties(doc)` | `documentProperties(doc)` |
| `goal_seek` | `doc, req: GoalSeekRequest` | `GoalSeekResult` | `goal_seek(doc, &req)` | `goalSeek(doc, req)` |
| `list_locales` | none | `LocaleInfo[]` | `list_locales()` | `listLocales()` |
| `set_locale` | `doc, locale` | `ApplyResult` (every sheet's used area changed) | `set_locale(doc, locale)` | `setLocale(doc, locale)` |
| `resolve_name_box_in` | `doc, sheet, text` | `SelectionDto` | `resolve_name_box_in(doc, sheet, text)` (names and unqualified refs relative to `sheet`, not the engine's active sheet; the name box uses it with the sheet the UI shows) | `resolveNameBoxIn(doc, sheet, text)` |

v0.3 shell commands (no `DocumentService` method; `TauriClient.openUrl` / `userName`,
optional in the UI's `LinkShell` interface, `packages/app/src/features.ts`):

| Tauri command | Arguments | Returns | What it does |
|---|---|---|---|
| `open_url` | `url: string` | `void` | Opens an `http(s)://host…` or `mailto:` link (≤ 2048 bytes) with the OS handler; anything else (`file:`, `javascript:`, …) is `invalid_argument`. See docs/security.md, "Hyperlinks". |
| `user_name` | none | `string \| null` | The OS account name (note authors). |

### Argument limits

Every command checks its arguments before the engine sees them
(`apps/desktop/src-tauri/src/limits.rs`) and rejects violations with
`invalid_argument` (the whole call; nothing is applied):

| Argument | Limit |
|---|---|
| Find / replace text, regular expressions (`find`, `replaceAll`) | ≤ 64 KiB |
| Cell input, formulas, every other string inside an op | ≤ 64 KiB |
| Defined names, sheet names | ≤ 256 characters |
| Name box text (`resolve_name_box`, `resolve_name_box_in`) | ≤ 1024 characters |
| TSV paste payload (`paste`, `pasteSpecial`); all strings of one transaction | ≤ 64 MiB |
| Ranges and cell positions (every `RangeDto`, `CellPos`, `{r, c}`) | `r0 ≤ r1 < 1,048,576`, `c0 ≤ c1 < 16,384` |
| Row/column inserts and deletes | `at + count` inside the sheet |
| Ranges per list; ops per transaction; style ids per `get_styles` | 10,000; 100,000; 65,536 |
| `open_url` | ≤ 2048 bytes, `http`/`https`/`mailto` only |

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

### v0.3 ops

All undoable. Formulas in v0.3 DTOs are UI-dialect text without the leading `=` (a
leading `=` is accepted), relative to the top-left cell of the first range; a
cell-value operand that does not parse (e.g. `Paid`) is taken as text.

| `type` | Fields | Notes |
|---|---|---|
| `addConditionalFormat` | `sheet, ranges, rules: CfRuleDto[]` | `CfRuleDto = { kind: CfRuleKind, style: CfStyleDto, stopIfTrue }`. Kinds: `cellValue {op: CmpOp, formulas}`, `formula`, `colorScale {stops: [{threshold, color}] ×2–3}`, `dataBar {min, max, color, negativeColor, showValue, gradient}`, `iconSet {set: IconSetKind, thresholds (one per icon), showValue, reverse}`, `duplicate`, `unique`, `top {rank, percent, bottom}`, `average {above, equal}`, `text {op: contains/notContains/beginsWith/endsWith, text}`, `blank`, `notBlank`, `error`, `notError`, `dateIs {period}`. `CfThreshold = { kind: min/max/percent/percentile/number/formula/autoMin/autoMax, value }`. The id is assigned by the engine (see `conditional_formats`). |
| `updateConditionalFormat` | `sheet, id, ranges, rules` | Replaces ranges and rules. |
| `deleteConditionalFormat` | `sheet, id` | |
| `addValidation` | `sheet, validation: ValidationDto` | `kind`: `any`, `list {source: values {items} \| range {reference}}`, `whole/decimal/date/time/textLength {op, formulas}`, `custom {formula}`; `allowBlank`, `showDropdown`, `inputHelp {title, text}`, `error {style: stop/warning/info, title, text}`. A new validation replaces older ones where they overlap (one rule per cell, as in Calc). |
| `updateValidation` / `deleteValidation` | `sheet, id[, validation]` | |
| `setNote` | `sheet, r, c, note: NoteDto` | `created: null` → now. Notes move with insert/delete/move/sort; delete rows/cols and "Clear all" remove them. |
| `deleteNote` | `sheet, r, c` | |
| `setLink` | `sheet, r, c, url` | `http(s)`, `mailto`, `file`, `#Sheet.A1`; `javascript:`/`data:`/`vbscript:` are refused. |
| `clearLink` | `sheet, ranges` | |
| `protectSheet` | `sheet, password, options: ProtectionOptions \| null` | Locked cells can no longer be edited (`protected` OpError); `options` allow inserting/deleting rows/cols, formatting, sorting, AutoFilter. |
| `unprotectSheet` | `sheet, password` | Wrong password → `protected`. |
| `setCellProtection` | `sheet, ranges, locked, hiddenFormula` | Refused while the sheet is protected. Cells are locked by default. Hidden formulas show an empty `cellEditInfo.input` while protected. |
| `protectDocument` / `unprotectDocument` | `password` | Structure protection: sheets cannot be added, deleted, renamed or moved. |
| `setPageSetup` | `sheet, setup: PageSetupDto` | Print model only (no rendering yet). Header/footer placeholders `&[PAGE] &[PAGES] &[SHEET] &[FILE] &[DATE] &[TIME]`. |
| `setPageBreak` | `sheet, axis: row \| col, at, present` | Manual break above row / left of column `at` (> 0). |
| `setDocumentProperties` | `props: DocPropertiesDto` | Title, subject, keywords, description, author (`created`/`modified`/`generator` are read-only; `modified` is set on save). |
| `autoFitRows` | `sheet, start, end` | Rows with wrapped, multi-line or larger-than-default text get the height they need (Inter advance-width table, 1.3 line height, ≤ 409 pt); others keep theirs. New heights in `ApplyResult.fittedRows`. |
| `refreshFilter` | `sheet` | Re-applies the AutoFilter to the current values (filters are no longer re-applied after every edit, as in AOO). |

Validation: typed input (`setCellInput`, `setRangeInputs`), pastes and fills are
checked after they are written. A `stop` alert reverts the op and returns
`OpError { code: "validation", message: error text }`; `warning`/`info` keep the value and
add an `OpError` to `ApplyResult.warnings`. Clearing cells is never checked; a
validation without `error` accepts anything.

Protection: on a protected sheet, edits of locked cells (`setCellInput`, ranges, clear,
paste, fill, replace, move, array formulas, notes, links) fail with `protected`;
formatting needs `options.formatCells` (or unlocked cells); structure ops, merges,
row/column sizes, AutoFilter changes, conditional formats and validations are refused
unless an option allows them. Password hashes are stored in the file formats' own
forms (ODF SHA-256/SHA-1 base64, OOXML legacy 16-bit or SHA-512 + salt + spin
count; a SHA-256 password set in Tally also stores the legacy Excel hash for XLSX).
This is not security: the files are not encrypted.

Error codes added in v0.3: `validation`, `protected`.

v0.3 DTO additions (all optional):
- `CellView.cfStyle?: number | null`: a synthesized style id (≥ 0x40000000) combining the
  cell's style with the matching conditional styles; resolve it with `get_styles` like any
  other id. A conditional number format also changes `text`.
- `CellView.cfBar?: DataBarView | null` (`{ fraction, color, negative, axis, gradient }`),
  `CellView.cfIcon?: IconView | null` (`{ set, index }`, index 0 = lowest); with
  `showValue: false` the `text` is empty.
- `CellView.link?: string | null` (the link attribute, or the URL of a top-level
  `HYPERLINK()` formula); flags `8` = note, `16` = list validation with a dropdown,
  `32` = link. Empty cells with a note, link, dropdown or matching conditional style get
  a `CellView`.
- `ApplyResult.warnings?: OpError[] | null` (validation warnings/info),
  `ApplyResult.fittedRows?: DimRunPx[] | null` (`autoFitRows`),
  `ApplyResult.replaced?: number | null` (cells changed by `replaceAll`).

Conditional formatting evaluation: rules are evaluated for the cells `get_cells`
returns (viewport-bound). Range-wide statistics (colour scales, data bars, icon
sets, top/bottom N, averages, duplicates) are cached per document version.
Style rules combine by priority (first rule setting a property wins); the first colour
scale sets the fill if no style rule did; the first data bar and icon set apply;
`stopIfTrue` ends the evaluation. Empty cells compare as 0 (or `""` against text), as in
Excel.

Locales: `set_locale` switches number formatting, input recognition (`1.234,5`,
`19.10.2026`, `12,50 €` in de-DE) and the UI formula separator (`;` where the decimal
separator is `,`). Function names stay English; formula text typed in the UI must still
use `.` as the decimal separator in v0.3. Persisted as the default cell style's
language in ODS.

### Sort semantics

Rows move whole (values, formulas, styles, notes and links). Every relative formula
reference keeps its relative offsets, as in AOO (`ScColumn::SwapRow`): a per-row
formula `=A2*B2` keeps reading its own record after the move, and a relative reference
outside the range (`=D1` from row 1) moves with the row too (`=D2` from row 2).
Absolute references (`$D$1`) are unaffected. Use `detect_header` to pre-fill "range
contains column labels".

### AutoFilter semantics

The filtered rows are recomputed when a column filter or the filter area changes (and
when that change is undone or redone), on a row/column change of the sheet, on a hard
recalc and on `refreshFilter`; not after ordinary edits (AOO behaviour). SUBTOTAL and
AGGREGATE formulas that read a sheet are recalculated whenever its row visibility
changes.

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
| `filter.byColumn` | `column`, `op` (`=`, `<>`, `>` …), `value` | turns the AutoFilter on (data region) and sets a condition on that column (v0.2) |
| `chart.byColumn` | `value`, `by` (column letters) | toast "Charts are coming in v0.2" |

## Events

| Event | Payload | `EngineClient` |
|---|---|---|
| `doc-changed` | `DocChangedEvent { doc, result: ApplyResult }` | `onDocChanged(cb)` |
| `menu-action` | `MenuActionEvent { id: string }` | `onMenuAction(cb)` |

Menu ids are `CommandRegistry` command ids (`packages/app/src/commands.ts`); the
native menu (`apps/desktop/src-tauri/src/menu.rs`) lists them, and
`packages/app/test/menuIds.test.ts` checks every one is registered.

`doc-changed` carries only changes the UI did not initiate (volatile recalc, async
load). Results of the UI's own `apply`/`undo`/`redo` come back as return values.
