//! [`EngineService`]: the engine's implementation of
//! [`tally_api::DocumentService`].
//!
//! Wiring in the shell:
//!
//! ```no_run
//! use tally_engine::EngineService;
//! // ODS/CSV via the IO crates when tally-engine's `io` feature is on,
//! // otherwise opening/saving reports `unsupported`:
//! let svc = EngineService::with_default_codec();
//! // The ⌘K provider is installed separately (see `set_assist_provider`).
//! # let _ = svc;
//! ```

pub mod assist;
pub mod codec;
pub mod templates;
pub mod view;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use tally_api::{
    ApiError, ApplyResult, CellBlock, CellEditInfo, CellPos, ClipboardOut, Direction, DocId,
    DocumentService, FileFormat, FunctionInfo, RangeDto, Result, SelectionDto, SelectionStats,
    SheetLayout, StyleDto, Suggestion, Transaction, WorkbookInfo,
};
use tally_functions::registry;
use tally_types::{CellAddr, CellRange, Value};

pub use assist::{AssistProvider, AssistRequest, SheetView};
#[cfg(feature = "io")]
pub use codec::IoCodec;
pub use codec::{Decoded, Export, FileCodec, UnsupportedCodec};

use crate::clipboard::{copy_range, ClipSnapshot};
use crate::document::Document;
use crate::ops::to_range;
use crate::recalc::RecalcBudget;
use crate::storage::{ColumnStore, StyleId};
use crate::workbook::{Rng, Workbook};

/// Largest file `open` reads (512 MiB).
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Most fields (cells plus empty fillers) one CSV export writes.
pub const MAX_CSV_EXPORT_CELLS: u64 = 10_000_000;

/// The engine behind the `DocumentService` facade.
pub struct EngineService {
    docs: BTreeMap<u32, Document>,
    next_doc: u32,
    clipboard: Option<ClipSnapshot>,
    codec: Box<dyn FileCodec>,
    assist: Option<AssistProvider>,
    rng: Rng,
    open_max_evaluated: u64,
    open_time_limit: Option<Duration>,
}

impl Default for EngineService {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EngineService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineService")
            .field("docs", &self.docs.len())
            .field("next_doc", &self.next_doc)
            .finish_non_exhaustive()
    }
}

fn not_found_doc(doc: DocId) -> ApiError {
    ApiError::not_found(format!("no document {}", doc.0))
}

impl EngineService {
    /// A service without file support ([`UnsupportedCodec`]).
    pub fn new() -> Self {
        Self {
            docs: BTreeMap::new(),
            next_doc: 1,
            clipboard: None,
            codec: Box::new(UnsupportedCodec),
            assist: None,
            rng: Rng::from_time(),
            open_max_evaluated: RecalcBudget::OPEN_MAX_CELLS,
            open_time_limit: Some(RecalcBudget::OPEN_TIME_LIMIT),
        }
    }

    /// Sets the recalc watchdog used by [`DocumentService::open`]: at most
    /// `max_evaluated` formulas within `time_limit` (defaults: 50M, 20 s).
    pub fn set_open_recalc_limits(&mut self, max_evaluated: u64, time_limit: Option<Duration>) {
        self.open_max_evaluated = max_evaluated;
        self.open_time_limit = time_limit;
    }

    /// A service with the IO crates' codec when the `io` feature is on.
    pub fn with_default_codec() -> Self {
        #[cfg(feature = "io")]
        {
            Self::new().with_codec(Box::new(IoCodec))
        }
        #[cfg(not(feature = "io"))]
        {
            Self::new()
        }
    }

    /// Replaces the file codec.
    pub fn with_codec(mut self, codec: Box<dyn FileCodec>) -> Self {
        self.codec = codec;
        self
    }

    /// Installs the ⌘K provider used by `assist_suggest`.
    pub fn set_assist_provider(&mut self, provider: AssistProvider) {
        self.assist = Some(provider);
    }

    /// Opens a workbook built elsewhere (tests, the CLI).
    pub fn open_workbook(&mut self, wb: Workbook, title: &str) -> WorkbookInfo {
        let id = DocId(self.next_doc);
        self.next_doc += 1;
        let doc = Document::new(id, wb, title);
        let info = doc.info();
        self.docs.insert(id.0, doc);
        info
    }

    /// A document.
    pub fn document(&self, doc: DocId) -> Result<&Document> {
        self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))
    }

    /// A document, mutably.
    pub fn document_mut(&mut self, doc: DocId) -> Result<&mut Document> {
        self.docs.get_mut(&doc.0).ok_or_else(|| not_found_doc(doc))
    }

    /// The active sheet as CSV rows, built from sparse storage.
    ///
    /// Rows are padded to the used width when the whole rectangle fits in
    /// [`MAX_CSV_EXPORT_CELLS`] (so ordinary sheets give a rectangular
    /// file). Otherwise each row stops at its last cell and empty rows are
    /// bare line breaks, so values at A1 and XFD1048576 cost two short rows
    /// and a million line breaks, not 17 G fields. Exports that still exceed
    /// the cap are refused.
    fn csv_rows(wb: &Workbook) -> Result<Vec<Vec<(String, bool)>>> {
        let too_large = || ApiError::unsupported("the sheet is too large to export as CSV");
        let sheet_id = wb.active_sheet;
        let Some(sheet) = wb.sheet(sheet_id) else {
            return Ok(Vec::new());
        };
        let Some(used) = sheet.used_range() else {
            return Ok(Vec::new());
        };
        let mut present: Vec<(u32, u32)> = Vec::new();
        for (ci, col) in sheet.columns.iter().enumerate() {
            for (r, _) in col.cells.iter_range(0, tally_types::MAX_ROWS - 1) {
                present.push((r, ci as u32));
                if present.len() as u64 > MAX_CSV_EXPORT_CELLS {
                    return Err(too_large());
                }
            }
        }
        present.sort_unstable();
        let rows = u64::from(used.r1) + 1;
        let width = u64::from(used.c1) + 1;
        let dense = rows.saturating_mul(width) <= MAX_CSV_EXPORT_CELLS;
        if !dense {
            let mut total = rows; // one line per row
            let mut i = 0;
            while i < present.len() {
                let r = present[i].0;
                while i + 1 < present.len() && present[i + 1].0 == r {
                    i += 1;
                }
                total = total.saturating_add(u64::from(present[i].1));
                i += 1;
            }
            if total > MAX_CSV_EXPORT_CELLS {
                return Err(too_large());
            }
        }
        let mut out: Vec<Vec<(String, bool)>> = Vec::with_capacity(rows as usize);
        let mut next = present.iter().peekable();
        for r in 0..=used.r1 {
            let mut row: Vec<(String, bool)> = Vec::new();
            while let Some(&&(pr, c)) = next.peek() {
                if pr != r {
                    break;
                }
                next.next();
                row.resize(c as usize, (String::new(), false));
                let addr = CellAddr::new(sheet_id, r, c);
                let is_text = matches!(wb.value(addr), Value::Text(_));
                row.push((view::display_text(wb, addr), is_text));
            }
            if dense {
                row.resize(width as usize, (String::new(), false));
            }
            out.push(row);
        }
        Ok(out)
    }
}

/// Writes `bytes` to `target` atomically: into a temporary file in the same
/// directory, flushed to disk, then renamed over `target`. A crash or a full
/// disk leaves the original file intact, and a symlink at `target` is
/// replaced rather than written through. The original's permissions are
/// kept.
pub fn write_atomically(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let name = target
        .file_name()
        .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let tmp = dir.join(format!(".{name}.tally-{}-{nanos}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = std::fs::metadata(target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, target)?;
        // Persist the rename itself (best effort; not all platforms allow
        // opening a directory).
        if let Ok(d) = std::fs::File::open(&dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn file_title(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Untitled".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl DocumentService for EngineService {
    fn new_workbook(&mut self, template: Option<&str>) -> Result<WorkbookInfo> {
        let (wb, title) = match template {
            None => (Workbook::new(), "Untitled"),
            Some(t) => templates::build(t)
                .ok_or_else(|| ApiError::not_found(format!("no template {t:?}")))?,
        };
        Ok(self.open_workbook(wb, title))
    }

    fn open(&mut self, path: &Path) -> Result<WorkbookInfo> {
        let meta =
            std::fs::metadata(path).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(ApiError::new(ApiError::BAD_FILE, "the file is too large"));
        }
        let bytes = std::fs::read(path).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let mut decoded = self.codec.read(&bytes, &ext)?;
        if decoded.data.sheets.len() > crate::workbook::MAX_SHEETS {
            return Err(ApiError::new(
                ApiError::BAD_FILE,
                format!(
                    "the file has more than {} sheets",
                    crate::workbook::MAX_SHEETS
                ),
            ));
        }
        // Only the first unnamed sheet (a CSV's only sheet) takes the file
        // name; other unnamed sheets get `SheetN` from `from_data`.
        if let Some(s) = decoded
            .data
            .sheets
            .iter_mut()
            .find(|s| s.name.trim().is_empty())
        {
            s.name = path
                .file_stem()
                .map_or_else(|| "Sheet1".to_owned(), |s| s.to_string_lossy().into_owned());
        }
        let wb = Workbook::from_data_opts(
            &decoded.data,
            decoded.recognize_text,
            decoded.evaluate_formulas,
        );
        let id = DocId(self.next_doc);
        self.next_doc += 1;
        let budget = RecalcBudget::within(self.open_max_evaluated, self.open_time_limit);
        let mut doc = Document::with_budget(id, wb, &file_title(path), &budget);
        doc.path = Some(path.to_path_buf());
        let info = doc.info();
        // Warnings are delivered once, with the open result.
        doc.warnings.clear();
        self.docs.insert(id.0, doc);
        Ok(info)
    }

    fn save(
        &mut self,
        doc: DocId,
        path: Option<&Path>,
        format: FileFormat,
    ) -> Result<WorkbookInfo> {
        if format == FileFormat::Xlsx {
            return Err(ApiError::unsupported("XLSX is not supported in Phase A"));
        }
        let d = self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))?;
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => d
                .path
                .clone()
                .ok_or_else(|| ApiError::invalid_argument("the document has no file path yet"))?,
        };
        let mut data = d.wb.to_data();
        data.meta.generator = Some(format!("Tally/{}", env!("CARGO_PKG_VERSION")));
        let csv_rows = if format == FileFormat::Csv {
            Some(Self::csv_rows(&d.wb)?)
        } else {
            None
        };
        let bytes = self.codec.write(
            &Export {
                data: &data,
                csv_rows,
            },
            format,
        )?;
        write_atomically(&target, &bytes)
            .map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
        let d = self
            .docs
            .get_mut(&doc.0)
            .ok_or_else(|| not_found_doc(doc))?;
        d.title = file_title(&target);
        d.path = Some(target);
        d.dirty = false;
        Ok(d.info())
    }

    fn close(&mut self, doc: DocId) -> Result<()> {
        self.docs
            .remove(&doc.0)
            .map(|_| ())
            .ok_or_else(|| not_found_doc(doc))
    }

    fn sheet_layout(&self, doc: DocId, sheet: tally_types::SheetId) -> Result<SheetLayout> {
        view::sheet_layout(&self.document(doc)?.wb, sheet)
    }

    fn cells(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        ranges: &[RangeDto],
    ) -> Result<Vec<CellBlock>> {
        let d = self.document(doc)?;
        view::cells(&d.wb, d.version, sheet, ranges)
    }

    fn styles(&self, doc: DocId, ids: &[u32]) -> Result<Vec<StyleDto>> {
        let d = self.document(doc)?;
        ids.iter()
            .map(|id| {
                if d.wb.styles.contains(*id) {
                    Ok(d.wb.styles.to_dto(StyleId(*id)))
                } else {
                    Err(ApiError::not_found(format!("no style {id}")))
                }
            })
            .collect()
    }

    fn cell_edit_info(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<CellEditInfo> {
        view::cell_edit_info(&self.document(doc)?.wb, sheet, r, c)
    }

    fn apply(&mut self, doc: DocId, tx: Transaction) -> Result<ApplyResult> {
        let clip = self.clipboard.as_ref();
        let d = self
            .docs
            .get_mut(&doc.0)
            .ok_or_else(|| not_found_doc(doc))?;
        Ok(d.apply(tx, clip))
    }

    fn undo(&mut self, doc: DocId) -> Result<ApplyResult> {
        Ok(self.document_mut(doc)?.undo())
    }

    fn redo(&mut self, doc: DocId) -> Result<ApplyResult> {
        Ok(self.document_mut(doc)?.redo())
    }

    fn selection_stats(&self, doc: DocId, sel: &SelectionDto) -> Result<SelectionStats> {
        view::selection_stats(&self.document(doc)?.wb, sel)
    }

    fn copy(&mut self, doc: DocId, sel: &SelectionDto) -> Result<ClipboardOut> {
        let d = self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))?;
        let dto = sel.ranges.first().copied().unwrap_or(RangeDto {
            r0: sel.active.r,
            c0: sel.active.c,
            r1: sel.active.r,
            c1: sel.active.c,
        });
        if d.wb.sheet(sel.sheet).is_none() {
            return Err(ApiError::not_found(format!("no sheet {}", sel.sheet)));
        }
        let range: CellRange = to_range(sel.sheet, &dto)?;
        if range.cell_count() > crate::ops::MAX_OP_CELLS {
            return Err(ApiError::invalid_argument(
                "the selection is too large to copy",
            ));
        }
        let (snap, tsv, html) = copy_range(&d.wb, range, &mut self.rng);
        let token = snap.token.clone();
        self.clipboard = Some(snap);
        Ok(ClipboardOut { tsv, html, token })
    }

    fn navigate_edge(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        from: CellPos,
        dir: Direction,
    ) -> Result<CellPos> {
        view::navigate_edge(&self.document(doc)?.wb, sheet, from, dir)
    }

    fn resolve_name_box(&self, doc: DocId, text: &str) -> Result<SelectionDto> {
        view::resolve_name_box(&self.document(doc)?.wb, text)
    }

    fn list_functions(&self) -> Vec<FunctionInfo> {
        let mut out: Vec<FunctionInfo> = registry()
            .iter()
            .map(|(_, d)| FunctionInfo {
                name: d.name.to_owned(),
                category: d.category.aoo_name().to_owned(),
                signature: d.signature().replace("; ", ", "),
                description: d.help.to_owned(),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    fn assist_suggest(
        &self,
        doc: DocId,
        query: &str,
        sel: &SelectionDto,
    ) -> Result<Vec<Suggestion>> {
        let d = self.document(doc)?;
        if d.wb.sheet(sel.sheet).is_none() {
            return Err(ApiError::not_found(format!("no sheet {}", sel.sheet)));
        }
        let Some(p) = &self.assist else {
            return Ok(Vec::new());
        };
        let req = AssistRequest {
            doc,
            query,
            selection: sel,
            sheet: SheetView::new(&d.wb, sel.sheet),
        };
        Ok(p(&req))
    }
}
