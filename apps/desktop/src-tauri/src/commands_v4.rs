//! Wave-5 commands (W5-0b): one thin `#[tauri::command]` per v0.4–v0.6
//! `DocumentService` query (docs/api.md, "v0.4–v0.6 (wave 5)"), plus the
//! shell-only `wb_export_pdf` and `wb_print`. Names match `COMMANDS` in
//! `packages/bridge/src/tauri.ts` exactly (a vitest checks the two lists).
//!
//! Every command validates its arguments with [`limits`] first, then runs the
//! engine call on a blocking worker under `catch_unwind`
//! ([`commands::blocking`], [`commands::with_doc`]). Until W5-1..W5-3
//! land, the engine answers `unsupported` for all of them; the shell layer
//! is complete so the UI packages can be wired against it now.

use tally_api::{
    AutocompleteDto, CalcSettingsDto, CellBlock, CellPos, CellStyleDto, CellsOptions, ChartDataDto,
    ChartDto, ChartKind, ChartSvgDto, DatabaseRangeDto, DocId, DocumentStatsDto, ImageDto,
    NavigatorDto, PageBreaksDto, PivotDto, PivotFieldItemsDto, PivotHit, PivotSource, PrintJobInfo,
    PrintLayoutDto, PrintOptionsDto, RangeDto, ScenarioDto, SolverRequest, SolverResult, TraceDto,
    ViewOptionsDto,
};
use tally_types::SheetId;
use tauri::AppHandle;

use crate::commands::{blocking, with_doc, with_engine, CmdResult};
use crate::limits;

// ----- v0.4 charts and pictures (W5-1) ---------------------------------------

#[tauri::command]
pub async fn charts(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<Vec<ChartDto>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.charts(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn chart_data(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    id: u32,
) -> CmdResult<ChartDataDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.chart_data(doc, sheet, id))
    })
    .await
}

#[tauri::command]
pub async fn suggest_chart(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    sel: RangeDto,
    kind: ChartKind,
) -> CmdResult<ChartDto> {
    limits::range("suggest_chart", &sel)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.suggest_chart(doc, sheet, sel, kind))
    })
    .await
}

#[tauri::command]
pub async fn images(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<Vec<ImageDto>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.images(doc, sheet))
    })
    .await
}

// ----- v0.5 DataPilot, data tools, styles, scenarios, navigator (W5-2) -------

#[tauri::command]
pub async fn pivots(app: AppHandle, doc: DocId) -> CmdResult<Vec<PivotDto>> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.pivots(doc))).await
}

#[tauri::command]
pub async fn pivot_field_items(
    app: AppHandle,
    doc: DocId,
    source: PivotSource,
) -> CmdResult<Vec<PivotFieldItemsDto>> {
    limits::pivot_source("pivot_field_items", &source)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.pivot_field_items(doc, &source))
    })
    .await
}

#[tauri::command]
pub async fn pivot_at(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<Option<PivotHit>> {
    limits::pos("pivot_at", r, c)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.pivot_at(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn autocomplete(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
    prefix: String,
) -> CmdResult<AutocompleteDto> {
    limits::pos("autocomplete", r, c)?;
    limits::chars_max("the prefix", &prefix, limits::MAX_AUTOCOMPLETE_CHARS)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.autocomplete(doc, sheet, r, c, &prefix))
    })
    .await
}

#[tauri::command]
pub async fn cell_styles(app: AppHandle, doc: DocId) -> CmdResult<Vec<CellStyleDto>> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.cell_styles(doc))).await
}

#[tauri::command]
pub async fn scenarios(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<Vec<ScenarioDto>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.scenarios(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn database_ranges(app: AppHandle, doc: DocId) -> CmdResult<Vec<DatabaseRangeDto>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.database_ranges(doc))
    })
    .await
}

#[tauri::command]
pub async fn navigator(app: AppHandle, doc: DocId) -> CmdResult<NavigatorDto> {
    blocking(app, move |_, st| with_doc(st, doc, |s| s.navigator(doc))).await
}

#[tauri::command]
pub async fn document_statistics(app: AppHandle, doc: DocId) -> CmdResult<DocumentStatsDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.document_statistics(doc))
    })
    .await
}

// ----- v0.6 print, PDF, calculation settings, solver, detective (W5-3) -------

#[tauri::command]
pub async fn print_layout(
    app: AppHandle,
    doc: DocId,
    opts: PrintOptionsDto,
) -> CmdResult<PrintLayoutDto> {
    limits::print_options(&opts)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.print_layout(doc, &opts))
    })
    .await
}

#[tauri::command]
pub async fn page_breaks(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<PageBreaksDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.page_breaks(doc, sheet))
    })
    .await
}

#[tauri::command]
pub async fn prepare_print_job(
    app: AppHandle,
    doc: DocId,
    opts: PrintOptionsDto,
    charts: Vec<ChartSvgDto>,
) -> CmdResult<PrintJobInfo> {
    limits::print_options(&opts)?;
    limits::chart_svgs(&charts)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.prepare_print_job(doc, &opts, &charts))
    })
    .await
}

#[tauri::command]
pub async fn print_job_page_png(
    app: AppHandle,
    job: u32,
    page: u32,
    dpi: u32,
) -> CmdResult<String> {
    limits::print_page("print_job_page_png", page)?;
    limits::dpi(dpi)?;
    blocking(app, move |_, st| {
        with_engine(st, |s| s.print_job_page_png(job, page, dpi))
    })
    .await
}

#[tauri::command]
pub async fn print_job_pdf(app: AppHandle, job: u32) -> CmdResult<Vec<u8>> {
    blocking(app, move |_, st| with_engine(st, |s| s.print_job_pdf(job))).await
}

#[tauri::command]
pub async fn calc_settings(app: AppHandle, doc: DocId) -> CmdResult<CalcSettingsDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.calc_settings(doc))
    })
    .await
}

#[tauri::command]
pub async fn solver(app: AppHandle, doc: DocId, req: SolverRequest) -> CmdResult<SolverResult> {
    limits::solver_request(&req)?;
    blocking(app, move |_, st| with_doc(st, doc, |s| s.solver(doc, &req))).await
}

#[tauri::command]
pub async fn precedents(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<Vec<TraceDto>> {
    limits::pos("precedents", r, c)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.precedents(doc, sheet, r, c))
    })
    .await
}

#[tauri::command]
pub async fn dependents(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
) -> CmdResult<Vec<TraceDto>> {
    limits::pos("dependents", r, c)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.dependents(doc, sheet, r, c))
    })
    .await
}

// ----- integration pass: queries the UI needs from the real engine ------------

/// `image_data(doc, sheet, id)`: a picture's bytes, base64 (≤ 16 MiB decoded).
#[tauri::command]
pub async fn image_data(app: AppHandle, doc: DocId, sheet: SheetId, id: u32) -> CmdResult<String> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.image_data(doc, sheet, id))
    })
    .await
}

/// `view_options(doc, sheet)`: the toggles and the persisted split.
#[tauri::command]
pub async fn view_options(app: AppHandle, doc: DocId, sheet: SheetId) -> CmdResult<ViewOptionsDto> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.view_options(doc, sheet))
    })
    .await
}

/// `invalid_cells(doc, sheet, range)`: Mark Invalid Data.
#[tauri::command]
pub async fn invalid_cells(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    range: Option<RangeDto>,
) -> CmdResult<Vec<CellPos>> {
    if let Some(r) = &range {
        limits::range("invalid_cells", r)?;
    }
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.invalid_cells(doc, sheet, range))
    })
    .await
}

/// `custom_sort_lists(doc)`: Tools ▸ Options ▸ Sort Lists.
#[tauri::command]
pub async fn custom_sort_lists(app: AppHandle, doc: DocId) -> CmdResult<Vec<Vec<String>>> {
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.custom_sort_lists(doc))
    })
    .await
}

/// `cells_with(doc, sheet, ranges, opts)`: `get_cells` with View options.
#[tauri::command]
pub async fn cells_with(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    ranges: Vec<RangeDto>,
    opts: CellsOptions,
) -> CmdResult<Vec<CellBlock>> {
    limits::ranges("cells_with", &ranges)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.cells_with(doc, sheet, &ranges, &opts))
    })
    .await
}

/// `preview_formula(doc, sheet, r, c, text)`: the Function Wizard's result.
#[tauri::command]
pub async fn preview_formula(
    app: AppHandle,
    doc: DocId,
    sheet: SheetId,
    r: u32,
    c: u32,
    text: String,
) -> CmdResult<Option<String>> {
    limits::pos("preview_formula", r, c)?;
    limits::text("preview_formula", &text)?;
    blocking(app, move |_, st| {
        with_doc(st, doc, |s| s.preview_formula(doc, sheet, r, c, &text))
    })
    .await
}

// ----- shell-only: PDF export and printing (W5-5, `print.rs`) ----------------

/// `wb_export_pdf(doc, job)`: shows a save dialog and writes the job's PDF
/// (W5-5: `print::export_pdf`; the engine's `print_job_pdf` answers
/// `unsupported` until W5-3 lands).
#[tauri::command]
pub async fn wb_export_pdf(
    app: AppHandle,
    state: tauri::State<'_, crate::AppState>,
    doc: DocId,
    job: u32,
) -> CmdResult<()> {
    crate::print::export_pdf(app, state, doc, job).await
}

/// `wb_print(doc, job)`: sends the job's PDF to the OS print dialog
/// (W5-5: `print::print`); a macOS print window belongs to the calling
/// document (tab): closing it closes the print window too.
#[tauri::command]
pub async fn wb_print(
    app: AppHandle,
    webview: tauri::Webview,
    doc: DocId,
    job: u32,
) -> CmdResult<()> {
    crate::print::print(app, webview.label().to_owned(), doc, job).await
}

/// Every wave-5 command name, in `generate_handler!` order (checked against
/// `lib.rs` here and against `TauriClient` by `tauriClientV4.test.ts`).
#[cfg(test)]
pub const COMMAND_NAMES: &[&str] = &[
    "charts",
    "chart_data",
    "suggest_chart",
    "images",
    "pivots",
    "pivot_field_items",
    "pivot_at",
    "autocomplete",
    "cell_styles",
    "scenarios",
    "database_ranges",
    "navigator",
    "document_statistics",
    "print_layout",
    "page_breaks",
    "prepare_print_job",
    "print_job_page_png",
    "print_job_pdf",
    "calc_settings",
    "solver",
    "precedents",
    "dependents",
    "image_data",
    "view_options",
    "invalid_cells",
    "custom_sort_lists",
    "cells_with",
    "preview_formula",
    "wb_export_pdf",
    "wb_print",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_names_are_unique_snake_case() {
        let mut names = COMMAND_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), COMMAND_NAMES.len());
        assert!(COMMAND_NAMES
            .iter()
            .all(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_')));
    }

    /// `lib.rs` registers every name in `COMMAND_NAMES` (the source is
    /// checked, since `generate_handler!` has no introspection).
    #[test]
    fn every_command_is_registered_in_lib_rs() {
        let lib = include_str!("lib.rs");
        for name in COMMAND_NAMES {
            assert!(
                lib.contains(&format!("commands_v4::{name},")),
                "{name} is not in generate_handler!"
            );
        }
    }
}
