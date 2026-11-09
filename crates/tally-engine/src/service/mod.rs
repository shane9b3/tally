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
pub mod charts;
pub mod codec;
pub mod print;
pub mod templates;
pub mod tools;
pub mod view;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tally_api::{
    ApiError, ApplyResult, AutoFilterInfo, CellBlock, CellEditInfo, CellPos, ClipboardOut,
    Direction, DocId, DocumentService, FileFormat, FindQuery, FindResult, FunctionInfo, NameInfo,
    RangeDto, Result, SelectionDto, SelectionStats, SheetLayout, StyleDto, Suggestion, Transaction,
    WorkbookInfo,
};
use tally_functions::registry;
use tally_types::{CellAddr, CellRange, Value};

pub use assist::{AssistProvider, AssistRequest, SheetView};
#[cfg(feature = "io")]
pub use codec::IoCodec;
pub use codec::{Decoded, Export, FileCodec, UnsupportedCodec};

use crate::clipboard::{copy_to_clipboard, ClipSnapshot};
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
///
/// One service can hold any number of documents. The desktop shell instead
/// gives every document its own service, made with [`EngineService::fork`]
/// and locked on its own, so a long recalc or save in one document does not
/// block the others. Forks share what crosses documents ([`Shared`]).
pub struct EngineService {
    docs: BTreeMap<u32, Document>,
    shared: Arc<Shared>,
    codec: Arc<dyn FileCodec>,
    assist: Option<Arc<AssistProvider>>,
    rng: Rng,
    open_max_evaluated: u64,
    open_time_limit: Option<Duration>,
    /// `chart_data` responses per document version (`charts.rs`).
    chart_cache: Mutex<charts::ChartDataCache>,
}

/// What every fork of one service shares: document ids (unique across
/// forks), the internal clipboard (copy in one document, paste in another)
/// and the prepared print jobs (looked up by job id alone).
///
/// Its mutexes are leaf locks: held only to read, swap or push a value,
/// never across a document operation, a codec call or a PDF render, and no
/// other lock is taken while one is held.
struct Shared {
    next_doc: AtomicU32,
    /// Forks made so far (mixed into each fork's clipboard-token seed).
    forks: AtomicU64,
    clipboard: Mutex<Option<Arc<ClipSnapshot>>>,
    /// The application's two-digit year window start (AOO Tools ▸ Options
    /// ▸ General ▸ "Year (two digits)", `utl::MiscCfg::GetYear2000`), given
    /// to every document created or opened from now on whose file does not
    /// set it (`ScDocShell::InitOptions`).
    two_digit_year: AtomicU16,
    /// The application's undo depth (AOO Tools ▸ Options ▸ OpenOffice ▸
    /// Memory ▸ Undo "Number of steps", `SvtUndoOptions::GetUndoCount`),
    /// given to every document (`SfxShell::SetUndoManager`).
    undo_depth: AtomicU32,
    /// Prepared print jobs (an LRU of [`crate::print::JOB_LRU`]).
    #[cfg_attr(not(feature = "io"), allow(dead_code))]
    print_jobs: Mutex<print::JobCache>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            next_doc: AtomicU32::new(1),
            forks: AtomicU64::new(0),
            clipboard: Mutex::new(None),
            two_digit_year: AtomicU16::new(tally_types::DEFAULT_TWO_DIGIT_YEAR),
            undo_depth: AtomicU32::new(crate::undo::UNDO_DEPTH as u32),
            print_jobs: Mutex::new(print::JobCache::default()),
        }
    }
}

/// Locks one of the [`Shared`] leaf mutexes, recovering from poisoning
/// (each holds a plain value that a panic cannot leave half-written).
fn leaf<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
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
            .field("next_doc", &self.shared.next_doc.load(Ordering::Relaxed))
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
            shared: Arc::default(),
            codec: Arc::new(UnsupportedCodec),
            assist: None,
            rng: Rng::from_time(),
            open_max_evaluated: RecalcBudget::OPEN_MAX_CELLS,
            open_time_limit: Some(RecalcBudget::OPEN_TIME_LIMIT),
            chart_cache: Mutex::default(),
        }
    }

    /// A new service with no documents that shares this one's document ids,
    /// internal clipboard and print jobs ([`Shared`]) and has the same codec,
    /// ⌘K provider and open limits (changing those later on one service
    /// does not reach forks already made). Documents opened or created on a
    /// fork live in that fork only; any fork can serve any print job.
    pub fn fork(&self) -> Self {
        let n = self.shared.forks.fetch_add(1, Ordering::Relaxed) + 1;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self {
            docs: BTreeMap::new(),
            shared: Arc::clone(&self.shared),
            codec: Arc::clone(&self.codec),
            assist: self.assist.clone(),
            rng: Rng::new(nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
            open_max_evaluated: self.open_max_evaluated,
            open_time_limit: self.open_time_limit,
            chart_cache: Mutex::default(),
        }
    }

    /// The next document id (unique across all forks).
    fn next_doc_id(&self) -> DocId {
        DocId(self.shared.next_doc.fetch_add(1, Ordering::Relaxed))
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
        self.codec = Arc::from(codec);
        self
    }

    /// Installs the ⌘K provider used by `assist_suggest`.
    pub fn set_assist_provider(&mut self, provider: AssistProvider) {
        self.assist = Some(Arc::new(provider));
    }

    /// Sets the application's two-digit year window start for documents
    /// created or opened from now on, in this service and all its forks
    /// (AOO: the General option is copied into a document's options when
    /// it is created or loaded; documents already open keep theirs). A
    /// file's own `table:null-year` wins. Out-of-range years (AOO allows
    /// 1583–9857) are refused.
    pub fn set_two_digit_year_default(&self, year: u16) -> Result<()> {
        if !tally_types::TWO_DIGIT_YEAR_RANGE.contains(&year) {
            return Err(ApiError::invalid_argument(
                "the two-digit year start must be 1583–9857",
            ));
        }
        self.shared.two_digit_year.store(year, Ordering::Relaxed);
        Ok(())
    }

    /// The application's two-digit year window start (1930 by default).
    pub fn two_digit_year_default(&self) -> u16 {
        self.shared.two_digit_year.load(Ordering::Relaxed)
    }

    /// Sets the application's undo depth (AOO Tools ▸ Options ▸ OpenOffice
    /// ▸ Memory ▸ Undo "Number of steps", 1–1000) for this service and all
    /// its forks, and applies it to this service's open documents at once:
    /// a lower depth drops the steps farthest from the current state
    /// (`SfxApplication::SetOptions_Impl` calls `SetMaxUndoActionCount` on
    /// every shell's undo manager). Documents of other forks take it on
    /// their next edit or undo, or when the caller sets it on their fork
    /// too (the desktop shell does). Out-of-range depths are refused.
    pub fn set_undo_depth(&mut self, depth: u32) -> Result<()> {
        if !crate::undo::UNDO_DEPTH_RANGE.contains(&(depth as usize)) {
            return Err(ApiError::invalid_argument(
                "the number of undo steps must be 1–1000",
            ));
        }
        self.shared.undo_depth.store(depth, Ordering::Relaxed);
        for d in self.docs.values_mut() {
            d.undo.set_depth(depth as usize);
        }
        Ok(())
    }

    /// The application's undo depth (100 by default).
    pub fn undo_depth(&self) -> u32 {
        self.shared.undo_depth.load(Ordering::Relaxed)
    }

    /// Files a document opened or created on this service, with the
    /// application's undo depth.
    fn insert_document(&mut self, mut doc: Document) {
        doc.undo.set_depth(self.undo_depth() as usize);
        self.docs.insert(doc.id.0, doc);
    }

    /// Opens a workbook built elsewhere (tests, the CLI).
    pub fn open_workbook(&mut self, wb: Workbook, title: &str) -> WorkbookInfo {
        let id = self.next_doc_id();
        let doc = Document::new(id, wb, title);
        let info = doc.info();
        self.insert_document(doc);
        info
    }

    /// A document.
    pub fn document(&self, doc: DocId) -> Result<&Document> {
        self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))
    }

    /// A document, mutably.
    pub fn document_mut(&mut self, doc: DocId) -> Result<&mut Document> {
        if let Ok(cache) = self.chart_cache.get_mut() {
            cache.forget(doc.0);
        }
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
    /// Serializes `doc` in `format` and writes it to `target` atomically;
    /// the document itself (path, title, dirty flag) is not touched.
    fn write_doc(&self, doc: DocId, target: &Path, format: FileFormat) -> Result<Vec<String>> {
        let d = self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))?;
        let mut data = d.wb.to_data();
        data.meta.generator = Some(format!("Tally/{}", env!("CARGO_PKG_VERSION")));
        data.meta.modified = Some(crate::ops_v03::now_iso());
        if data.meta.created.is_none() {
            data.meta.created = data.meta.modified.clone();
        }
        let csv_rows = if format == FileFormat::Csv {
            Some(Self::csv_rows(&d.wb, d.csv_options.as_shown)?)
        } else {
            None
        };
        let warnings = csv_rows
            .as_deref()
            .map(|rows| codec::csv_warnings(rows, &d.csv_options))
            .unwrap_or_default();
        let bytes = self.codec.write(
            &Export {
                data: &data,
                csv_rows,
                csv_options: Some(d.csv_options.clone()),
            },
            format,
        )?;
        write_atomically(target, &bytes).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
        Ok(warnings)
    }

    /// The active sheet's rows for CSV: the displayed text (`as_shown`),
    /// else as AOO's `GetInputString` (numbers at full precision, dates in
    /// the input-line format), with a text-cell flag.
    fn csv_rows(wb: &Workbook, as_shown: bool) -> Result<Vec<Vec<(String, bool)>>> {
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
                let v = wb.value(addr);
                let is_text = matches!(v, Value::Text(_));
                let text = match &v {
                    // AOO `ScDocShell::AsciiSave`: "Hide all" cells of a
                    // protected sheet are written empty.
                    _ if crate::protection::cell_hidden(wb, sheet_id, r, c) => String::new(),
                    _ if as_shown => view::display_text(wb, addr),
                    Value::Text(t) => t.to_string(),
                    _ => view::edit_text(wb, &v, wb.styles.get(sheet.style(r, c))),
                };
                row.push((text, is_text));
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

impl EngineService {
    /// Reads and decodes `path` into a document with id `id` (not filed).
    fn load_file(&self, id: DocId, path: &Path) -> Result<Document> {
        let mut decoded = self.decode_file(path)?;
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
        // Files without calculation settings (XLSX, CSV, ODS without
        // `table:calculation-settings`) take the application's window
        // before their text is recognised.
        decoded
            .data
            .settings
            .calc_settings
            .two_digit_year
            .get_or_insert(self.two_digit_year_default());
        let wb = Workbook::from_data_owned(
            decoded.data,
            decoded.recognize_text,
            decoded.evaluate_formulas,
        );
        let budget = RecalcBudget::within(self.open_max_evaluated, self.open_time_limit);
        let mut doc = Document::with_budget(id, wb, &file_title(path), &budget);
        doc.path = Some(path.to_path_buf());
        if let Some(o) = decoded.csv_options {
            doc.csv_options = o;
        }
        doc.has_macros = decoded.has_macros;
        Ok(doc)
    }

    /// Reads and decodes `path` with the limits of an open (file size,
    /// the codec's own budgets, sheet count). Also Load Styles' reader:
    /// the other file is as untrusted as any file opened.
    fn decode_file(&self, path: &Path) -> Result<Decoded> {
        let meta =
            std::fs::metadata(path).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
        if meta.len() > MAX_FILE_BYTES {
            return Err(ApiError::new(
                ApiError::BAD_FILE,
                format!(
                    "limit exceeded: the file is too large or complex to open safely \
                     (file size over {} MiB)",
                    MAX_FILE_BYTES / (1024 * 1024)
                ),
            ));
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        // The file bytes are dropped as soon as they are decoded and the
        // decoded cells while they are loaded, so neither adds to the peak
        // memory of an open.
        let decoded = {
            let bytes =
                std::fs::read(path).map_err(|e| ApiError::new(ApiError::IO, e.to_string()))?;
            self.codec.read(&bytes, &ext)?
        };
        if decoded.data.sheets.len() > crate::workbook::MAX_SHEETS {
            return Err(ApiError::new(
                ApiError::BAD_FILE,
                format!(
                    "the file has more than {} sheets",
                    crate::workbook::MAX_SHEETS
                ),
            ));
        }
        Ok(decoded)
    }

    /// The document, if it may be edited; [`ApiError::READ_ONLY`] if it is
    /// read-only.
    fn editable(&mut self, doc: DocId) -> Result<&mut Document> {
        let depth = self.undo_depth() as usize;
        let d = self.document_mut(doc)?;
        if d.read_only {
            return Err(read_only_error());
        }
        // A depth changed on another fork reaches this document here.
        if d.undo.depth() != depth {
            d.undo.set_depth(depth);
        }
        Ok(d)
    }
}

/// The error of an edit to a read-only document (AOO's `STR_READONLYERR`).
pub fn read_only_error() -> ApiError {
    ApiError::new(ApiError::READ_ONLY, "Document opened in read-only mode.")
}

/// `true` if `a` and `b` name the same file (as given, or canonicalized).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

fn file_title(path: &Path) -> String {
    path.file_name().map_or_else(
        || "Untitled".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl DocumentService for EngineService {
    fn new_workbook(&mut self, template: Option<&str>) -> Result<WorkbookInfo> {
        let (mut wb, title) = match template {
            None => (Workbook::new(), "Untitled"),
            Some(t) => templates::build(t)
                .ok_or_else(|| ApiError::not_found(format!("no template {t:?}")))?,
        };
        wb.settings
            .two_digit_year
            .get_or_insert(self.two_digit_year_default());
        Ok(self.open_workbook(wb, title))
    }

    fn open(&mut self, path: &Path) -> Result<WorkbookInfo> {
        let id = self.next_doc_id();
        let mut doc = self.load_file(id, path)?;
        // AOO opens a file whose `LoadReadonly` is set read-only (its
        // "read-only UI"; Edit Mode switches it off without a reload).
        doc.read_only = doc.wb.load_readonly;
        let info = doc.info();
        // Warnings are delivered once, with the open result.
        doc.warnings.clear();
        doc.has_macros = false;
        self.insert_document(doc);
        Ok(info)
    }

    fn save(
        &mut self,
        doc: DocId,
        path: Option<&Path>,
        format: FileFormat,
    ) -> Result<WorkbookInfo> {
        let d = self.docs.get(&doc.0).ok_or_else(|| not_found_doc(doc))?;
        let target = match path {
            Some(p) => p.to_path_buf(),
            None => d
                .path
                .clone()
                .ok_or_else(|| ApiError::invalid_argument("the document has no file path yet"))?,
        };
        // A read-only document is never written over its own file (AOO
        // disables Save); Save As to another file makes it editable.
        if d.read_only && d.path.as_deref().is_some_and(|p| same_file(p, &target)) {
            return Err(read_only_error());
        }
        let warnings = self.write_doc(doc, &target, format)?;
        let d = self
            .docs
            .get_mut(&doc.0)
            .ok_or_else(|| not_found_doc(doc))?;
        d.title = file_title(&target);
        d.path = Some(target);
        d.dirty = false;
        d.read_only = false;
        let mut info = d.info();
        // Save warnings (CSV: unquoted delimiters, unmappable characters).
        info.warnings.extend(warnings);
        Ok(info)
    }

    fn open_untitled(&mut self, path: &Path, dirty: bool) -> Result<WorkbookInfo> {
        let mut info = self.open(path)?;
        let d = self.document_mut(info.doc)?;
        d.path = None;
        d.title = "Untitled".to_owned();
        d.dirty = dirty;
        // A copy (recovery, a template, AOO's Open Copy) is editable.
        d.read_only = false;
        info.read_only = false;
        info.path = None;
        info.title = d.title.clone();
        info.dirty = dirty;
        Ok(info)
    }

    fn info(&self, doc: DocId) -> Result<WorkbookInfo> {
        Ok(self.document(doc)?.info())
    }

    fn is_dirty(&self, doc: DocId) -> Result<bool> {
        Ok(self.document(doc)?.dirty)
    }

    fn set_read_only(&mut self, doc: DocId, read_only: bool) -> Result<WorkbookInfo> {
        let d = self.document_mut(doc)?;
        if read_only && d.dirty {
            // AOO saves or discards (a reload) before switching to read-only.
            return Err(ApiError::invalid_argument(
                "save or discard the changes before switching to read-only",
            ));
        }
        d.read_only = read_only;
        Ok(d.info())
    }

    fn reload(&mut self, doc: DocId) -> Result<WorkbookInfo> {
        let old = self.document(doc)?;
        let path = old
            .path
            .clone()
            .ok_or_else(|| ApiError::invalid_argument("the document has no file to reload"))?;
        let (read_only, version, title) = (old.read_only, old.version, old.title.clone());
        let mut fresh = self.load_file(doc, &path)?;
        fresh.read_only = read_only;
        // Versions only grow, so the UI never mistakes the reloaded cells
        // for cached ones.
        fresh.version = version.saturating_add(1);
        fresh.title = title;
        let info = fresh.info();
        fresh.warnings.clear();
        fresh.has_macros = false;
        if let Ok(cache) = self.chart_cache.get_mut() {
            cache.forget(doc.0);
        }
        self.insert_document(fresh);
        Ok(info)
    }

    fn detach_path(&mut self, doc: DocId) -> Result<WorkbookInfo> {
        let d = self.document_mut(doc)?;
        d.path = None;
        d.title = "Untitled".to_owned();
        d.read_only = false;
        Ok(d.info())
    }

    fn save_copy(&self, doc: DocId, path: &Path, format: FileFormat) -> Result<()> {
        self.write_doc(doc, path, format).map(|_| ())
    }

    fn set_csv_options(&mut self, doc: DocId, options: &tally_api::CsvExportOptions) -> Result<()> {
        options.delimiters()?;
        self.document_mut(doc)?.csv_options = options.clone();
        Ok(())
    }

    fn close(&mut self, doc: DocId) -> Result<()> {
        if let Ok(cache) = self.chart_cache.get_mut() {
            cache.forget(doc.0);
        }
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
                let mut dto = if d.wb.styles.contains(*id) {
                    d.wb.styles.to_dto(StyleId(*id))
                } else if let Some(e) = d.wb.transient.style(*id) {
                    crate::styles::style_dto(*id, &e.data, &e.format_code)
                } else {
                    return Err(ApiError::not_found(format!("no style {id}")));
                };
                // Cells without a family are drawn with the document's
                // default font (the grid falls back to the app font).
                if dto.font_family.is_none() {
                    dto.font_family.clone_from(&d.wb.default_font);
                }
                Ok(dto)
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
        // A clone of the Arc: the clipboard lock is not held during the apply.
        let clip = leaf(&self.shared.clipboard).clone();
        let d = self.editable(doc)?;
        Ok(d.apply(tx, clip.as_deref()))
    }

    fn undo(&mut self, doc: DocId) -> Result<ApplyResult> {
        Ok(self.editable(doc)?.undo())
    }

    fn redo(&mut self, doc: DocId) -> Result<ApplyResult> {
        Ok(self.editable(doc)?.redo())
    }

    fn undo_steps(&mut self, doc: DocId, count: u32) -> Result<ApplyResult> {
        Ok(self.editable(doc)?.undo_steps(count))
    }

    fn load_styles(
        &mut self,
        doc: DocId,
        path: &Path,
        opts: &tally_api::LoadStylesOptions,
        label: Option<&str>,
    ) -> Result<ApplyResult> {
        // Refuse a read-only document before reading the other file.
        self.editable(doc)?;
        let decoded = self.decode_file(path)?;
        let source = crate::loadstyles::StyleSource::from_data(&decoded.data);
        drop(decoded);
        self.editable(doc)?.load_styles(&source, opts, label)
    }

    fn load_styles_from_template(
        &mut self,
        doc: DocId,
        template: &str,
        opts: &tally_api::LoadStylesOptions,
        label: Option<&str>,
    ) -> Result<ApplyResult> {
        self.editable(doc)?;
        let (wb, _) = templates::build(template)
            .ok_or_else(|| ApiError::not_found(format!("no template {template:?}")))?;
        let source = crate::loadstyles::StyleSource::from_workbook(&wb);
        self.editable(doc)?.load_styles(&source, opts, label)
    }

    fn redo_steps(&mut self, doc: DocId, count: u32) -> Result<ApplyResult> {
        Ok(self.editable(doc)?.redo_steps(count))
    }

    fn undo_history(&self, doc: DocId) -> Result<tally_api::UndoHistory> {
        Ok(self
            .document(doc)?
            .undo
            .history(*crate::undo::UNDO_DEPTH_RANGE.end()))
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
        let (mut snap, tsv, html) = copy_to_clipboard(&d.wb, range, &mut self.rng);
        snap.source_doc = Some(doc);
        let token = snap.token.clone();
        *leaf(&self.shared.clipboard) = Some(Arc::new(snap));
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

    fn next_unlocked(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        from: CellPos,
        dir: Direction,
        within: Option<RangeDto>,
    ) -> Result<Option<CellPos>> {
        let wb = &self.document(doc)?.wb;
        sheet_of(wb, sheet)?;
        Ok(
            crate::protection::next_unlocked(wb, sheet, (from.r, from.c), dir, within.as_ref())
                .map(|(r, c)| CellPos { r, c }),
        )
    }

    fn data_region(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        range: RangeDto,
    ) -> Result<RangeDto> {
        let wb = &self.document(doc)?.wb;
        sheet_of(wb, sheet)?;
        let r = crate::charts::data_region(wb, sheet, (range.r0, range.c0, range.r1, range.c1));
        Ok(RangeDto {
            r0: r.r0,
            c0: r.c0,
            r1: r.r1,
            c1: r.c1,
        })
    }

    fn paste_overwrites(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        target: RangeDto,
        payload: tally_api::PastePayload,
        options: tally_api::PasteOptions,
    ) -> Result<bool> {
        let wb = &self.document(doc)?.wb;
        let s = sheet_of(wb, sheet)?;
        let clip = leaf(&self.shared.clipboard).clone();
        let target = to_range(sheet, &target)?;
        let Some(area) =
            crate::clipboard::paste_area(sheet, target, &payload, &options, clip.as_deref())?
        else {
            return Ok(false);
        };
        // Notes count as data (AOO `IsBlockEmpty` with bIgnoreNotes off,
        // document.hxx:882).
        Ok((area.c0..=area.c1).any(|c| {
            s.column(c)
                .is_some_and(|col| col.cells.iter_range(area.r0, area.r1).next().is_some())
        }) || crate::annotations::entries_in(&s.meta.notes, &area)
            .next()
            .is_some())
    }

    fn resolve_name_box(&self, doc: DocId, text: &str) -> Result<SelectionDto> {
        view::resolve_name_box(&self.document(doc)?.wb, text)
    }

    fn resolve_name_box_in(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        text: &str,
    ) -> Result<SelectionDto> {
        view::resolve_name_box_in(&self.document(doc)?.wb, sheet, text)
    }

    fn set_view_zoom(&mut self, doc: DocId, zoom: u16) -> Result<()> {
        if !(tally_model::MIN_ZOOM..=tally_model::MAX_ZOOM).contains(&zoom) {
            return Err(ApiError::invalid_argument("the zoom must be 20–400 %"));
        }
        // View state only, like the active sheet: no undo, no dirty flag.
        let d = self
            .docs
            .get_mut(&doc.0)
            .ok_or_else(|| not_found_doc(doc))?;
        let z = tally_model::zoom_from_file(i64::from(zoom));
        for s in &mut d.wb.sheets {
            s.meta.zoom = z;
        }
        Ok(())
    }

    fn set_active_sheet(&mut self, doc: DocId, sheet: tally_types::SheetId) -> Result<()> {
        // View state only: no undo step, no dirty flag, no cache to drop.
        let d = self
            .docs
            .get_mut(&doc.0)
            .ok_or_else(|| not_found_doc(doc))?;
        if d.wb.sheet(sheet).is_none() {
            return Err(ApiError::not_found(format!("no sheet {sheet}")));
        }
        d.wb.active_sheet = sheet;
        Ok(())
    }

    fn list_functions(&self) -> Vec<FunctionInfo> {
        let mut out: Vec<FunctionInfo> = registry()
            .iter()
            .map(|(_, d)| {
                let meta = tally_functions::FunctionMeta::from_def(d);
                FunctionInfo {
                    name: meta.name,
                    category: meta.category,
                    signature: d.signature().replace("; ", ", "),
                    description: meta.help,
                    params: meta
                        .params
                        .into_iter()
                        .map(|p| tally_api::ParamInfo {
                            name: p.name,
                            kind: p.kind,
                            optional: p.optional,
                            repeat: p.repeat,
                        })
                        .collect(),
                    example: meta.example,
                }
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

    fn names(&self, doc: DocId) -> Result<Vec<NameInfo>> {
        let wb = &self.document(doc)?.wb;
        let mut out: Vec<NameInfo> = wb
            .names
            .defined()
            .filter_map(|(_, e)| {
                let def = e.def.as_ref()?;
                let sheet_name = match e.scope {
                    None => None,
                    Some(s) => Some(wb.sheet(s)?.name.to_string()),
                };
                Some(NameInfo {
                    name: e.name.to_string(),
                    scope: e.scope,
                    refers_to: wb.name_text_of(def),
                    sheet_name,
                    usage: tally_api::NameUsage {
                        print_range: e.usage.print_range,
                        filter: e.usage.filter,
                        repeat_row: e.usage.repeat_row,
                        repeat_column: e.usage.repeat_column,
                    },
                })
            })
            .collect();
        out.sort_by(|a, b| {
            let tab = |s: Option<tally_types::SheetId>| s.and_then(|s| wb.tab_of(s));
            tab(a.scope)
                .cmp(&tab(b.scope))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    fn filter_info(&self, doc: DocId, sheet: tally_types::SheetId) -> Result<AutoFilterInfo> {
        crate::filter::filter_info(&self.document(doc)?.wb, sheet)
            .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
    }

    fn find(&self, doc: DocId, query: &FindQuery) -> Result<FindResult> {
        crate::find::find(&self.document(doc)?.wb, query)
    }

    fn detect_header(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        range: RangeDto,
    ) -> Result<bool> {
        let wb = &self.document(doc)?.wb;
        if wb.sheet(sheet).is_none() {
            return Err(ApiError::not_found(format!("no sheet {sheet}")));
        }
        let r = to_range(sheet, &range)?;
        Ok(crate::sort::detect_header(wb, &r))
    }

    fn recalc_all(&mut self, doc: DocId) -> Result<ApplyResult> {
        Ok(self.document_mut(doc)?.recalc_all())
    }

    // ----- v0.3 -----------------------------------------------------------

    fn conditional_formats(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<Vec<tally_api::ConditionalFormatDto>> {
        let wb = &self.document(doc)?.wb;
        let s = sheet_of(wb, sheet)?;
        Ok(s.meta
            .cond_formats
            .iter()
            .map(|cf| crate::condfmt::to_api(wb, sheet, cf))
            .collect())
    }

    fn validations(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<Vec<tally_api::ValidationDto>> {
        let wb = &self.document(doc)?.wb;
        let s = sheet_of(wb, sheet)?;
        Ok(s.meta
            .validations
            .iter()
            .map(|v| crate::validation::to_api(wb, sheet, v))
            .collect())
    }

    fn validation_at(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<Option<tally_api::ValidationInfo>> {
        let wb = &self.document(doc)?.wb;
        sheet_of(wb, sheet)?;
        Ok(crate::validation::info_at(wb, sheet, r, c))
    }

    fn note(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<Option<tally_api::NoteDto>> {
        let wb = &self.document(doc)?.wb;
        let s = sheet_of(wb, sheet)?;
        Ok(s.meta.notes.get(&(r, c)).map(|n| n.to_dto(r, c)))
    }

    fn notes(&self, doc: DocId, sheet: tally_types::SheetId) -> Result<Vec<tally_api::NoteDto>> {
        let wb = &self.document(doc)?.wb;
        let s = sheet_of(wb, sheet)?;
        Ok(s.meta
            .notes
            .iter()
            .map(|((r, c), n)| n.to_dto(*r, *c))
            .collect())
    }

    fn protection_info(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<tally_api::ProtectionInfo> {
        let wb = &self.document(doc)?.wb;
        crate::protection::info(wb, sheet)
            .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
    }

    fn page_setup(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<tally_api::PageSetupDto> {
        let wb = &self.document(doc)?.wb;
        Ok(crate::pagesetup::to_api(&sheet_of(wb, sheet)?.meta))
    }

    fn document_properties(&self, doc: DocId) -> Result<tally_api::DocPropertiesDto> {
        Ok(crate::pagesetup::props_to_api(&self.document(doc)?.wb.meta))
    }

    fn goal_seek(
        &self,
        doc: DocId,
        req: &tally_api::GoalSeekRequest,
    ) -> Result<tally_api::GoalSeekResult> {
        crate::goalseek::goal_seek(&self.document(doc)?.wb, req)
    }

    fn list_locales(&self) -> Vec<tally_api::LocaleInfo> {
        tally_numfmt::LOCALES
            .iter()
            .map(|l| tally_api::LocaleInfo {
                tag: l.tag.to_string(),
                decimal_sep: l.decimal_sep.to_string(),
                group_sep: l.group_sep.to_string(),
                list_sep: l.list_sep.to_string(),
                currency_symbol: l.currency_symbol.to_string(),
            })
            .collect()
    }

    fn set_locale(&mut self, doc: DocId, locale: &str) -> Result<ApplyResult> {
        self.editable(doc)?.set_locale(locale)
    }

    // ----- v0.5 (W5-2) ----------------------------------------------------

    fn pivots(&self, doc: DocId) -> Result<Vec<tally_api::PivotDto>> {
        tools::pivots(&self.document(doc)?.wb)
    }

    fn pivot_field_items(
        &self,
        doc: DocId,
        source: &tally_api::PivotSource,
    ) -> Result<Vec<tally_api::PivotFieldItemsDto>> {
        tools::pivot_field_items(&self.document(doc)?.wb, source)
    }

    fn pivot_at(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<Option<tally_api::PivotHit>> {
        tools::pivot_at(&self.document(doc)?.wb, sheet, r, c)
    }

    fn autocomplete(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
        prefix: &str,
    ) -> Result<tally_api::AutocompleteDto> {
        tools::autocomplete(&self.document(doc)?.wb, sheet, r, c, prefix)
    }

    fn cell_styles(&self, doc: DocId) -> Result<Vec<tally_api::CellStyleDto>> {
        tools::cell_styles(&self.document(doc)?.wb)
    }

    fn scenarios(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<Vec<tally_api::ScenarioDto>> {
        tools::scenarios(&self.document(doc)?.wb, sheet)
    }

    fn database_ranges(&self, doc: DocId) -> Result<Vec<tally_api::DatabaseRangeDto>> {
        tools::database_ranges(&self.document(doc)?.wb)
    }

    fn navigator(&self, doc: DocId) -> Result<tally_api::NavigatorDto> {
        tools::navigator(&self.document(doc)?.wb)
    }

    fn document_statistics(&self, doc: DocId) -> Result<tally_api::DocumentStatsDto> {
        let d = self.document(doc)?;
        let mut stats = tools::document_statistics(&d.wb)?;
        // Pages: every visible sheet at its own page setup, empty pages
        // suppressed (AOO's File ▸ Properties ▸ Statistics).
        let opts = tally_api::PrintOptionsDto {
            sheets: tally_api::PrintSheets::All,
            selection: None,
            suppress_empty_pages: true,
            page_from: None,
            page_to: None,
            active_sheet: None,
            page_ranges: None,
        };
        let ctx = crate::print::HfContext::now(&d.wb, &d.title, None);
        stats.pages = crate::print::layout(&d.wb, &opts, &ctx)
            .ok()
            .map(|(pages, _)| pages.len() as u32);
        Ok(stats)
    }

    fn custom_sort_lists(&self, doc: DocId) -> Result<Vec<Vec<String>>> {
        Ok(self.document(doc)?.wb.custom_sort_lists.clone())
    }

    // ----- v0.4 charts and pictures (W5-1) ----------------------------------

    fn charts(&self, doc: DocId, sheet: tally_types::SheetId) -> Result<Vec<tally_api::ChartDto>> {
        self.charts_impl(doc, sheet)
    }

    fn chart_data(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        id: u32,
    ) -> Result<tally_api::ChartDataDto> {
        self.chart_data_impl(doc, sheet, id)
    }

    fn chart_data_table(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        id: u32,
    ) -> Result<tally_api::ChartDataTableDto> {
        self.chart_data_table_impl(doc, sheet, id)
    }

    fn suggest_chart(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        sel: tally_api::RangeDto,
        kind: tally_api::ChartKind,
    ) -> Result<tally_api::ChartDto> {
        self.suggest_chart_impl(doc, sheet, sel, kind)
    }

    fn images(&self, doc: DocId, sheet: tally_types::SheetId) -> Result<Vec<tally_api::ImageDto>> {
        self.images_impl(doc, sheet)
    }

    fn image_data(&self, doc: DocId, sheet: tally_types::SheetId, id: u32) -> Result<String> {
        Ok(self.image_bytes(doc, sheet, id)?.data)
    }

    // ----- v0.6 (W5-3): calculation settings, solver, detective, print -----

    fn calc_settings(&self, doc: DocId) -> Result<tally_api::CalcSettingsDto> {
        Ok(crate::calcsettings::to_api(
            &self.document(doc)?.wb.settings,
        ))
    }

    fn solver(
        &self,
        doc: DocId,
        req: &tally_api::SolverRequest,
    ) -> Result<tally_api::SolverResult> {
        crate::solver::solve(&self.document(doc)?.wb, req)
    }

    fn precedents(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<Vec<tally_api::TraceDto>> {
        crate::detective::precedents(&self.document(doc)?.wb, sheet, r, c)
    }

    fn dependents(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
    ) -> Result<Vec<tally_api::TraceDto>> {
        crate::detective::dependents(&self.document(doc)?.wb, sheet, r, c)
    }

    fn detective(
        &mut self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
        action: tally_api::DetectiveAction,
    ) -> Result<tally_api::DetectiveState> {
        self.document_mut(doc)?.detective(sheet, r, c, action)
    }

    fn detective_arrows(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<tally_api::DetectiveState> {
        let wb = &self.document(doc)?.wb;
        if wb.sheet(sheet).is_none() {
            return Err(ApiError::not_found(format!("no sheet {sheet}")));
        }
        let mut st = crate::detective::state(wb, sheet, false);
        st.dirty = self.document(doc)?.dirty;
        Ok(st)
    }

    fn set_detective_auto(&mut self, doc: DocId, on: bool) -> Result<()> {
        self.document_mut(doc)?.detective_auto = on;
        Ok(())
    }

    fn print_layout(
        &self,
        doc: DocId,
        opts: &tally_api::PrintOptionsDto,
    ) -> Result<tally_api::PrintLayoutDto> {
        print::print_layout(self, doc, opts)
    }

    fn page_breaks(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<tally_api::PageBreaksDto> {
        print::page_breaks(self, doc, sheet)
    }

    fn prepare_print_job(
        &mut self,
        doc: DocId,
        opts: &tally_api::PrintOptionsDto,
        charts: &[tally_api::ChartSvgDto],
    ) -> Result<tally_api::PrintJobInfo> {
        print::prepare_print_job(self, doc, opts, charts)
    }

    fn print_job_page_png(&self, job: u32, page: u32, dpi: u32) -> Result<String> {
        print::print_job_page_png(self, job, page, dpi)
    }

    fn print_job_pdf(&self, job: u32) -> Result<Vec<u8>> {
        print::print_job_pdf(self, job)
    }

    fn view_options(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
    ) -> Result<tally_api::ViewOptionsDto> {
        let d = self.document(doc)?;
        crate::tools::view_options(&d.wb, sheet)
            .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
    }

    fn invalid_cells(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        range: Option<RangeDto>,
    ) -> Result<Vec<CellPos>> {
        let d = self.document(doc)?;
        crate::validation::invalid_cells(&d.wb, sheet, range)
    }

    fn cells_with(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        ranges: &[RangeDto],
        opts: &tally_api::CellsOptions,
    ) -> Result<Vec<CellBlock>> {
        let d = self.document(doc)?;
        let mut blocks = view::cells(&d.wb, d.version, sheet, ranges)?;
        view::apply_cells_options(&d.wb, sheet, &mut blocks, opts);
        Ok(blocks)
    }

    fn preview_formula(
        &self,
        doc: DocId,
        sheet: tally_types::SheetId,
        r: u32,
        c: u32,
        text: &str,
    ) -> Result<Option<String>> {
        let d = self.document(doc)?;
        view::preview_formula(d, sheet, r, c, text)
    }
}

fn sheet_of(wb: &Workbook, sheet: tally_types::SheetId) -> Result<&crate::storage::Sheet> {
    wb.sheet(sheet)
        .ok_or_else(|| ApiError::not_found(format!("no sheet {sheet}")))
}
