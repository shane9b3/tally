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
  Codes: `not_found`, `invalid_argument`, `io`, `bad_file`, `unsupported`, `internal`.
- `DocId` and `SheetId` are numbers. A `SheetId` is stable and is not the tab index.
- Optional values are `T | null` in both directions.
- Enums with data are discriminated unions on `type` (`{ type: "setCellInput", ... }`);
  plain enums are camelCase strings (`"ods"`, `"down"`).

## Commands

| Tauri command | Arguments | Returns | `DocumentService` method | `EngineClient` method |
|---|---|---|---|---|
| `wb_new` | `template?: string` | `WorkbookInfo` | `new_workbook(template)` | `newWorkbook(template?)` |
| `wb_open_dialog` | none | `WorkbookInfo \| null` | shell only: native dialog, then `open(path)` | (WP7, Tauri-specific) |
| `wb_open` | `path: string` | `WorkbookInfo` | `open(path)` | `open(path)` |
| `wb_save` | `doc, saveAs: boolean, format?: FileFormat` | `WorkbookInfo` | shell shows a dialog when `saveAs` or no path, then `save(doc, path, format)` | `save(doc, path \| null, format)` |
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
| `recent_files` | none | `string[]` | shell only (app data) | (WP7, Tauri-specific) |
| `list_templates` | none | (WP7 defines) | shell only (bundled resources) | (WP7, Tauri-specific) |

`wb_open_dialog`, `recent_files`, `list_templates` and the dialog half of `wb_save`
belong to the shell, not the engine, so they have no `DocumentService` method. WP7
adds them to `TauriClient` (and optionally to `EngineClient` as an additive change).

## Events

| Event | Payload | `EngineClient` |
|---|---|---|
| `doc-changed` | `DocChangedEvent { doc, result: ApplyResult }` | `onDocChanged(cb)` |
| `menu-action` | `MenuActionEvent { id: string }` | `onMenuAction(cb)` |

`doc-changed` carries only changes the UI did not initiate (volatile recalc, async
load). Results of the UI's own `apply`/`undo`/`redo` come back as return values.
