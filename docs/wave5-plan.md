# Tally wave 5: work-package briefs for Calc parity (charts, DataPilot, print/PDF, remaining inventory, 1.0 exit)

Everything here is based on HEAD `c98f956`. The working tree had one untracked file (`apps/desktop/src-tauri/src/limits.rs`) and no tracked changes. Versions I checked on this machine:

| Item | Version | Notes |
|---|---|---|
| echarts | **6.1.0** (Apache-2.0) | from `pnpm view echarts version`, latest tag |
| rust_xlsxwriter | 0.100.0 | supports charts, but not Bubble or Stock with volume |
| tauri / wry | 2.12.4 / 0.57 | `Webview::print()` runs only on macOS (it opens the modal WKWebView `printOperationWithPrintInfo`). It cannot write a PDF to a path without the dialog. |
| PDF stack | krilla 0.8.2, krilla-svg 0.8.1, subsetter 0.2.6, hayro 0.7.1, hayro-syntax 0.7.2, fontdb 0.23, ttf-parser 0.25 | all in `~/.cargo/registry`; MIT or MIT/Apache; rust-version 1.92, below the pinned 1.94 |
| calamine | 0.37 | in the registry; reads XLS |

Where the code stands today (relevant to the design):
- **No settings API.** `CalcSettings` (`crates/tally-types/src/settings.rs`) already has `iterations: Option<IterSettings>` (steps 100, min 0.001) and `precision_as_shown`. Both are read from and written to ODS, but recalc ignores them: cycles always give Err:522, and nothing rounds to the displayed precision.
- **Undo pattern.** `ops_v03.rs` changes per-sheet state through `with_meta()`, which records `Change::SheetMeta`. The ops dispatch through `ops::apply_op`, which calls `ops_v03::apply` first. `structure.rs:258 adjust_meta_for_structure` moves CF and validation ranges, including on other sheets (lines 305–340). Wave 5 copies these patterns.
- **ApplyResult** has no object or chart signals. New optional fields must follow the existing `#[serde(default)] #[ts(optional = nullable)]` convention.
- **Page setup.** `PageScale` has only `Percent` and `FitTo{width,height}`, so AOO's "fit to N pages" mode is missing. There is no pagination code: `pagesetup.rs` holds the model only.
- **Functions.** `FunctionInfo` has no per-argument help. `docs/calc-functions.json` has `params[].description` for all 405 functions. `tally_functions::registry::Param` has names only.
- **e2e** runs against `WebMockClient` (`packages/app/src/dev/webMock.ts`), so every new flow needs a mock emulation.
- **Native menu.** `packages/app/test/menuIds.test.ts` requires every `menu.rs` item to be a registered command.
- **Fonts.** The UI bundles Inter only as WOFF2 (`@fontsource-variable/inter`), which a PDF writer can't use, so static Inter TTFs have to be vendored (OFL-1.1).

---

## 0. Ownership, sequencing and seams

Owners:

| Owner | Files |
|---|---|
| **ENG** (engine agent) | `crates/tally-engine`, `tally-api`, `tally-model`, `tally-types`, `tally-formula`, `tally-numfmt`, all `tally-io-*` (including the new `tally-io-pdf` and `tally-io-xls`), `packages/bridge/src/generated/**`, and `packages/bridge/src/client.ts` (the interface only) |
| **UI** | `packages/app`, `packages/grid`, `packages/tokens`, `apps/desktop/**`, `packages/bridge/src/{tauri.ts,mock.ts,mockTemplates.ts,shell.ts}`, `tests/e2e/**` |
| **FN** (functions agent) | `crates/tally-functions/**`, `tests/conformance/**` |

I'm assuming `tally-formula` and `tally-numfmt` stay with ENG, as they did in Phase A.

Sequence:

1. **W5-0a Contracts (ENG, 2 days, blocks everything).** One PR that adds:
   - DTO modules `crates/tally-api/src/{charts.rs,pivot.rs,print.rs,tools.rs}`, re-exported from `lib.rs`.
   - Every new `Op` variant (the `Op` enum is in `dto.rs`, which every package would otherwise touch).
   - The new `ApplyResult` fields.
   - Default `DocumentService` methods that return `unsupported`.
   - Model fields and `SheetMeta` fields.
   - Dispatch stubs `ops_v04::apply` and `ops_v05::apply` in `ops::apply_op`.
   - Empty hook calls `charts::adjust_structure`, `outline::adjust_structure` and `pivot::adjust_structure` in `structure.rs`, and `charts::{to_data,from_data}` and the others in `convert.rs`.
   - Empty dispatch arms in `tally-io-ods/src/content.rs` for `draw:frame`, `table:shapes`, `table:table-row-group`, `table:table-column-group`, `table:data-pilot-tables` and `table:scenario`.
   - Regenerated bindings and `EngineClientV4` in `client.ts`.
   - A "v0.4–v0.6" section in `docs/api.md`.

   After this PR every work package edits only its own files.
2. **W5-0b Shell seams (UI, 1 day, right after W5-0a).**
   - Thin Tauri commands for all new methods in a new `apps/desktop/src-tauri/src/commands_v4.rs`, registered once in `lib.rs`.
   - `TauriClient` and `MockClient` stubs.
   - `menu.rs` items and the matching registry commands, initially `enabled:false` with `reason:"coming in wave 5"` so `menuIds.test.ts` stays green.
   - Grid seams (see W5-5).
3. Then W5-1 to W5-6 run in parallel. W5-1, W5-2 and W5-3 are ENG; one ENG agent can run them sequentially, or three can run them in parallel. W5-5 and W5-6 are UI. W5-4 is FN.

New `ApplyResult` fields (additive, serde default, `ts(optional = nullable)`):

```rust
/// Charts whose resolved data changed (cells in a series range changed, recalculated,
/// or became hidden/visible). The UI re-fetches `chart_data` for the visible ones.
pub charts_changed: Option<Vec<ChartRef>>,
/// Sheets whose object list changed (chart added/removed/moved/updated). Re-fetch `charts()`.
pub objects_changed: Option<Vec<SheetId>>,
/// Ids allocated by Add*/Create* ops, in op order.
pub created: Option<Vec<CreatedId>>,          // CreatedId { op_index: u32, id: u32 }
/// The pivot table list changed (create/update/delete/refresh).
pub pivots_changed: Option<bool>,
```

---

## W5-1 Charts: model, engine, ODS, XLSX (ENG)

### Model (`crates/tally-model/src/charts.rs`, re-exported; `SheetData.charts: Vec<ChartData>` with `#[serde(default)]`)
The model doesn't depend on any file format.
- Ranges are `ChartRangeData { areas: Vec<(u32 /*sheet index*/, CellRangeLocal)>, invalid: bool }`. An empty, non-invalid value means "not set".
- Literal data is `ChartValuesData::Literal(Vec<Option<f64>>)` or `LiteralText(Vec<String>)`. It's used for XLSX `c:numLit`/`c:strLit` and for an ODS `chart:local-table` that has no ranges.
- The anchor is "to cell", as in AOO: `ChartAnchorData { row, col, dx_hmm, dy_hmm, width_hmm, height_hmm }`. The size is absolute, the position follows the cell, and the end cell is computed when writing.
- The other fields mirror the DTO below.

### DTOs (`crates/tally-api/src/charts.rs`, same serde and ts-rs conventions as `features.rs`)
Range fields are text in the UI dialect, like CF formulas (for example `$Sheet1.$B$2:$B$9`; a `~` union is allowed). The engine parses them with `parse_reference_text`, stores the resolved ranges, and returns canonical text. Unparsable text becomes `OpError invalid_argument`. A reference to a deleted sheet becomes `#REF!`, and that series renders empty.

```rust
pub struct ChartRef { pub sheet: SheetId, pub id: u32 }
pub struct ChartAnchor { pub r: u32, pub c: u32, pub dx_mm: f32, pub dy_mm: f32, pub width_mm: f32, pub height_mm: f32 }
pub enum ChartKind { Column, Bar, Line, Area, Pie, Scatter, Bubble, Radar, Stock }   // camelCase strings
pub enum Stacking { None, Stacked, Percent }
pub enum StockVariant { Hlc, Ohlc, VolumeHlc, VolumeOhlc }
pub enum LegendPos { None, Left, Right, Top, Bottom }
pub struct ChartDto {
  pub id: u32, pub name: String,            // "Object 1" from ODS; ignored on add
  pub anchor: ChartAnchor,
  pub kind: ChartKind, pub stacking: Stacking,
  pub lines: bool, pub markers: bool, pub smooth: bool,      // line/scatter/radar variants
  pub hole: Option<f32>, pub exploded: bool,                 // pie: Some(0.5) = donut
  pub filled: bool,                                          // radar
  pub stock: Option<StockVariant>,
  pub three_d: bool,                                         // stored for round trip; rendered 2D
  pub source: Option<ChartSourceDto>,                        // wizard "Data range"
  pub categories: Option<String>,
  pub series: Vec<ChartSeriesDto>,
  pub title: Option<String>, pub subtitle: Option<String>,
  pub legend: LegendPos,
  pub x_axis: ChartAxisDto, pub y_axis: ChartAxisDto, pub y2_axis: Option<ChartAxisDto>,
  pub background: Option<u32>, pub plot_background: Option<u32>,
  pub include_hidden: bool,                                  // AOO default false
}
pub struct ChartSourceDto { pub range: String, pub series_in_rows: bool, pub first_row_label: bool, pub first_col_label: bool }
pub struct ChartSeriesDto {
  pub name: Option<SeriesName>,                 // tagged: { type:"ref", ref } | { type:"text", text }
  pub values: String, pub x_values: Option<String>, pub sizes: Option<String>,
  pub kind: Option<ChartKind>,                  // combo override (Line on a Column chart)
  pub secondary_axis: bool, pub color: Option<u32>, pub marker: Option<MarkerShape>,
  pub labels: DataLabelsDto,                    // { value, percent, category, seriesName, position: Auto|Outside|Inside|Center|Above|Below }
  pub trend: Option<TrendLineDto>,              // { kind: Linear|Logarithmic|Exponential|Power, showEquation, showR2, name }
  pub error_bars: Option<ErrorBarsDto>,         // { kind, direction: Both|Plus|Minus }
  pub mean_line: bool,
}
pub enum ErrorBarKind { Constant{plus:f64,minus:f64}, Percent{percent:f64}, StdDev{k:f64}, Variance,
                        StdError, ErrorMargin{percent:f64}, Range{plus:String, minus:String} }
pub struct ChartAxisDto { pub visible: bool, pub title: Option<String>, pub min: Option<f64>, pub max: Option<f64>,
  pub major: Option<f64>, pub minor_count: Option<u32>, pub log: bool, pub reverse: bool,
  pub major_grid: bool, pub minor_grid: bool, pub number_format: Option<String> }
// Query result
pub struct ChartDataDto { pub chart: ChartRef, #[ts(type="number")] pub version: u64,
  pub categories: Vec<String>, pub series: Vec<SeriesDataDto>, pub truncated: bool, pub problems: Vec<String> }
pub struct SeriesDataDto { pub name: String, pub color: u32, pub values: Vec<Option<f64>>,
  pub x: Option<Vec<Option<f64>>>, pub sizes: Option<Vec<Option<f64>>>, pub labels: Vec<String>,
  pub number_format: Option<String>, pub trend: Option<TrendDataDto>, pub errors: Option<Vec<[f64;2]>>, pub mean: Option<f64> }
pub struct TrendDataDto { pub points: Vec<[f64;2]>, pub equation: String, pub r2: f64 }
```
- AOO 4.1 has no polynomial or moving-average trend lines. When XLSX import finds them, they are kept as `Linear` and reported as a warning.
- The default palette is AOO's: `004586 ff420e ffd320 579d1c 7e0021 83caff 314004 aecf00 4b1f6f ff950e c5000b 0084d1`. The engine resolves `SeriesDataDto.color`.

**Ops** (all undoable; refused on a protected sheet, because AOO protects objects with the sheet):
- `AddChart { sheet, chart }`. When `series` is empty and `source` is set, the engine derives the series. The id is returned in `created`.
- `UpdateChart { sheet, id, chart }`: full replace; the id is kept.
- `MoveResizeChart { sheet, id, anchor }`
- `DeleteChart { sheet, id }`
- `MoveChartToSheet { sheet, id, to }`

**Queries** (`DocumentService` methods, Tauri command names in brackets):
- `charts(doc, sheet) -> Vec<ChartDto>` [`charts`]
- `chart_data(doc, sheet, id) -> ChartDataDto` [`chart_data`]. It reads cached values without recalculating. Caps: ≤255 series and ≤32,000 points per series; anything beyond sets `truncated`. Target: ≤20 ms for 10k points.
- `suggest_chart(doc, sheet, sel: RangeDto, kind) -> ChartDto` [`suggest_chart`]. It auto-detects the region (current region around the cursor when only one cell is selected, reusing `tally-assist/region.rs` logic or a copy in `charts.rs`). The AOO heuristic: more rows than columns means series in columns; a text first row means labels; a text first column means categories. The anchor goes to the right of the region; the default size is 16×9 cm, as in AOO.

**Engine files:**
- `crates/tally-engine/src/charts.rs`: DTO↔model conversion, validation, data resolution (formatted labels through `service/view.rs` formatting), trend math (least squares on transformed axes; R² identical to `RSQ`), error-bar math, and `ChartIndex`.
- `crates/tally-engine/src/ops_v04.rs`
- `SheetMeta.charts: Vec<charts::Chart>`. Ids come from `meta.alloc_id()`, and undo goes through `with_meta`.

**Refresh:** `Document::result` intersects `report.changed` rectangles with the chart areas (a `ChartIndex` rebuilt only when `objects_changed`). Visibility changes in `layout_changed` also count when `include_hidden` is false. The result is `charts_changed`, and the same goes out in `doc-changed` events.

**Following edits:**
- Structure changes shift anchors with `point_after` and ranges with `span_after`. A fully deleted range becomes `invalid`. Charts on other sheets are handled too, mirroring the CF code at `structure.rs:305–340`.
- `MoveRange` moves areas that lie fully inside the cut source (AOO behaviour).
- `DeleteSheet` invalidates series that reference it.
- Column-width and row-height changes leave the size unchanged.

### ODS (`crates/tally-io-ods/src/chart.rs` for the object XML read/write, `src/frame.rs` for the `draw:frame` in the cell and the manifest)
Write (inside the anchor cell, as AOO does for charts anchored to a cell):
```xml
<draw:frame draw:z-index="0" draw:name="Object 1" svg:x="0.2cm" svg:y="0.1cm" svg:width="16cm" svg:height="9cm"
  table:end-cell-address="Sheet1.K20" table:end-x="0.5cm" table:end-y="0.3cm">
 <draw:object draw:notify-on-update-of-ranges="Sheet1.A1:Sheet1.C6" xlink:href="./Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>
</draw:frame>
```
- The manifest gets `Object 1/` (`application/vnd.oasis.opendocument.chart`), `Object 1/content.xml` and `Object 1/styles.xml`.
- `Object N/content.xml` contains `office:chart/chart:chart chart:class=… svg:width svg:height`, `chart:title`, `chart:subtitle`, `chart:legend chart:legend-position="end|start|top|bottom"`, `chart:plot-area table:cell-range-address chart:data-source-has-labels="both|row|column|none"`, `chart:axis chart:dimension="x|y" chart:name="primary-x|primary-y|secondary-y"` with `chart:categories table:cell-range-address` and `chart:grid chart:class="major|minor"`, and `chart:series chart:values-cell-range-address chart:label-cell-address chart:class` (for combo) and `chart:attached-axis`.
- Series children: `chart:domain` (scatter X, then bubble X), `chart:regression-curve`, `chart:error-indicator chart:dimension="y"` and `chart:mean-value`.
- A `table:table table:name="local-table"` holds cached values so other viewers can draw the chart.
- Class mapping: column `chart:bar`; bar `chart:bar` with `chart:vertical="true"`; `chart:line`; `chart:area`; pie `chart:circle`; donut `chart:ring`; `chart:scatter`; `chart:bubble`; `chart:radar` or `chart:filled-radar`; `chart:stock` (volume is a first series with `chart:class="chart:bar"` on the secondary y axis).
- `style:chart-properties` attributes: `chart:stacked`, `chart:percentage`, `chart:three-dimensional`, `chart:symbol-type="none|automatic"`, `chart:interpolation="cubic-spline"`, `chart:minimum`/`maximum`/`interval-major`/`interval-minor-divisor`/`logarithmic`/`reverse-direction`, `chart:data-label-number="value|percentage|value-and-percentage"`, `chart:data-label-text`, `chart:regression-type`, `chart:error-category` with the `error-*` attributes, and `chart:japanese-candle-stick`. Colours use `draw:fill-color` and `svg:stroke-color`.
- Ranges are printed with `v03::range_list` quoting.

Read (security):
- Frames are taken from cells and from `table:shapes` (page anchor converted to a cell anchor).
- `xlink:href` must match `^\./?Object \d{1,5}$` and the manifest must list it as a chart. Otherwise the frame is skipped with a warning.
- Limits: at most 1,000 charts per document, 16 MB per object XML, 255 series. The existing XML depth and attribute caps and the `Package::with_entry` lookup apply.
- Images and OLE objects that aren't charts are skipped and counted in the report.

### XLSX (`crates/tally-io-xlsx/src/chart_write.rs`, `chart_read.rs`, `drawing_read.rs`)

Writing uses rust_xlsxwriter:

| Tally | rust_xlsxwriter |
|---|---|
| Column/Bar/Area/Line × None/Stacked/Percent | `Column`, `ColumnStacked`, `ColumnPercentStacked` (same for Bar, Area, Line) |
| Pie / donut | `Pie` / `Doughnut` (hole size set) |
| Scatter points / lines / lines+markers / smooth | `Scatter`, `ScatterStraight`, `ScatterStraightWithMarkers`, `ScatterSmooth(WithMarkers)` |
| Radar plain / markers / filled | `Radar`, `RadarWithMarkers`, `RadarFilled` |
| Stock HLC/OHLC | `Stock` (high-low lines, up-down bars) |
| Stock with volume | written as `Stock` without volume, plus a warning |
| Bubble | not supported: written as `Scatter` with a warning (writing our own DrawingML through the `write_v03::patches` pass is a stretch goal) |
| Combo | `chart.combine(&line)`; `set_secondary_axis` |
| Trend / error bars / labels | `ChartTrendline`, `ChartErrorBars`, `ChartDataLabel` |
| 3D, multi-area ranges | written as 2D / first area only, plus a warning |

Placement: `insert_chart_with_offset(row, col, &chart, dx_px, dy_px)` plus `set_width`/`set_height` in px, converted with `units::hmm_to_px`.

The reader is our own:
- Path: sheet rels `drawing` → `xdr:twoCellAnchor`/`oneCellAnchor`/`absoluteAnchor` (EMU: 1 hmm = 360 EMU) → `c:chart r:id` → `xl/charts/chartN.xml`.
- Chart groups read: `c:barChart` (barDir, grouping), `c:bar3DChart`, `c:lineChart`, `c:line3DChart`, `c:areaChart`, `c:pieChart`, `c:pie3DChart`, `c:doughnutChart` (holeSize), `c:ofPieChart` (read as pie), `c:scatterChart` (scatterStyle), `c:bubbleChart`, `c:radarChart`, `c:stockChart`. Surface charts are skipped with a warning.
- Series: `c:ser` with `c:tx`, `c:cat`, `c:val`, `c:xVal`, `c:yVal`, `c:bubbleSize` (both `*Ref/c:f` and `*Lit` forms), `c:spPr` srgbClr or schemeClr through `colors.rs`, `c:trendline`, `c:errBars`, `c:dLbls`.
- Axes: `c:catAx`/`c:valAx`/`c:dateAx` scaling, gridlines and title. Also `c:legendPos` and the title text (rich text flattened).
- `Sheet1!$B$2:$B$9` references are translated with `formula.rs`.
- The same limits as ODS apply, and part paths are resolved only through the existing rels helpers.

### Tests
Engine, `crates/tally-engine/tests/charts.rs`:
- `add_chart_returns_id_and_undo_removes`, `move_resize_undo_redo`, `insert_rows_above_shifts_anchor_and_ranges`, `delete_rows_inside_series_shrinks_range`, `delete_whole_series_range_marks_invalid`, `delete_referenced_sheet_invalidates_series`
- `cut_paste_moves_series_ranges`, `edit_in_series_reports_charts_changed`, `edit_elsewhere_reports_nothing`, `recalc_dependent_reports_charts_changed`, `hidden_rows_excluded_unless_include_hidden`
- `trend_linear_matches_slope_intercept_rsq`, `trend_log_exp_power`, `error_bars_stdev_matches_stdev_fn`, `percent_error_bars`
- `suggest_chart_detects_region_and_labels`, `protected_sheet_refuses_chart_ops`, `limits_series_and_points_truncate`

ODS, `crates/tally-io-ods/tests/charts.rs` (round trip through the engine):
- One per type: `roundtrip_column`, `_column_stacked`, `_column_percent`, `_bar`, `_line_points_lines`, `_line_smooth`, `_area_stacked`, `_pie`, `_pie_exploded`, `_donut`, `_scatter_lines`, `_bubble`, `_radar`, `_radar_filled`, `_stock_hlc`, `_stock_ohlc`, `_stock_volume_ohlc`, `_combo_column_line`
- Features: `_secondary_axis`, `_trend_eq_r2`, `_error_bars_range`, `_axis_log_min_max`, `_legend_positions`, `_titles`, `_data_labels`, `_three_d_flag`, `_anchor_offsets_size`
- Fixtures and safety: `reads_aoo41_fixture` (`tests/fixtures/ods/charts-aoo41.ods`, made by the owner in the AOO GUI), `href_traversal_rejected`, `missing_object_skipped`, `too_many_charts_capped`

XLSX, `crates/tally-io-xlsx/tests/charts.rs`:
- Write→read round trip for every supported type
- `bubble_degrades_to_scatter_with_warning`, `stock_volume_dropped_with_warning`, `two_cell_anchor_emu_to_hmm`, `reads_excel_fixture`

Manual QA: open a written file in AOO 4.1 and check every type renders. If AOO shows an empty frame because `ObjectReplacements/` is missing, write a PNG replacement rendered by the UI (stretch).

---

## W5-2 DataPilot, outline and data tools: engine and IO (ENG)

### Pivot DTOs (`crates/tally-api/src/pivot.rs`)
```rust
pub enum AggFn { Sum, Count, Average, Max, Min, Product, CountNums, StDev, StDevP, Var, VarP }
pub enum PivotSource { Range { sheet: SheetId, range: RangeDto }, DatabaseRange { name: String } }   // tagged
pub struct PivotDto {
  pub id: u32, pub name: String,                       // "DataPilot1"
  pub source: PivotSource, pub target_sheet: SheetId, pub target: CellPos,
  pub output: Option<RangeDto>,                        // read-only: last written area
  pub row_fields: Vec<PivotFieldDto>, pub col_fields: Vec<PivotFieldDto>, pub page_fields: Vec<PivotFieldDto>,
  pub data_fields: Vec<PivotDataFieldDto>, pub data_layout: DataLayout,     // Row|Column ("Data" pseudo-field)
  pub row_totals: bool, pub col_totals: bool, pub ignore_empty_rows: bool, pub identify_categories: bool,
  pub show_filter_button: bool, pub drill_down: bool,
}
pub struct PivotFieldDto { pub source_col: u32, pub name: String, pub subtotals: Subtotals /*Auto|None|Custom{fns}*/,
  pub show_empty: bool, pub sort: PivotSort /*None|Name{ascending}|Data{dataField,ascending}|Manual{order}*/,
  pub hidden_items: Vec<String>, pub page_item: Option<String>, pub layout: PivotLayout /*Tabular|OutlineTop|OutlineBottom*/,
  pub empty_line_after: bool, pub auto_show: Option<AutoShow /*{top,count,dataField}*/>, pub group: Option<PivotGroupDto> }
pub struct PivotDataFieldDto { pub source_col: u32, pub function: AggFn, pub caption: Option<String>, pub show_as: Option<ShowAs> }
pub struct ShowAs { pub kind: ShowAsKind /*Difference|PercentOf|PercentDifference|RunningTotal|PercentOfRow|PercentOfColumn|PercentOfTotal|Index*/,
  pub base_field: Option<u32>, pub base_item: Option<BaseItem /*Named{name}|Previous|Next*/> }
pub enum PivotGroupDto { Numeric { start: Option<f64>, end: Option<f64>, step: f64 },
  Date { start: Option<f64>, end: Option<f64>, parts: Vec<DatePart /*Seconds|Minutes|Hours|Days{n}|Months|Quarters|Years*/> } }
pub struct PivotFieldItemsDto { pub source_col: u32, pub name: String, pub numeric: bool, pub date: bool, pub items: Vec<PivotItemDto /*{text,count}*/> }
pub struct PivotHit { pub pivot: u32, pub role: PivotCellRole /*Header|RowItem|ColItem|PageField|Data|Total*/, pub field: Option<u32>, pub item: Option<String> }
```
**Ops:**
- `CreatePivot { pivot, overwrite }` and `UpdatePivot { id, pivot, overwrite }`. If the target area is not empty outside the previous output, the op fails with `OpError code "target_not_empty"`; the UI then asks the AOO question "Destination range is not empty. Overwrite?".
- `RefreshPivot { id }` (AOO never refreshes automatically).
- `DeletePivot { id, clear_output }`
- `PivotDrillDown { id, r, c }` inserts a sheet holding the matching source rows.

**Queries:** `pivots(doc)`, `pivot_field_items(doc, source)` (the source can be scanned before the pivot exists), and `pivot_at(doc, sheet, r, c) -> Option<PivotHit>`.

**Model:** `WorkbookData.pivots: Vec<PivotData>` (workbook level, because source and target may be on different sheets). In the engine: `Workbook.pivots: pivot::PivotTable` and undo through `Change::Pivots(Box<PivotTable>)` plus a `Change::Range` of old∪new output. Output cells are plain values with direct styles: bold headers and a light fill on totals.

**`crates/tally-engine/src/pivot.rs`:**
- It reads the source (header row = field names), builds a cache of unique items per field, applies grouping, aggregates with the AOO semantics (Count = non-empty, CountNums = numeric; StDev/Var with n<2 → `#DIV/0!`, to verify), applies show-as, and lays the result out.
- Item order: ascending, numbers before text, case-insensitive (to verify against the AOO GUI).
- Tabular layout to match:
  - optional "Filter" button row;
  - one row per page field (`Field | - all -`), then a blank row;
  - corner cell with the data caption `Sum - Revenue`; the column-field name row; the row-field names next to the column items;
  - subtotal rows `<item> Result`; grand totals `Total Result`;
  - with more than one data field, a `Data` pseudo-field.
- The exact strings must be checked against AOO 4.1 by the owner (fixture `tests/fixtures/ods/pivot-aoo41.ods`).
- `adjust_structure` moves the source and target ranges.

**ODS (`crates/tally-io-ods/src/pivot.rs`):** write `table:data-pilot-tables` after `table:database-ranges` (schema order):
- `table:data-pilot-table` with `table:name`, `table:target-range-address`, `table:grand-total="both|row|column|none"`, `table:ignore-empty-rows`, `table:identify-categories`, `table:show-filter-button` and `table:drill-down-on-double-click`.
- `table:source-cell-range` with `table:cell-range-address`, or `table:database-source-table`.
- `table:data-pilot-field` with `table:source-field-name`, `table:orientation`, `table:function`, `table:selected-page`, and `table:is-data-layout-field` for the pseudo-field.
- `table:data-pilot-level table:show-empty` with `data-pilot-subtotals`, `data-pilot-members` (`table:display`), `data-pilot-display-info`, `data-pilot-sort-info`, `data-pilot-layout-info`.
- `table:data-pilot-field-reference` for show-as, and `table:data-pilot-groups` for grouping.

**XLSX:**
- `pivot_write.rs` adds, through the `write_v03::patches` pass: `xl/pivotCache/pivotCacheDefinition1.xml` with `saveData="0" refreshOnLoad="1"`, `cacheFields` with `sharedItems` for the axis fields, and `xl/pivotTables/pivotTable1.xml` (`location`, `pivotFields` with items, `rowFields`, `colFields`, `pageFields`, `dataFields`), plus the workbook `pivotCaches`, the rels and the content types. This is best effort.
- `pivot_read.rs` reads the definition only; the output cells are already in the sheet.

**Data-tool ops (`ops_v05.rs`, modules `crates/tally-engine/src/{stdfilter.rs,subtotals.rs,outline.rs,consolidate.rs,text_to_columns.rs,dedupe.rs,scenarios.rs}`):**
- `StandardFilter { sheet, range, conditions: Vec<StdCondition{connector: And|Or, col, op: StdFilterOp, value}>, options: StdFilterOptions{case_sensitive, regex, has_header, no_duplicates, copy_to: Option<SheetCell>, keep_criteria} }`
  - `StdFilterOp` covers all 16 AOO conditions: = < > <= >= <>, Largest, Smallest, LargestPct, SmallestPct, Contains, NotContains, BeginsWith, NotBeginsWith, EndsWith, NotEndsWith.
  - The filter is stored on the database range; ODS uses `table:filter-or`/`filter-and`.
- `AdvancedFilter { sheet, range, criteria: SheetRange, options }` (reuses `tally-functions/criteria.rs` D-function semantics)
- `ResetFilter { sheet }`
- `Subtotals { sheet, range, groups (≤3: {groupBy, columns, function}), options{page_breaks, case_sensitive, presort, ascending, include_formats} }` inserts rows holding `SUBTOTAL(n;…)` and the labels `X Result` and `Grand Total`, and builds an outline. `RemoveSubtotals { sheet, range }`.
- Outline: `GroupRows`/`UngroupRows`/`GroupCols`/`UngroupCols { sheet, start, end }`, `SetOutlineCollapsed { sheet, axis, start, end, collapsed }`, `AutoOutline`, `RemoveOutline`.
  - Model: `SheetData.row_groups`/`col_groups: Vec<OutlineGroupData{start,end,level,collapsed}>`, at most 8 levels.
  - ODS: `table:table-row-group`/`table:table-column-group table:display`. XLSX: `outlineLevel`/`collapsed`/`hidden`.
- `Consolidate { function, sources: Vec<SheetRange>, dest: SheetCell, by_row_labels, by_col_labels, link }`
- `TextToColumns { sheet, range, options: CsvImportOptionsDto }` (reuses the `tally-io-csv` tokenizer)
- `RemoveDuplicates { sheet, range, cols, has_header }`
- Scenarios: `AddScenario`, `UpdateScenario`, `ShowScenario { id }` (with copy-back), `DeleteScenario`. AOO's model is a hidden sheet after its base sheet: `SheetData.scenario: Option<ScenarioData{ranges, comment, border_color, display_border, copy_back, copy_styles, prevent_changes}>`; ODS `table:scenario`; XLSX `scenarios`.

**Pivot tests** use `tests/fixtures/pivot/sales.csv`: 24 rows with Region, Rep, Product, Quarter, Units, Revenue, modelled on the AOO help DataPilot example. The golden outputs are hand-computed until the AOO fixture exists.
- `pivot_sum_region_by_quarter`, `pivot_count_and_countnums`, `pivot_stdev_var_match_functions`, `pivot_two_row_fields_auto_subtotals`, `pivot_no_grand_totals`
- `pivot_page_field_filters`, `pivot_hidden_items`, `pivot_sort_desc_by_data`, `pivot_autoshow_top3`
- `pivot_show_as_{percent_of_column,difference_previous,running_total,index}`, `pivot_two_data_fields_data_pseudo_field`
- `pivot_target_not_empty_refused`, `pivot_refresh_after_source_edit`, `pivot_undo_restores_output_and_definition`, `pivot_insert_rows_moves_source_and_target`
- `pivot_group_months`, `pivot_group_numeric_step`, `pivot_drill_down_sheet`
- `ods_pivot_roundtrip`, `reads_aoo41_pivot_fixture`, `xlsx_pivot_definition_roundtrip`

**Data-tool tests:** `subtotals_insert_and_remove`, `subtotals_outline_levels`, `std_filter_or_conditions`, `std_filter_copy_to`, `advanced_filter_criteria_rows_or`, `consolidate_by_labels`, `text_to_columns_semicolon`, `remove_duplicates_keeps_first`, `outline_ods_xlsx_roundtrip`, `scenario_show_copy_back`.

---

## W5-3 Print, PDF and calculation semantics (ENG)

### Recommendation: write the PDF in Rust (krilla) and send Print through that PDF
Option A is a print-only HTML page plus `window.print()` / `Webview::print()`.
- It's fast to build and uses WebKit's text rendering.
- But it only works through the modal dialog: there is no silent export, so "Export PDF…" can't write a file.
- WKWebView's `printOperation` ignores CSS `@page size`, so the page geometry would be wrong whenever the user changes paper in the dialog.
- `Webview::print` only works on macOS. Output would differ across the three OSes, and it can't be unit-tested.

Option B is a hand-written PDF writer with embedded TrueType. A raw one would be expensive, but krilla (MIT/Apache, already in the local registry) handles subsetting (subsetter), shaping (rustybuzz) and SVG (krilla-svg/usvg). B is deterministic, testable and the same on every OS.

**Decision:** B is the only renderer. Export, Print and Preview all come from one cached print job:
- **Preview** rasterizes job pages to PNG with hayro.
- **Print** loads the job PDF in a small `WebviewWindow` through a custom `tally-print://` URI scheme and calls `Webview::print()`. This is macOS only. Spike this on day 1. Fallbacks: PDFKit `PDFDocument.printOperation` through objc2, or opening the file in Preview.
- **Windows and Linux** open the temporary PDF in the system viewer through Rust `std::process::Command`. Nothing is exposed to JS, which keeps the minimal capabilities in `capabilities/default.json`.

**New crate `crates/tally-io-pdf`** (ENG):
- `PrintJob { pages: Vec<PageJob{ size_pt, items: Vec<Item> }> }` with items `Fill{rect,color}`, `Line{from,to,width,color,style}`, `Text{rect, text, font: FontSpec{family,bold,italic,size_pt,color}, h_align, v_align, wrap, overflow: (f32,f32) /*free space left/right*/, clip}` and `Svg{rect, svg}`.
- `render_pdf(&PrintJob, &PdfOptions) -> Vec<u8>` and `rasterize(&[u8], page, dpi) -> png` (hayro).
- Fonts: vendored `fonts/Inter-{Regular,Bold,Italic,BoldItalic}.ttf` (Inter 4.x, OFL-1.1, SHA-256 recorded in NOTICE) through `include_bytes!`. Other families are looked up in system fonts with fontdb, falling back to Inter. Missing glyphs (CJK) fall back per character to system fonts.
- Text measurement and line breaking live here, because this crate holds the fonts.
- `cargo deny` must pass. If a transitive dependency needs a licence outside the allow list, flag it to the owner; don't widen the list.

**Pagination, `crates/tally-engine/src/print.rs`.** All sizes are in hmm, then converted to pt (1 mm = 72/25.4 pt).
- **Area:** the print range if set; otherwise the used area, meaning content plus visible formatting (fill, borders) plus chart bounds.
- **Body size:** paper minus margins, minus the header block (`header_mm`) when a header is set, minus the footer block likewise.
- **Scale:** `Percent` multiplies every size. For `FitTo{w,h}`, search downward from 100% to 10% for the largest integer percent with pages across ≤ w and pages down ≤ h (0 = unlimited). For the new `FitPages{pages}` (ODS `style:scale-to-pages`), the largest percent with total pages ≤ N. Whether AOO ignores manual breaks in fit modes needs verifying.
- **Breaks:** a page always holds at least one row or column. Hidden rows and columns are skipped. Manual breaks force a new page. Repeat rows and columns take space on every page after the first page that naturally contains them.
- **Page order:** top-to-bottom-then-right is AOO's default; left-to-right-then-down is the alternative.
- **Empty pages:** suppressed by default.
- **Placement:** centring offsets. `&[PAGE]` starts at `first_page_number`. `&[PAGES]`, `&[SHEET]`, `&[FILE]`, `&[PATH]`, `&[DATE]` and `&[TIME]` are resolved with locale formatting.
- **Job build:** cells are converted to items using `service/view.rs` display text and styles (CF styles included; data bars and icons are a stretch). Merges, borders, gridlines and headers are optional. Notes are printed "at end" when that is chosen. Charts use the SVG supplied by the UI.

**DTOs (`crates/tally-api/src/print.rs`) and the `PageSetupDto` additions:**
- New fields: `PageScale::FitPages{pages}`, `print_charts` (true), `print_notes: NotesPrint{None|AtEnd}`, `print_zero_values` (true), `print_formulas`. All `serde(default)`, with the model, ODS and XLSX updated.
```rust
pub struct PrintOptionsDto { pub sheets: PrintSheets /*Active|All|Some{ids}*/, pub selection: Option<RangeDto>,
  pub suppress_empty_pages: bool, pub page_from: Option<u32>, pub page_to: Option<u32> }
pub struct ChartSvgDto { pub sheet: SheetId, pub id: u32, pub svg: String }   // ≤ 4 MB each
pub struct PrintPageDto { pub index: u32, pub number: u32, pub sheet: SheetId, pub rows: [u32;2], pub cols: [u32;2],
  pub repeat_rows: Option<[u32;2]>, pub repeat_cols: Option<[u32;2]>, pub scale_pct: u32,
  pub paper_mm: [f32;2], pub origin_mm: [f32;2], pub header: Option<[String;3]>, pub footer: Option<[String;3]> }
pub struct PrintLayoutDto { pub pages: Vec<PrintPageDto>, pub warnings: Vec<String> }
pub struct PageBreaksDto { pub area: Option<RangeDto>, pub row_breaks: Vec<BreakInfo /*{at, manual}*/>, pub col_breaks: Vec<BreakInfo> }
pub struct PrintJobInfo { pub job: u32, pub layout: PrintLayoutDto }
```
**Service methods:**
- `print_layout(doc, opts)` covers the brief's "print_preview(doc, sheet)" metadata.
- `page_breaks(doc, sheet)` feeds Page Break Preview.
- `prepare_print_job(doc, opts, charts) -> PrintJobInfo`; the engine keeps an LRU of 4 jobs.
- `print_job_page_png(job, page, dpi) -> String` (base64) and `print_job_pdf(job) -> Vec<u8>`.
- Shell-only commands: `wb_export_pdf(doc, job)` (save dialog, then `write_atomically`) and `wb_print(doc, job)`.

**Calculation semantics (same ENG agent, `recalc.rs` and `eval.rs`):**
- **Iterations (AOO):** when `iterations` is set, each cyclic SCC is evaluated Gauss–Seidel in Tarjan order, starting from the previous cached value (0 for new cells). It repeats up to `steps` rounds until max |Δ| < `min_change`; a change that isn't numeric means not converged. Without convergence every member gets Err:523; with iterations off it stays Err:522.
- **Precision as shown:** numbers are rounded to their format's displayed decimals when read by formulas and when stored as results. General isn't rounded; percent adds 2 decimals. This needs a new helper `tally_numfmt::display_decimals(code)`.
- **Settings:** `Op::SetCalcSettings { settings: CalcSettingsDto }` (undoable through `Change::Settings`, followed by a hard recalc) and the query `calc_settings(doc)`. `CalcSettingsDto { iterations: Option<{steps,minChange}>, dateEpoch, caseSensitive, precisionAsShown, wholeCellMatch, criteriaMode, textToNumber }`.
- **Multiple operations:** `EvalContext::table_op(&mut self, formula: CellAddr, subs: &[(CellAddr, Value)]) -> Option<Value>`, with a default of `None` in `tally-functions/src/context.rs`. The trait change goes through W5-0a; FN implements the function in W5-4. The engine implements it with a generalized goal-seek sandbox that re-evaluates the precedent chain from the input cells to the formula. Capped at 10,000 calls per recalc.
- **Pivot data:** `EvalContext::pivot_data(&mut self, at: CellAddr, field: &str, filters: &[(String,String)]) -> Option<Value>`.

**Tests:**
- Pagination (`print::tests`), with A4 portrait, 20 mm margins, no header or footer, 4.52 mm rows and 22.58 mm columns: `a4_rows_per_page_is_56` (257/4.52), `a4_cols_per_page_is_7` (170/22.58), `repeat_rows_every_page`, `repeat_rows_not_duplicated_on_first_page`, `fit_to_1x1_finds_largest_scale`, `fit_width_only`, `fit_pages_total`, `manual_row_break_forces_page`, `hidden_rows_skipped`, `page_order_left_to_right`, `print_range_limits_area`, `empty_pages_suppressed`, `centering_offsets`, `header_placeholders_resolved`, `first_page_number_offsets`, `charts_extend_used_area`.
- PDF (`tally-io-pdf/tests`, hayro-syntax as a dev-dependency): `page_count_and_mediabox`, `fonts_subset_embedded`, `svg_chart_embedded`, `cjk_fallback_or_warning`, and the golden raster `invoices_page1.png` compared with a tolerance.
- Calculation: `iteration_self_ref_converges` (`A1: =A1/2+1` gives ≈2 within 0.001), `iteration_divergent_err523` (`=A1+1`), `iteration_off_err522`, `precision_as_shown_rounds_sum` (`0.333` shown as `0.33`, three copies summed give `0.99`), `set_calc_settings_undo`, `multiple_operations_one_and_two_variable`.

---

## W5-4 Functions hooks and metadata (FN)
- **GETPIVOTDATA** through `pivot_data`; tests in `tests/conformance/spreadsheet/GETPIVOTDATA.toml`, with a mock pivot in `mock.rs`.
- **MULTIPLE.OPERATIONS** through `table_op` (the current stub returns Err:504); 2-, 3- and 5-argument forms.
- **CURRENT()**: needs an `EvalContext::current_value()` hook (S).
- **STYLE()**: returns 0 and records `EvalContext::request_style(name, then: Option<(secs, name2)>)`. Depends on named styles in W5-6E.
- **Argument help:** a `ParamHelp` table generated from `docs/calc-functions.json` by `cargo xtask gen-function-help`, written to `crates/tally-functions/src/help_params.rs`. ENG exposes it as the additive `FunctionInfo.params: Vec<ParamInfoDto{name, description, optional, repeat}>`.
- **Localized function names:** `crates/tally-functions/src/l10n/{de,fr,es}.rs` (canonical name → localized name), generated by `cargo xtask gen-function-names --aoo <checkout>` from AOO's Apache-2.0 l10n (`extras/l10n/source/<lang>/localize.sdf`, entries for `formula/.../core_resource.src` and `scaddins/.../analysis_funcnames.src`). LibreOffice's MPL `.po` files must not be used. The owner provides the checkout.

---

## W5-5 UI: charts, print and PDF (UI agent A; owns `packages/grid`)

**Grid seams** (first PR, so W5-6 can build on them):
- `onViewportChange(vp)`
- `getGeometry(): { colLeft(c), rowTop(r), headerW, headerH, scrollX, scrollY, frozen, zoom }` in CSS px
- a `rangePick?: boolean` prop that turns on point mode without an open edit and reuses `onPointRange`
- `outline?: OutlineProps` (gutter with +/− and level buttons; events `onOutlineToggle` and `onOutlineLevel`)
- `pageBreaks?: PageBreaksDto` (Page Break Preview lines)
- `traces?: TraceArrow[]` (detective)
- an `aria` mirror, implemented in W5-6

**Chart files (`packages/app/src/charts/`):**
- `echarts.ts`: tree-shaken registration from `echarts/core` of BarChart, LineChart, PieChart, ScatterChart, RadarChart, CandlestickChart and CustomChart (error bars); TitleComponent, LegendComponent, GridComponent, TooltipComponent, MarkLineComponent and RadarComponent; CanvasRenderer and SVGRenderer. Pin `"echarts": "6.1.0"` exactly in `packages/app/package.json`.
- `toEcharts.ts`: a pure function `(ChartDto, ChartDataDto, theme) → EChartsOption`.
  - Bar charts swap the axes. Stacking uses `stack:"total"`; percent stacking is normalized here.
  - A points-only line chart is drawn as scatter on the category axis. Area is `line` with `areaStyle`.
  - Donut with several series becomes concentric rings, as in AOO. Exploded pie uses `selected` with an offset.
  - Bubble is scatter with `symbolSize ∝ sqrt(size)`, max 60 px.
  - Radar indicators come from the categories.
  - Stock: candlestick with ECharts' `[open, close, low, high]` order; HLC sets open = close; volume is a bar on `yAxisIndex` 1.
  - The trend line is an extra dashed `line` series with the equation and R² label.
  - Error bars are a `custom` series. The mean value is a `markLine`.
  - Axis labels are formatted through `numfmtLite.ts`. Dark-theme colours come from tokens.
- `ChartLayer.tsx`: an absolutely positioned layer over the grid's scroll area, placed from `getGeometry` and anchor mm (mm × 96/25.4 × zoom), clipped to the scrolling pane; charts don't sit in frozen panes (document this).
  - At most 20 live ECharts instances; charts are created when they enter the viewport and disposed when they leave it.
  - `charts_changed` and `objects_changed` are batched per animation frame, and each `chart_data` request carries the document version.
- `ChartFrame.tsx`: selection, 8 resize handles and drag-to-move. CSS-transform preview, then one `MoveResizeChart` on pointerup. Arrows nudge by 1 mm (Shift: 10 mm). Delete removes the chart, Esc deselects. ⌘C/⌘X/⌘V copy the `ChartDto` in memory and also write a PNG to the system clipboard; paste is `AddChart` at the active cell. Double-click opens the editor.
- `InsertChartPopover.tsx` (the type gallery) and `ChartEditorPanel.tsx`, a right-side panel with sections:
  - Type: kind, variant, stacking, 3D flag
  - Data: source range with a "pick" button using `rangePick`, series in rows/columns, label checkboxes, and a per-series list with name, values, categories, x, sizes, kind, axis and colour
  - Titles; Legend; Axes (min, max, major, log, reverse, grids, format); Labels; Statistics (trend, error bars, mean)

  Each edit is one `UpdateChart` transaction, labelled "Edit chart".
- `chartStore.ts`
- Commands: `insert.chart` is enabled (`suggest_chart`, then insert and select). New `chart.edit` and `chart.delete`. `chart.byColumn` (the assist action) creates a chart.
- `webMock.ts` needs chart emulation: store DTOs and compute `chart_data` from mock cells.

**Print UI (`packages/app/src/print/`):**
- `PrintPreview.tsx`: full-window; page thumbnails and the current page from `print_job_page_png`; zoom; page n/N; "Page setup…" opens the wave-4 dialog; Print and Export PDF buttons.
- `PageBreakPreview`: the `view.pageBreakPreview` grid mode with `pageBreaks` lines, dragging a break issues `SetPageBreak`, and a "Page N" watermark.
- `chartSvgs.ts` renders each chart with `echarts.init(null, null, {renderer:"svg", ssr:true, width, height})` and `renderToSVGString()`.
- Commands: `file.print` (⌘P), `file.printPreview`, `file.exportPdf` (in the Share popover and the File ▸ Export menu), `view.pageBreakPreview`.
- Desktop: the `tally-print` URI scheme, the print window and the jobs in `apps/desktop/src-tauri/src/print.rs`.

**Tests:**
- `packages/app/test/toEcharts.test.ts`: about 26 snapshots, one per kind×variant plus combo, trend, error bars, multi-ring donut, bubble and the 4 stock variants. Plus property tests: percent stacks sum to 100, bar swaps axes, palette order.
- `chartLayer.test.ts`: anchor mm → px at zoom 1.25 with scroll.
- `chartFrame.test.ts`: a drag emits exactly one op.
- `packages/grid/test/geometry.test.ts`
- Playwright `tests/e2e/charts.spec.ts`:
  1. Budget template, select A1:C6, Chart, Column.
  2. The chart appears and `window.__tallyCharts[id].getOption().series.length === 2`.
  3. Rename the title in the panel.
  4. Type in B2; the series value updates.
  5. Drag the chart; ⌘Z restores the anchor.
  6. Delete; ⌘Z restores the chart.
  7. Pick a series range with point mode.
- `tests/e2e/print.spec.ts`: Preview opens with N pages (mock layout).
- Rust command-layer tests in `apps/desktop/src-tauri/src/tests.rs`: `chart_roundtrip_ods_via_commands` and `export_pdf_job_bytes`.

---

## W5-6 UI: data tools, dialogs, panels, a11y, i18n (UI agent B)
Owns `packages/app/src/{dialogs/**,panels/**,i18n/**,a11y/**}`, new files only. It asks UI agent A for grid props and doesn't edit grid internals; `aria.ts` in grid is its one allowed file, landed through A's seam PR. The detailed list is in §4. In order:
- A: DataPilot dialog and pivot integration
- B: filters, subtotals, consolidate, text to columns, duplicates, scenarios, solver, multiple-operations dialogs
- C: Tools ▸ Options dialog, Function Wizard, AutoInput, AutoCorrect
- D: Navigator, Styles panel, AutoFormat, templates gallery, At a glance polish
- E: ARIA mirror and i18n

**DataPilot dialog (`dialogs/DataPilotDialog.tsx`):**
- Field list from `pivot_field_items`, drag and drop into Page / Column / Row / Data areas (keyboard alternative: a context menu "Move to…").
- Data field function picker with the 11 functions, a Field Options sub-dialog (subtotals, sort, show empty, hidden items, layout, AutoShow, show-as), and result options (target, totals, filter button, drill-down).
- When the op reports `target_not_empty`, show the overwrite confirm and retry with `overwrite`.
- Output cells carry the `data-pilot` style, so `pivot_at` drives the context menu (Refresh, Edit layout, Delete). A toolbar "Refresh" appears while the cursor is inside a pivot. Double-click a data cell issues `PivotDrillDown`.
- e2e `pivot.spec.ts` against the mock.

---

## 4. Remaining inventory items

These come from `docs/calc-features-inventory.md` and `docs/phase-a.md` versus HEAD. Sizes are S, M or L. "Test" is the one-line acceptance test. Where a WP column says W5-6A–E, the part letter matches the order in §W5-6.

**Charts, pivots, print and calculation semantics** (charts, pivots and print themselves are covered above)

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 1 | Iterative calculation (AOO semantics) | M | ENG | 0a | W5-3 | `iteration_self_ref_converges` |
| 2 | Precision as shown | M | ENG | numfmt helper | W5-3 | `precision_as_shown_rounds_sum` |
| 3 | Date epoch setting UI and Tools ▸ Options (Calculate, View, Input, Formula syntax, Sort lists, Print, Load/Save); app preferences in `apps/desktop` `prefs.json` | M | UI + ENG S | `SetCalcSettings` | W5-6C | changing to 1904 and back round-trips through ODS `table:null-date`; Options persists after restart |
| 4 | MULTIPLE.OPERATIONS hook and Data ▸ Multiple Operations dialog (writes formulas) | M | ENG + FN + UI S | `table_op` | W5-3/4/6B | 2-variable table equals hand values |
| 5 | GETPIVOTDATA, CURRENT | S | FN + ENG | pivots | W5-4 | conformance TOMLs pass |

**Filters, subtotals and data tools**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 6 | Standard filter (4 conditions, AND/OR, 16 ops, regex, no duplicates, copy to) and its dialog | M | ENG + UI | 0a | W5-2/6B | `std_filter_or_conditions`; e2e shows 3 of 10 rows |
| 7 | Advanced filter (criteria range) | S | ENG + UI S | #6 | W5-2/6B | `advanced_filter_criteria_rows_or` |
| 8 | Reset filter and Hide AutoFilter commands | S | UI | none | W5-6B | menu items toggle state |
| 9 | Subtotals dialog (3 groups, 11 functions, options, remove) | M | ENG M + UI S | #10 | W5-2/6B | `subtotals_insert_and_remove` |
| 10 | Group and outline rows and columns (F12 / ⌘F12), AutoOutline, gutter UI | L | ENG M + UI M | grid seam | W5-2/5/6B | `outline_ods_xlsx_roundtrip`; collapsing hides rows |
| 11 | Consolidate | M | ENG + UI S | none | W5-2/6B | `consolidate_by_labels` |
| 12 | Text to columns (CSV options dialog) | M | ENG + UI | io-csv | W5-2/6B | `text_to_columns_semicolon` |
| 13 | Remove duplicates (modern extra) | S | ENG + UI | none | W5-2/6B | `remove_duplicates_keeps_first` |
| 14 | Scenarios (hidden scenario sheets, Navigator switch) | L | ENG M + UI M | #31 | W5-2/6B | `scenario_show_copy_back`; ODS round trip |
| 15 | Solver (linear: own two-phase simplex + branch and bound, ~700 lines; no lp_solve, which is LGPL). `solver(doc, SolverRequest) -> SolverResult`, run in a sandbox like `goal_seek`; linearity checked by finite differences | L | ENG L + UI M | none | W5-3 (stretch) / 6B | the AOO help "max profit" example gives a known optimum; an integer constraint is respected |

**Editing assists**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 16 | AutoInput (column completion: case-insensitive, text cells in the same data block, first match in alphabetical order). `autocomplete(doc, sheet, r, c, prefix) -> Option<String>`; also the Ctrl+D selection list | S + S | ENG + UI | none | W5-2/6C | typing "Ap" under "Apple" offers "ple" |
| 17 | AutoCorrect (two initial capitals, capitalize sentence, dashes, ordinals, quotes, replace table from AOO `acor` lists) | M | UI | prefs | W5-6C | "TEst" → "Test"; "1st" kept; undo reverts |
| 18 | Spell check: skipped. Optional `spellcheck` attribute on the editor gives macOS underlines during editing only | S | UI | none | W5-6C | documented as deviation |
| 19 | Function Wizard (⌘F2): categories, Last Used (SUM AVERAGE MIN MAX IF), description, argument fields with point mode, live result through `eval_formula_text_at`, structure tree | M | UI + FN S + ENG S | `FunctionInfo.params` | W5-4/6C | building `SUMIF` via the wizard inserts the exact formula |

**File formats and links**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 20 | External links and DDE disabled by policy: DDE → #N/A, `'file:…'#$S.A1` → #REF! with a warning, linked sheets imported as values | S | ENG | none | W5-3 | `external_ref_disabled_warning` |
| 21 | SXC import (OOo 1.x namespaces mapped onto the ODS parser, `oooc:` formulas) | M | ENG | none | W5-3 | fixture opens with values and formulas |
| 22 | XLS import (calamine values and cached results, formulas as text where calamine allows; new crate `tally-io-xls`; size pre-checks plus fuzzing) | M | ENG | none | W5-3 | BIFF8 fixture values match; malicious corpus rejected |
| 23 | HTML import (tables) and HTML clipboard paste; own tolerant tokenizer | M | ENG + UI S | none | W5-3 | 2 tables become 2 ranges; `<script>` ignored |
| 24 | HTML export (`FileFormat::Html`, one table per sheet, inline styles, escaping) | S | ENG | none | W5-3 | export→import round trip of values |
| 25 | dBase .dbf import/export (code-page option); DIF/SYLK optional | M | ENG | none | W5-3 | `dbf_roundtrip_cp1252` |

**Styles and view**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 26 | Named cell styles (Default, Result, Result2, Heading, Heading1), parent chains, Styles panel (⌘F11), paint-can, new style from selection; ODS `office:styles` plus `style:parent-style-name`; XLSX `cellStyleXfs`. Ops `DefineCellStyle`, `DeleteCellStyle`, `ApplyCellStyle` | L | ENG M + UI M | none | W5-2 or W5-3 / 6D | applying Heading then editing the style updates every cell; ODS round trip keeps names |
| 27 | AutoFormat (AOO's 17 presets, options number/font/align/borders/pattern/autofit). `ApplyAutoFormat` op | M | ENG S + UI M | #26 | W5-6D | "Blue" on A1:D5 gives the expected header/body/total fills |
| 28 | Themes: app light/dark exists; add XLSX custom `theme1.xml` colours on import | S | ENG | none | W5-3 | custom-theme fixture colours |
| 29 | Full Format Cells dialog (⌘1): rotation, indent, shrink to fit, diagonal borders, double underline; `StylePatch` and model fields, ODS `style:rotation-angle`/`shrink-to-fit`/`diagonal-*`, grid renders rotation | M + M | ENG + UI | grid | W5-2/5/6D | rotation 45° round trips through ODS and XLSX |
| 30 | Split window (Window ▸ Split, independent panes; ODS `HorizontalSplitMode/Position`) | M | UI + ENG S | grid | W5-5 | split survives ODS round trip |
| 31 | Sheet extras: `CopySheet`, `SetSheetHidden` (Show Sheet dialog), `SetTabColor`, multi-sheet selection editing, insert sheet from file | M | ENG M + UI M | none | W5-2/6D | copied sheet's formulas point at the copy; hidden sheet keeps no tab |
| 32 | Insert/Delete Cells with shift (down/right, up/left) and partial-range reference adjustment | L | ENG | tally-formula adjust | W5-2 | `insert_cells_shift_down_adjusts_refs` |
| 33 | Delete Contents dialog (text, numbers, dates, formulas, formats, notes, objects) | S | ENG S + UI S | none | W5-6B | deleting numbers only keeps text |
| 34 | Detective: trace precedents/dependents/error, remove traces, mark invalid data. `precedents()` and `dependents()` queries; grid `traces` arrows | M | ENG S + UI M | grid | W5-3/5 | ⇧F7 on `=A1+B1` draws 2 arrows |
| 35 | View toggles: show formulas (⌃\`), value highlighting (⌘F8), zero values, gridlines, headers; persisted in ODS settings | S | UI + ENG S | none | W5-6D | ⌃\` shows formulas in cells |

**Fill, sort and names**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 36 | Fill Series dialog, Fill Up/Left, sort lists (custom lists) | S | UI + ENG S | none | W5-6B | "Q1" series via custom list |
| 37 | Sort options: left-to-right, copy results to, custom order, include formats; extend `SortRange` additively | S | ENG + UI | none | W5-2/6B | sorting columns left-to-right |
| 38 | Names extras: Insert Name list, Create Names from labels, Define Database Range | S | ENG + UI | none | W5-2/6B | creating names from the top row yields 3 names |

**Panels, templates and objects**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 39 | Navigator panel (F5): sheets, names, database ranges, charts, pivots, notes, scenarios. `navigator(doc)` query | M | UI + ENG S | none | W5-6D | double-clicking a chart selects it |
| 40 | At a glance polish (column picker, chart → real chart insert, empty states) | S | UI | charts | W5-6D | e2e |
| 41 | Templates gallery plus Save as Template (.ots, mimetype `…spreadsheet-template`) and a user templates folder | M | UI + ENG S | none | W5-6D | .ots opens as Untitled |
| 42 | Images (insert PNG/JPEG, anchored like charts; ODS `draw:image` in `Pictures/`, XLSX `insert_image`). Reuses the chart anchor | M | ENG + UI | W5-1 | W5-1 (stretch) / W5-5 | image round trips in ODS and XLSX |

**Platform, security and polish**

| # | Item | Size | Owner | Deps | WP | Test |
|---|---|---|---|---|---|---|
| 43 | Autosave and crash recovery (periodic ODS snapshot to app data, recovery prompt) | M | UI desktop + ENG S | none | W5-6D | killing the process mid-edit and relaunching offers recovery |
| 44 | Document statistics (sheets, cells, pages) | S | ENG + UI | print | W5-6D | numbers match |
| 45 | Password-protected ODS open/save (ODF 1.2 AES-256-CBC + PBKDF2; read AOO Blowfish/SHA1) with RustCrypto crates | L | ENG | none | W5-3 (stretch) | AOO-encrypted fixture opens |
| 46 | Accessibility ARIA mirror (`role=grid`, `aria-rowcount`/`colcount`, `aria-activedescendant`, live region with address + value + formula; dialogs focus-trapped) | M | UI | grid seam | W5-6E | VoiceOver script in `docs/qa/a11y.md`; Playwright accessibility-snapshot test |
| 47 | Localisation: function names de/fr/es (#FN data), locale separators in the formula dialect (`;` argument separator, `,` decimal), UI strings (`i18n/{en,de,fr,es}.json`, `t()`), native menu strings | L | ENG M (tally-formula) + FN S + UI L | AOO l10n checkout | W5-3/4/6E | de-DE: `=SUMME(A1;B1)` parses and prints; menus in German |
| 48 | Windows custom title bar (decorations off, caption buttons, drag region, the capability permissions) | S | UI desktop | none | W5-5 | manual on Windows |
| 49 | Keyboard shortcut parity. Gaps: ⌘⇧1–6 formats, ⌘1, ⌘F2, ⌃\`, F5, ⇧F5/⇧F7, F8/⇧F8, ⌘F8, ⌘F11, ⇧F11, F12/⌘F12, ⌘⇧J, ⌘⇧T, ⌘⌥C, ⌥↩ fill selection, ⌘* / ⌘/ select data/array, ⌘⇧Y repeat, ⌥ arrows resize, Delete → Delete Contents dialog. Deliberate deviation: ⌘D stays Fill Down (Excel and macOS usage); Ctrl+D is the selection list. Table in `docs/shortcuts.md` | M | UI | the commands above | W5-6C | table-driven `keymap.test.ts` against `docs/shortcuts.md` |
| 50 | Zoom presets (optimal, page width, whole page), full screen, Repeat last command | S | UI | print | W5-6D | e2e |
| 51 | Merge cells "move hidden contents?" prompt | S | UI + ENG S | none | W5-6B | the prompt moves text |

**Never or post-1.0** (documented in `docs/parity-deviations.md`): macros (banner only), form controls, track changes and shared workbooks, data source browser (F4), Euro converter, 3D chart rendering (the flag is kept), Lotus/Quattro/StarCalc 1–5/UOF/Pocket Excel/MiniCalc, XLS export, thesaurus, hyphenation.

**Signing, notarization and updater:** documentation only, in `docs/release.md`. It covers Developer ID Application cert + hardened runtime, `notarytool` keychain profile, stapling, `tauri-plugin-updater` (the 2.x version matching tauri 2.12) with a minisign key and a static `latest.json`, Windows Authenticode or Azure Trusted Signing, and Linux AppImage/GPG.

---

## 5. Exit criteria for 1.0

1. **Functions:** 407/407 registered; DDE and macro calls marked unsupported. `cargo xtask function-coverage --check` passes.
2. **Conformance ≥ 99% against an AOO oracle, without LibreOffice:**
   - Run AOO headless as a separate instance: `/Applications/OpenOffice.app/Contents/MacOS/soffice -headless -nologo -norestore -env:UserInstallation=file:///private/tmp/aoo-oracle "macro:///TallyOracle.Main.Run(\"<in.ods>\",\"<out.csv>\")"`. The headless hang comes from the request being handed to the running GUI instance through the shared user-profile pipe; a separate `UserInstallation` avoids that.
   - The Basic library is installed only in that oracle profile, by `xtask aoo-oracle --setup`. It loads the file, calls `calculateAll()` and writes every formula cell's value and error code as CSV.
   - AOO 4.1 has no "recalc on load" setting (I checked `share/registry/main.xcd`), which is why the explicit `calculateAll` is needed.
   - `cargo xtask aoo-oracle --run` turns the 5,003 conformance cases into ODS batches, runs the oracle and diffs the results. Deviations go in `docs/functions/deviations.md`.
   - Owner action: run it locally, since CI has no AOO.
3. **Fidelity corpus:** 200 real-world ODS/XLSX files (the owner sources them and clears licences). The tests open, save, reopen and compare a normalized model diff (`xtask fidelity`). Target: ≥98% lossless for supported features and 0 crashes.
4. **Performance (M-series):** 100k-formula full recalc < 500 ms; `SUM(A1:A1000000)` < 30 ms; 10k-chain edit < 50 ms; 60 fps scroll; open a 50 MB ODS < 10 s and < 1.5 GB RSS; `chart_data` 10k points < 20 ms; 20-page PDF < 1 s; 100k-row × 3-field pivot < 500 ms. Criterion benches gate on all three OSes. The owner provides Windows and Linux runners or machines.
5. **Accessibility:** VoiceOver walkthrough passes (`docs/qa/a11y.md`); every dialog works from the keyboard; Playwright accessibility-snapshot test. axe-core is MPL-2.0, so adding it even as an npm dev dependency is the owner's call.
6. **Security:** the malicious corpus is extended with chart, pivot, XLS and HTML cases; fuzz targets for the ODS, XLSX and XLS readers run 24 h without crashes; security review round 3.
7. **Parity docs:** the inventory checklist is marked done or deviation for every row; `docs/shortcuts.md`; `docs/parity-deviations.md`.
8. **Only the owner can:**
   - provide the Apple Developer ID cert and notarization credentials, a Windows signing cert or Azure Trusted Signing, and custody of the updater key plus hosting;
   - set up GitHub Actions secrets and macOS/Windows runners, including x86_64 macOS through rustup (this machine has only aarch64 std);
   - make the App Store decision (MAS sandbox entitlements);
   - create the AOO GUI fixtures (charts, pivots, scenarios, encrypted file);
   - download the Inter TTFs and an AOO l10n checkout;
   - collect the corpus;
   - optionally install LibreOffice for cross-checks.

### Critical files for implementation
- /Volumes/FILES/Projects/OSX_Office/crates/tally-api/src/dto.rs (`Op` enum and `ApplyResult` additions; the W5-0a hotspot)
- /Volumes/FILES/Projects/OSX_Office/crates/tally-engine/src/ops_v03.rs (`with_meta` and dispatch pattern to copy for `ops_v04.rs` and `ops_v05.rs`)
- /Volumes/FILES/Projects/OSX_Office/crates/tally-engine/src/structure.rs (`adjust_meta_for_structure`, lines 258–340: hooks for charts, outline and pivots)
- /Volumes/FILES/Projects/OSX_Office/crates/tally-io-ods/src/write.rs (manifest, content order, cell writer: frames, data-pilot tables, row groups) together with `src/content.rs` (read dispatch)
- /Volumes/FILES/Projects/OSX_Office/crates/tally-io-xlsx/src/write.rs and `src/write_v03.rs` (rust_xlsxwriter charts plus the patches pass for pivots and bubble)
- /Volumes/FILES/Projects/OSX_Office/crates/tally-engine/src/recalc.rs (SCC handling for iterations; `CalcCtx` hooks for `table_op` and `pivot_data`)
- /Volumes/FILES/Projects/OSX_Office/packages/grid/src/grid.ts and `packages/grid/src/types.ts` (viewport, geometry, range-pick, outline, page-break and ARIA seams)
- /Volumes/FILES/Projects/OSX_Office/packages/app/src/commands.ts and `apps/desktop/src-tauri/src/menu.rs` (command and menu ids; must stay in sync with `menuIds.test.ts`)
