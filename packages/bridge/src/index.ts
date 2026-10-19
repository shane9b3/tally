/**
 * @tally/bridge: types generated from crates/tally-api (ts-rs), the
 * EngineClient and ShellClient interfaces, helpers and the in-memory
 * MockClient. `TauriClient` is a separate entry point (`@tally/bridge/tauri`).
 *
 * Regenerate the types with `cargo xtask gen-bindings`; never edit
 * src/generated/ by hand.
 */

export type { ActionArg } from "./generated/ActionArg";
export type { ApiError } from "./generated/ApiError";
export type { ApplyResult } from "./generated/ApplyResult";
export type { AutoFilterInfo } from "./generated/AutoFilterInfo";
export type { BorderEdgePatch } from "./generated/BorderEdgePatch";
export type { BorderLineDto } from "./generated/BorderLineDto";
export type { BorderStyleDto } from "./generated/BorderStyleDto";
export type { BordersDto } from "./generated/BordersDto";
export type { BordersPatch } from "./generated/BordersPatch";
export type { CellBlock } from "./generated/CellBlock";
export type { CellEditInfo } from "./generated/CellEditInfo";
export type { CellPos } from "./generated/CellPos";
export type { CellView } from "./generated/CellView";
export type { ClearWhat } from "./generated/ClearWhat";
export type { ClipboardOut } from "./generated/ClipboardOut";
export type { ColumnFilter } from "./generated/ColumnFilter";
export type { DimRunPx } from "./generated/DimRunPx";
export type { Direction } from "./generated/Direction";
export type { DisplayDto } from "./generated/DisplayDto";
export type { DocChangedEvent } from "./generated/DocChangedEvent";
export type { DocId } from "./generated/DocId";
export type { FileFormat } from "./generated/FileFormat";
export type { FillMode } from "./generated/FillMode";
export type { FilterColumnInfo } from "./generated/FilterColumnInfo";
export type { FilterCondition } from "./generated/FilterCondition";
export type { FilterOp } from "./generated/FilterOp";
export type { FindHit } from "./generated/FindHit";
export type { FindMode } from "./generated/FindMode";
export type { FindQuery } from "./generated/FindQuery";
export type { FindResult } from "./generated/FindResult";
export type { FunctionInfo } from "./generated/FunctionInfo";
export type { HAlign } from "./generated/HAlign";
export type { MenuActionEvent } from "./generated/MenuActionEvent";
export type { NameInfo } from "./generated/NameInfo";
export type { Op } from "./generated/Op";
export type { OpError } from "./generated/OpError";
export type { PasteOperation } from "./generated/PasteOperation";
export type { PasteOptions } from "./generated/PasteOptions";
export type { PastePayload } from "./generated/PastePayload";
export type { RangeDto } from "./generated/RangeDto";
export type { SelectionDto } from "./generated/SelectionDto";
export type { SelectionStats } from "./generated/SelectionStats";
export type { SheetInfo } from "./generated/SheetInfo";
export type { SheetLayout } from "./generated/SheetLayout";
export type { SheetRanges } from "./generated/SheetRanges";
export type { SortKey } from "./generated/SortKey";
export type { StyleDto } from "./generated/StyleDto";
export type { StylePatch } from "./generated/StylePatch";
export type { Suggestion } from "./generated/Suggestion";
export type { Transaction } from "./generated/Transaction";
export type { UndoState } from "./generated/UndoState";
export type { VAlign } from "./generated/VAlign";
export type { ValueKind } from "./generated/ValueKind";
export type { WorkbookInfo } from "./generated/WorkbookInfo";

export { isApiError } from "./client";
export type { EngineClient, EngineClientV2, SheetId, Unsubscribe } from "./client";
export { MockClient, DEFAULT_COL_WIDTH_PX, DEFAULT_ROW_HEIGHT_PX } from "./mock";
export { BUILTIN_TEMPLATES, CANCELLED } from "./shell";
export type { ShellClient, TemplateInfo } from "./shell";
