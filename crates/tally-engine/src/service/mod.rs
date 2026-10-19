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
use crate::storage::StyleId;
use crate::workbook::{Rng, Workbook};

/// Largest file `open` reads (512 MiB).
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// The engine behind the `DocumentService` facade.
pub struct EngineService {
    docs: BTreeMap<u32, Document>,
    next_doc: u32,
    clipboard: Option<ClipSnapshot>,
    codec: Box<dyn FileCodec>,
    assist: Option<AssistProvider>,
    rng: Rng,
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
        }
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

    fn csv_rows(wb: &Workbook) -> Vec<Vec<(String, bool)>> {
        let sheet = wb.active_sheet;
        let Some(used) = wb.sheet(sheet).and_then(|s| s.used_range()) else {
            return Vec::new();
        };
        (0..=used.r1)
            .map(|r| {
                (0..=used.c1)
                    .map(|c| {
                        let addr = CellAddr::new(sheet, r, c);
                        let is_text = matches!(wb.value(addr), Value::Text(_));
                        (view::display_text(wb, addr), is_text)
                    })
                    .collect()
            })
            .collect()
    }
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
        let stem = path
            .file_stem()
            .map_or_else(|| "Sheet1".to_owned(), |s| s.to_string_lossy().into_owned());
        for s in &mut decoded.data.sheets {
            if s.name.trim().is_empty() {
                s.name.clone_from(&stem);
            }
        }
        let wb = Workbook::from_data_with(&decoded.data, decoded.recognize_text);
        let id = DocId(self.next_doc);
        self.next_doc += 1;
        let mut doc = Document::new(id, wb, &file_title(path));
        doc.path = Some(path.to_path_buf());
        let info = doc.info();
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
        let csv_rows = (format == FileFormat::Csv).then(|| Self::csv_rows(&d.wb));
        let bytes = self.codec.write(
            &Export {
                data: &data,
                csv_rows,
            },
            format,
        )?;
        std::fs::write(&target, bytes).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
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
