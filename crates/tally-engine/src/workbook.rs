//! The workbook: sheets in tab order, styles, formula cells, the dependency
//! index and settings. Cell-level mutators record what changed so the next
//! [`crate::recalc`] knows where to start.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tally_formula::{
    parse, print, Dialect, Expr, Formula, FuncId as AstFuncId, FuncRef, NameId, ResolvedRef,
    Resolver,
};
use tally_functions::{registry, FnFlags, FuncId, ParamKind};
use tally_model::DocMeta;
use tally_numfmt::{recognize_input_with, Locale, RecognizeContext, Recognized, EN_US};
use tally_types::{CalcSettings, CellAddr, CellRange, SheetId, Value, MAX_COLS, MAX_ROWS};

use crate::deps::{DepIndex, HashDepIndex};
use crate::formula_cells::{CellState, FormulaCell, FormulaCellId, FormulaCells, Span3d};
use crate::storage::{CellContent, ColumnStore, Sheet, StyleId};
use crate::styles::StyleTable;

/// Hard cap on sheets per workbook (LibreOffice allows 10,000; AOO 256).
pub const MAX_SHEETS: usize = 10_000;

/// Case-insensitive key for sheet-name uniqueness.
fn name_key(name: &str) -> String {
    name.to_lowercase()
}

/// Makes sheet names unique (case-insensitively) in amortized O(1) per name:
/// a set of taken names plus, per base name, the next suffix to try, so
/// thousands of equal names do not rescan `_2`, `_3`, … each time.
#[derive(Debug, Default)]
pub struct SheetNamer {
    taken: HashSet<String>,
    next_suffix: HashMap<String, u64>,
    /// `Sheet{n}` is known taken for every `n` in `known_taken`.
    known_taken: std::ops::Range<u64>,
}

impl SheetNamer {
    /// A namer that treats `names` as taken.
    pub fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            taken: names.into_iter().map(name_key).collect(),
            next_suffix: HashMap::new(),
            known_taken: 1..1,
        }
    }

    /// `name` if free, else `name_2`, `name_3`, …; the result is now taken.
    pub fn claim(&mut self, name: &str) -> String {
        let base = name_key(name);
        if self.taken.insert(base.clone()) {
            return name.to_owned();
        }
        let mut i = self.next_suffix.get(&base).copied().unwrap_or(2);
        loop {
            let cand = format!("{name}_{i}");
            i += 1;
            if self.taken.insert(name_key(&cand)) {
                self.next_suffix.insert(base, i);
                return cand;
            }
        }
    }

    /// The first free `SheetN` with `N >= from`; the result is now taken.
    pub fn claim_numbered(&mut self, from: u64) -> String {
        let r = &self.known_taken;
        let mut n = if r.start <= from && from <= r.end {
            r.end
        } else {
            from
        };
        loop {
            let cand = format!("Sheet{n}");
            if self.taken.insert(name_key(&cand)) {
                // Names are never released, so `from..=n` stays taken.
                self.known_taken = from..n + 1;
                return cand;
            }
            n += 1;
        }
    }
}

/// Content of one cell as captured for undo and the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapContent {
    /// A constant.
    Value(Value),
    /// A formula (relative AST) and, for unparsable input, the raw text.
    Formula {
        /// The formula.
        formula: Arc<Formula>,
        /// Raw text of an invalid formula.
        invalid_text: Option<Arc<str>>,
        /// For an array formula anchor: the block size `(rows, cols)`.
        array: Option<(u32, u32)>,
    },
}

impl SnapContent {
    /// A plain (non-array) formula.
    pub fn formula(formula: Arc<Formula>, invalid_text: Option<Arc<str>>) -> Self {
        SnapContent::Formula {
            formula,
            invalid_text,
            array: None,
        }
    }
}

/// A small xorshift64* generator for RAND (the `rand` crate is not a dependency).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// Seeded from the clock.
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        Self::new(nanos)
    }

    /// Seeded with `seed` (0 is replaced by a constant).
    pub fn new(seed: u64) -> Self {
        let s = seed ^ 0x9E37_79B9_7F4A_7C15;
        Rng(if s == 0 { 0x2545_F491_4F6C_DD1D } else { s })
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// What changed since the last recalc.
#[derive(Clone, Debug, Default)]
pub struct Pending {
    /// Edited cells.
    pub cells: Vec<CellAddr>,
    /// Edited ranges (clears, pastes, style changes).
    pub ranges: Vec<CellRange>,
    /// Formula cells created dirty.
    pub dirty: Vec<FormulaCellId>,
    /// Ranges whose display changed without a value change (styles).
    pub display_only: Vec<CellRange>,
}

/// State shared by the cells of one bulk load (see
/// [`Workbook::set_input_bulk`]).
#[derive(Debug)]
pub(crate) struct BulkInput {
    /// Recognition context (today's date), read once.
    pub ctx: RecognizeContext,
    /// `(style, implied format code)` → the derived style.
    pub formats: HashMap<(StyleId, String), StyleId>,
    /// Inputs already recognized in this load (a function of the input,
    /// the locale and `ctx` only), at most [`BULK_RECOGNIZED_CAP`] of at
    /// most [`BULK_RECOGNIZED_MAX_LEN`] bytes. A
    /// repeated text also shares one allocation.
    pub recognized: HashMap<Arc<str>, Recognized>,
}

/// Most distinct inputs remembered by [`BulkInput::recognized`].
const BULK_RECOGNIZED_CAP: usize = 1 << 16;

/// Longest input (bytes) remembered by [`BulkInput::recognized`], as for
/// the readers' text interners: long unique texts (descriptions) are not
/// worth a copy.
const BULK_RECOGNIZED_MAX_LEN: usize = 64;

impl BulkInput {
    pub(crate) fn new(settings: &tally_types::CalcSettings) -> Self {
        Self {
            ctx: RecognizeContext::for_settings(settings),
            formats: HashMap::new(),
            recognized: HashMap::new(),
        }
    }
}

/// `input` as a number when it is a plain decimal number (`-?d+(.d+)?`,
/// the `.` only where it is the locale's decimal separator) that the input
/// recognizer would read as exactly that number with no implied format;
/// `None` sends the input through the full recognizer.
pub(crate) fn plain_number(input: &str, locale: &Locale) -> Option<f64> {
    let b = input.as_bytes();
    let (neg, digits) = match b.first()? {
        b'-' => (true, &b[1..]),
        _ => (false, b),
    };
    let dot_ok = locale.decimal_sep == '.' && !locale.is_group_char('.');
    let mut int_len = 0;
    while int_len < digits.len() && digits[int_len].is_ascii_digit() {
        int_len += 1;
    }
    if int_len == 0 || int_len > 15 {
        return None;
    }
    if int_len < digits.len() {
        let frac = &digits[int_len + 1..];
        if !dot_ok
            || digits[int_len] != b'.'
            || frac.is_empty()
            || frac.len() > 15
            || !frac.iter().all(u8::is_ascii_digit)
        {
            return None;
        }
    }
    // The recognizer checks the boolean names and a currency prefix first;
    // none of the supported locales spells them with digits, but stay on
    // the full path if one ever does.
    let numeric = |s: &str| s.is_empty() || s.bytes().any(|c| c.is_ascii_digit() || c == b'-');
    if numeric(locale.true_name) || numeric(locale.false_name) || numeric(locale.currency_symbol) {
        return None;
    }
    // `digits` is ASCII digits with at most one `.`, as the recognizer's
    // own `parse_unsigned` builds it.
    let v: f64 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// The workbook.
#[derive(Clone, Debug)]
pub struct Workbook {
    /// Sheets in tab order.
    pub sheets: Vec<Sheet>,
    /// Sheet id → tab index.
    pub sheet_index: HashMap<SheetId, usize>,
    /// The tab order 3-D spans were last checked against
    /// ([`Workbook::refresh_all_precedents`]).
    span_order: Vec<SheetId>,
    next_sheet_id: SheetId,
    /// Styles.
    pub styles: StyleTable,
    /// Calculation settings.
    pub settings: CalcSettings,
    /// Formula cells.
    pub formulas: FormulaCells,
    /// Reverse dependency index.
    pub deps: HashDepIndex,
    /// Random numbers for RAND/RANDBETWEEN.
    pub rng: Rng,
    /// Changes since the last recalc.
    pub pending: Pending,
    /// Document metadata.
    pub meta: DocMeta,
    /// Id of the active sheet.
    pub active_sheet: SheetId,
    /// Named ranges and expressions.
    pub names: crate::names::NameTable,
    /// Locale for formatting and input recognition.
    pub locale: &'static Locale,
    /// Document structure protection (`None`: not protected).
    pub structure_protection: Option<tally_model::SheetProtectionData>,
    /// Caches derived from the document (conditional formats).
    pub transient: crate::condfmt::Transient,
    /// What loading dropped or changed to stay within limits (the
    /// reader's warnings plus the engine's); [`crate::Document`] shows them
    /// once when the file opens.
    pub import_warnings: Vec<String>,
    /// While loading: original (lowercase) names of sheets that were
    /// renamed to valid names, so the file's formulas still resolve them.
    pub(crate) load_aliases: HashMap<String, SheetId>,
    /// Named cell styles with their parent chains (v0.4, W5-2; see
    /// [`crate::cellstyles`]). Empty means only the built-in `Default`.
    pub cell_styles: Vec<tally_model::CellStyleData>,
    /// Custom sort lists (v0.5).
    pub custom_sort_lists: Vec<Vec<String>>,
    /// Document-wide view options (comment indicator, grid colour, sheet
    /// tabs, outline symbols, page breaks; `SetViewOptions`).
    pub doc_view: tally_model::DocViewData,
    /// The document's default font family (cells whose style has no
    /// family use it); `None` for the application font (see
    /// [`crate::fonts`]).
    pub default_font: Option<String>,
    /// The file's `LoadReadonly` setting (ODS `settings.xml`): open it
    /// read-only. Written back on save.
    pub load_readonly: bool,
    /// External links the reader disabled by policy (v0.6, W5-3; see
    /// [`crate::external`]).
    pub external_links: Vec<tally_api::ExternalLinkWarning>,
    /// `MULTIPLE.OPERATIONS` sandbox evaluations in the current recalc
    /// (capped at [`crate::tools::MAX_TABLE_OPS`]).
    pub table_op_calls: u32,
    /// `MULTIPLE.OPERATIONS` sandbox work in the current recalc (capped at
    /// `recalc::MAX_TABLE_OP_WORK`, security review 4, H3).
    pub table_op_work: u64,
    /// Lookup indexes shared by exact-match lookups; `Some` only while a
    /// recalc runs (see [`crate::lookup_cache`]).
    pub(crate) lookup_cache: Option<crate::lookup_cache::LookupCache>,
    /// Single- or multithreaded recalculation for this workbook
    /// ([`crate::parallel`]); `Auto` follows the app setting. Not saved.
    pub threading: crate::parallel::Threading,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    /// A workbook with one sheet, "Sheet1".
    pub fn new() -> Self {
        let mut wb = Self::empty();
        wb.add_sheet(Some("Sheet1"), None);
        wb
    }

    /// A workbook without sheets (for loading).
    pub fn empty() -> Self {
        Self {
            sheets: Vec::new(),
            sheet_index: HashMap::new(),
            span_order: Vec::new(),
            next_sheet_id: 1,
            styles: StyleTable::new(),
            settings: CalcSettings::default(),
            formulas: FormulaCells::with_key(),
            deps: HashDepIndex::new(),
            rng: Rng::from_time(),
            pending: Pending::default(),
            meta: DocMeta::default(),
            active_sheet: 1,
            names: crate::names::NameTable::default(),
            locale: &EN_US,
            structure_protection: None,
            transient: crate::condfmt::Transient::default(),
            import_warnings: Vec::new(),
            load_aliases: HashMap::new(),
            cell_styles: Vec::new(),
            custom_sort_lists: Vec::new(),
            doc_view: tally_model::DocViewData::default(),
            default_font: None,
            load_readonly: false,
            external_links: Vec::new(),
            table_op_calls: 0,
            table_op_work: 0,
            lookup_cache: None,
            threading: crate::parallel::Threading::Auto,
        }
    }

    /// The formula dialect of the UI for the current locale (`;` argument
    /// separators where the locale's decimal separator is `,`).
    pub fn ui_dialect(&self) -> Dialect {
        let arg_sep = if self.locale.list_sep == ';' {
            tally_formula::ArgSep::Semicolon
        } else {
            tally_formula::ArgSep::Comma
        };
        Dialect::CalcA1 {
            arg_sep,
            sheet_sep: tally_formula::SheetSep::Dot,
        }
    }

    /// Sets the locale by tag; `false` if unknown.
    pub fn set_locale(&mut self, tag: &str) -> bool {
        match tally_numfmt::locale_by_tag(tag) {
            Some(l) => {
                self.locale = l;
                true
            }
            None => false,
        }
    }

    // ----- sheets ---------------------------------------------------------

    pub(crate) fn reindex(&mut self) {
        self.sheet_index = self
            .sheets
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, i))
            .collect();
        let order: Vec<SheetId> = self.sheets.iter().map(|s| s.id).collect();
        self.deps.set_tab_order(&order);
    }

    /// The sheets a 3-D span covers now, in tab order (empty if an end
    /// sheet is gone).
    pub fn span_sheets(&self, s: &Span3d) -> &[Sheet] {
        match (self.tab_of(s.first), self.tab_of(s.last)) {
            (Some(a), Some(b)) => self.sheets.get(a.min(b)..=a.max(b)).unwrap_or_default(),
            _ => &[],
        }
    }

    /// Whether a 3-D span covers any cell of `r`.
    pub fn span_hits(&self, s: &Span3d, r: &CellRange) -> bool {
        match (
            self.tab_of(s.first),
            self.tab_of(s.last),
            self.tab_of(r.sheet),
        ) {
            (Some(a), Some(b), Some(t)) => a.min(b) <= t && t <= a.max(b) && s.rect_hits(r),
            _ => false,
        }
    }

    /// Whether formula `f` reads (statically) any cell of `r`.
    pub fn reads_range(&self, f: &FormulaCell, r: &CellRange) -> bool {
        f.precedents.iter().any(|p| p.intersects(r))
            || f.spans().iter().any(|s| self.span_hits(s, r))
    }

    /// Whether formula `f` reads (statically) anything on one of `sheets`.
    pub fn reads_sheet(&self, f: &FormulaCell, sheets: &[SheetId]) -> bool {
        f.precedents.iter().any(|p| sheets.contains(&p.sheet))
            || f.spans()
                .iter()
                .any(|s| self.span_sheets(s).iter().any(|x| sheets.contains(&x.id)))
    }

    /// The sheet with id `id`.
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheet_index.get(&id).and_then(|i| self.sheets.get(*i))
    }

    /// The sheet with id `id`, mutably.
    pub fn sheet_mut(&mut self, id: SheetId) -> Option<&mut Sheet> {
        let i = *self.sheet_index.get(&id)?;
        self.sheets.get_mut(i)
    }

    /// Tab index of a sheet.
    pub fn tab_of(&self, id: SheetId) -> Option<usize> {
        self.sheet_index.get(&id).copied()
    }

    /// Sheet id by (case-insensitive) name.
    pub fn sheet_by_name_ci(&self, name: &str) -> Option<SheetId> {
        // The name is lowercased once, and a sheet name only when it is not
        // ASCII (review 2 INFO: two allocations per sheet per lookup).
        let lower = name.to_lowercase();
        self.sheets
            .iter()
            .find(|s| {
                s.name.eq_ignore_ascii_case(name)
                    || ((!s.name.is_ascii() || !name.is_ascii()) && s.name.to_lowercase() == lower)
            })
            .map(|s| s.id)
    }

    /// A [`SheetNamer`] over the current sheet names (except `except`).
    fn namer(&self, except: Option<SheetId>) -> SheetNamer {
        SheetNamer::new(
            self.sheets
                .iter()
                .filter(|s| Some(s.id) != except)
                .map(|s| &*s.name),
        )
    }

    /// The next free `SheetN` name.
    pub fn next_sheet_name(&self) -> String {
        self.namer(None)
            .claim_numbered(self.sheets.len() as u64 + 1)
    }

    /// `name` made unique among the sheets by appending `_2`, `_3`, …
    pub fn unique_sheet_name(&self, name: &str, except: Option<SheetId>) -> String {
        self.namer(except).claim(name)
    }

    /// A fresh sheet id.
    pub(crate) fn alloc_sheet_id(&mut self) -> SheetId {
        let id = self.next_sheet_id;
        self.next_sheet_id += 1;
        id
    }

    /// Adds a sheet (name made unique) at tab `index` (appends when `None`).
    pub fn add_sheet(&mut self, name: Option<&str>, index: Option<usize>) -> SheetId {
        let id = self.next_sheet_id;
        self.next_sheet_id += 1;
        let name = match name {
            Some(n) if !n.trim().is_empty() => self.unique_sheet_name(n, None),
            _ => self.next_sheet_name(),
        };
        let at = index.unwrap_or(self.sheets.len()).min(self.sheets.len());
        self.sheets.insert(at, Sheet::new(id, &name));
        if at + 1 == self.sheets.len() {
            // Appending moves no other sheet: no full reindex (a file with
            // thousands of sheets is loaded in linear time).
            self.sheet_index.insert(id, at);
            self.deps.append_tab(id, at as u32);
        } else {
            self.reindex();
        }
        if self.sheets.len() == 1 {
            self.active_sheet = id;
        }
        id
    }

    /// Inserts a previously removed sheet with its id at `index`.
    pub fn insert_sheet(&mut self, sheet: Sheet, index: usize) {
        if sheet.id >= self.next_sheet_id {
            self.next_sheet_id = sheet.id + 1;
        }
        let at = index.min(self.sheets.len());
        self.sheets.insert(at, sheet);
        self.reindex();
    }

    /// Removes a sheet (and its formula cells) and returns it.
    pub fn remove_sheet(&mut self, id: SheetId) -> Option<Sheet> {
        let i = self.tab_of(id)?;
        let sheet = self.sheets.remove(i);
        self.reindex();
        for col in &sheet.columns {
            for (_, c) in col.cells.iter_range(0, MAX_ROWS - 1) {
                if let CellContent::Formula(fid) = c {
                    self.deps.forget(*fid);
                    self.formulas.remove(*fid);
                }
            }
        }
        if self.active_sheet == id {
            self.active_sheet = self
                .sheets
                .get(i.min(self.sheets.len().saturating_sub(1)))
                .map_or(0, |s| s.id);
        }
        Some(sheet)
    }

    /// Moves a sheet to tab `to`; returns the old tab index.
    pub fn move_sheet(&mut self, id: SheetId, to: usize) -> Option<usize> {
        let from = self.tab_of(id)?;
        let s = self.sheets.remove(from);
        let to = to.min(self.sheets.len());
        self.sheets.insert(to, s);
        self.reindex();
        Some(from)
    }

    /// After a sheet operation (insert, delete, move, copy, rename):
    /// recalculates the SHEET / SHEETS formulas, and the formulas whose 3-D spans now cover a different
    /// sequence of sheets (other sheets, or the same ones in another order,
    /// which NPV, IRR and the first-error order see). Spans are matched by
    /// tab position, so the dependency index needs no update; the cost is
    /// one comparison of sheet ids per spanned sheet, with no allocation
    /// per formula.
    pub fn refresh_all_precedents(&mut self) {
        // SHEET / SHEETS follow every sheet operation (cheap: a flag check).
        let tab_dep: Vec<FormulaCellId> = self
            .formulas
            .iter()
            .filter(|(_, f)| f.tab_order_dependent)
            .map(|(id, _)| id)
            .collect();
        self.pending.dirty.extend(tab_dep);
        let order: Vec<SheetId> = self.sheets.iter().map(|s| s.id).collect();
        if order == self.span_order {
            return;
        }
        let old_pos: HashMap<SheetId, usize> = self
            .span_order
            .iter()
            .enumerate()
            .map(|(i, s)| (*s, i))
            .collect();
        let slice = |pos: &dyn Fn(SheetId) -> Option<usize>, s: &Span3d| {
            let (a, b) = (pos(s.first)?, pos(s.last)?);
            Some(a.min(b)..a.max(b) + 1)
        };
        let mut dirty = Vec::new();
        for (id, f) in self.formulas.iter() {
            if f.three_d.is_none() {
                continue;
            }
            // Static spans and those built during evaluation (`:` across
            // sheets, INDIRECT).
            let changed = f.spans().iter().chain(f.dynamic_spans()).any(|s| {
                let old = slice(&|x| old_pos.get(&x).copied(), s);
                let new = slice(&|x| self.tab_of(x), s);
                match (old, new) {
                    (Some(o), Some(n)) => self.span_order[o] != order[n],
                    (None, None) => false,
                    _ => true,
                }
            });
            if changed {
                dirty.push(id);
            }
        }
        self.pending.dirty.extend(dirty);
        self.span_order = order;
    }

    // ----- cells ----------------------------------------------------------

    /// The value of a cell (the cached result for formulas).
    pub fn value(&self, addr: CellAddr) -> Value {
        match self
            .sheet(addr.sheet)
            .and_then(|s| s.get(addr.row, addr.col))
        {
            None => Value::Empty,
            Some(CellContent::Value(v)) => v.clone(),
            Some(CellContent::Formula(id)) => self
                .formulas
                .get(*id)
                .map_or(Value::Empty, |f| f.cached.clone()),
            Some(CellContent::ArrayPart(id)) => self.formulas.get(*id).map_or(Value::Empty, |f| {
                f.array_value(
                    addr.row.saturating_sub(f.addr.row),
                    addr.col.saturating_sub(f.addr.col),
                )
            }),
        }
    }

    /// The array formula anchor whose block contains `addr` (the anchor
    /// itself or a covered cell).
    pub fn array_at(&self, addr: CellAddr) -> Option<(FormulaCellId, &FormulaCell)> {
        let id = match self.sheet(addr.sheet)?.get(addr.row, addr.col)? {
            CellContent::Formula(id) | CellContent::ArrayPart(id) => *id,
            CellContent::Value(_) => return None,
        };
        self.formulas
            .get(id)
            .filter(|f| f.array.is_some())
            .map(|f| (id, f))
    }

    /// The array blocks on `sheet` intersecting `range`.
    pub fn array_blocks_in(&self, range: &CellRange) -> Vec<CellRange> {
        let mut out: Vec<CellRange> = Vec::new();
        let Some(sheet) = self.sheet(range.sheet) else {
            return out;
        };
        let c1 = range.c1.min(sheet.columns.len().saturating_sub(1) as u32);
        if sheet.columns.is_empty() {
            return out;
        }
        for c in range.c0..=c1 {
            let Some(col) = sheet.column(c) else {
                continue;
            };
            for r in col.formula_rows.range(range.r0..=range.r1) {
                if let Some(id) = col.cells.get(*r).and_then(CellContent::formula_id) {
                    if let Some(b) = self.formulas.get(id).and_then(|f| f.array_range()) {
                        if !out.contains(&b) {
                            out.push(b);
                        }
                    }
                }
            }
        }
        out
    }

    /// The formula cell at `addr`.
    pub fn formula_at(&self, addr: CellAddr) -> Option<(FormulaCellId, &FormulaCell)> {
        match self.sheet(addr.sheet)?.get(addr.row, addr.col)? {
            CellContent::Formula(id) => self.formulas.get(*id).map(|f| (*id, f)),
            CellContent::Value(_) | CellContent::ArrayPart(_) => None,
        }
    }

    /// The style of a cell.
    pub fn style_at(&self, addr: CellAddr) -> StyleId {
        self.sheet(addr.sheet)
            .map_or(StyleId::DEFAULT, |s| s.style(addr.row, addr.col))
    }

    /// Captures the content of a cell.
    pub fn snapshot_content(&self, addr: CellAddr) -> Option<SnapContent> {
        match self.sheet(addr.sheet)?.get(addr.row, addr.col)? {
            CellContent::Value(v) => Some(SnapContent::Value(v.clone())),
            CellContent::Formula(id) => self.formulas.get(*id).map(|f| SnapContent::Formula {
                formula: f.formula.clone(),
                invalid_text: f.invalid_text.clone(),
                array: f.array,
            }),
            // Covered cells of an array block are restored with their anchor.
            CellContent::ArrayPart(_) => None,
        }
    }

    /// Restores captured content (or clears with `None`).
    pub fn restore_content(&mut self, addr: CellAddr, content: Option<SnapContent>) {
        match content {
            None => self.clear_content(addr),
            Some(SnapContent::Value(v)) => self.set_value(addr, v),
            Some(SnapContent::Formula {
                formula,
                invalid_text,
                array: None,
            }) => self.set_formula(addr, formula, invalid_text),
            Some(SnapContent::Formula {
                formula,
                array: Some((rows, cols)),
                ..
            }) => self.set_array_formula(addr, formula, rows, cols),
        }
    }

    fn drop_formula(&mut self, content: Option<CellContent>) {
        if let Some(CellContent::Formula(id)) = content {
            let block = self.formulas.get(id).and_then(|f| f.array_range());
            self.deps.forget(id);
            self.formulas.remove(id);
            if let Some(b) = block {
                self.remove_array_parts(b, id);
            }
        }
    }

    /// Removes the covered cells of the array block `b` owned by `id`.
    fn remove_array_parts(&mut self, b: CellRange, id: FormulaCellId) {
        let Some(sheet) = self.sheet_mut(b.sheet) else {
            return;
        };
        let mut removed = false;
        for c in b.c0..=b.c1 {
            for r in b.r0..=b.r1 {
                if sheet.get(r, c) == Some(&CellContent::ArrayPart(id)) {
                    sheet.remove(r, c);
                    removed = true;
                }
            }
        }
        if removed {
            self.pending.ranges.push(b);
        }
    }

    /// Stores an array formula over the `rows × cols` block whose top-left is
    /// `addr`; the other cells of the block become covered cells (their
    /// previous content is dropped).
    pub fn set_array_formula(
        &mut self,
        addr: CellAddr,
        formula: Arc<Formula>,
        rows: u32,
        cols: u32,
    ) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if u64::from(addr.row) + u64::from(rows) > u64::from(MAX_ROWS)
            || u64::from(addr.col) + u64::from(cols) > u64::from(MAX_COLS)
        {
            return;
        }
        // Clear the block first (this also drops other array blocks' parts
        // overlapping it, through their anchors).
        for c in addr.col..addr.col + cols {
            for r in addr.row..addr.row + rows {
                if (r, c) != (addr.row, addr.col) {
                    self.clear_content(CellAddr::new(addr.sheet, r, c));
                }
            }
        }
        self.set_formula(addr, formula, None);
        let Some((id, _)) = self.formula_at(addr) else {
            return;
        };
        if let Some(f) = self.formulas.get_mut(id) {
            f.array = Some((rows, cols));
        }
        if let Some(sheet) = self.sheet_mut(addr.sheet) {
            for c in addr.col..addr.col + cols {
                for r in addr.row..addr.row + rows {
                    if (r, c) != (addr.row, addr.col) {
                        sheet.set(r, c, CellContent::ArrayPart(id));
                    }
                }
            }
        }
        self.pending.ranges.push(CellRange::new(
            addr.sheet,
            addr.row,
            addr.col,
            addr.row + rows - 1,
            addr.col + cols - 1,
        ));
    }

    /// Replaces the AST of the formula at `addr` in place (keeping its raw
    /// text and array block) and returns the previous content for undo.
    pub fn replace_formula_ast(
        &mut self,
        addr: CellAddr,
        formula: Arc<Formula>,
    ) -> Option<SnapContent> {
        let (id, _) = self.formula_at(addr)?;
        let old = self.snapshot_content(addr);
        let vis = is_visibility_dependent(&formula);
        if let Some(fc) = self.formulas.get_mut(id) {
            fc.formula = formula;
            fc.visibility_dependent = vis;
        }
        self.refresh_formula(id);
        self.pending.cells.push(addr);
        old
    }

    /// Removes the content of a cell.
    pub fn clear_content(&mut self, addr: CellAddr) {
        let old = self
            .sheet_mut(addr.sheet)
            .and_then(|s| s.remove(addr.row, addr.col));
        if old.is_some() {
            self.pending.cells.push(addr);
        }
        self.drop_formula(old);
    }

    /// Stores a constant (`Empty` clears).
    pub fn set_value(&mut self, addr: CellAddr, v: Value) {
        if v.is_empty() {
            self.clear_content(addr);
            return;
        }
        let v = match v {
            Value::Array(m) => m.get(0, 0).cloned().unwrap_or(Value::Empty),
            other => other,
        };
        let Some(sheet) = self.sheet_mut(addr.sheet) else {
            return;
        };
        let old = sheet.set(addr.row, addr.col, CellContent::Value(v));
        self.pending.cells.push(addr);
        self.drop_formula(old);
    }

    /// The static precedents of `f` at `addr`, 3-D spans expanded by tab
    /// order (including those of the names it uses).
    pub fn precedents_of(&self, f: &Formula, addr: CellAddr) -> Vec<CellRange> {
        let a = self.analyze(f, addr);
        let mut out = a.precedents;
        for s in &a.spans {
            out.extend(self.span_sheets(s).iter().map(|x| s.on(x.id)));
        }
        out.sort_by_key(|r| (r.sheet, r.r0, r.c0, r.r1, r.c1));
        out.dedup();
        out
    }

    /// Everything the dependency index needs about `f` at `addr`:
    /// precedents, strict precedents, name keys, whether it has 3-D refs
    /// and whether it is volatile. Names are not inlined: every reachable
    /// definition is visited once ([`crate::names::collect_names`]) and its
    /// references are resolved at `addr`.
    pub fn analyze(&self, f: &Formula, addr: CellAddr) -> Analysis {
        let (defs, keys) = if crate::names::has_names(&f.expr) {
            crate::names::collect_names(&self.names, &f.expr, addr.sheet)
        } else {
            Default::default()
        };
        let mut refs = f.static_precedents(addr);
        let mut volatile = is_volatile(f);
        for d in defs.formulas() {
            refs.extend(d.static_precedents(addr));
            volatile |= is_volatile(d);
        }
        let has_3d = refs.iter().any(|r| matches!(r, ResolvedRef::Span3d { .. }));
        let (precedents, spans) = split_refs(refs);
        let (strict, strict_spans) = match self.strict_precedents_with(&f.expr, addr, &defs) {
            Some(s) => {
                let (r, sp) = split_refs(s);
                (Some(r), sp)
            }
            None => (None, spans.clone()),
        };
        Analysis {
            precedents,
            strict,
            spans,
            strict_spans,
            keys,
            has_3d,
            volatile,
        }
    }

    /// The precedents of `f` whose values are always read (see
    /// [`FormulaCell::strict_precedents`]); `None` when that is all of them.
    pub fn strict_precedents_of(&self, f: &Formula, addr: CellAddr) -> Option<Vec<CellRange>> {
        self.analyze(f, addr).strict
    }

    fn strict_precedents_with(
        &self,
        root: &Expr,
        addr: CellAddr,
        defs: &crate::names::NameDefs,
    ) -> Option<Vec<ResolvedRef>> {
        let reg = registry();
        let mut strict = Vec::new();
        let mut excluded = false;
        // Each (name, inside a lazy argument) is walked once.
        let mut seen: HashSet<(u32, bool)> = HashSet::new();
        let plain = |a: &Expr| {
            let mut e = a;
            for _ in 0..=crate::names::MAX_NAME_DEPTH {
                match e {
                    Expr::Paren(x) => e = x,
                    Expr::Name(n) => match defs.resolve(n) {
                        Some((_, f)) => e = &f.expr,
                        None => return false,
                    },
                    other => return is_plain_ref(other),
                }
            }
            false
        };
        // (node, inside a lazy argument)
        let mut stack: Vec<(&Expr, bool)> = vec![(root, false)];
        while let Some((e, lazy)) = stack.pop() {
            match e {
                Expr::Ref(r) => {
                    if lazy {
                        excluded = true;
                    } else {
                        strict.push(r.resolve(addr));
                    }
                }
                Expr::Name(n) => {
                    if let Some((id, f)) = defs.resolve(n) {
                        if seen.insert((id, lazy)) {
                            stack.push((&f.expr, lazy));
                        }
                    }
                }
                Expr::ArrayLit(a) => stack.extend(a.items.iter().rev().map(|x| (x, lazy))),
                Expr::Call { func, args } => {
                    let def = match func {
                        FuncRef::Known(id) => reg.get(FuncId(id.0)),
                        FuncRef::Unknown(_) => None,
                    };
                    for (i, a) in args.iter().enumerate().rev() {
                        let kind = def.map(|d| d.param_kind(i));
                        let address_only = def
                            .is_some_and(|d| d.flags.contains(FnFlags::REF_ADDRESS_ONLY))
                            && kind == Some(ParamKind::Reference)
                            && plain(a);
                        if address_only {
                            excluded = true;
                            continue;
                        }
                        stack.push((a, lazy || kind == Some(ParamKind::Lazy)));
                    }
                }
                Expr::Unary(_, x) | Expr::Postfix(_, x) | Expr::Paren(x) => stack.push((x, lazy)),
                Expr::Binary(_, l, r) => {
                    stack.push((r, lazy));
                    stack.push((l, lazy));
                }
                _ => {}
            }
        }
        excluded.then_some(strict)
    }

    /// Stores a formula; the cell becomes dirty.
    pub fn set_formula(
        &mut self,
        addr: CellAddr,
        formula: Arc<Formula>,
        invalid_text: Option<Arc<str>>,
    ) {
        if self.sheet(addr.sheet).is_none() || addr.row >= MAX_ROWS || addr.col >= MAX_COLS {
            return;
        }
        let Analysis {
            precedents,
            strict: strict_precedents,
            spans,
            strict_spans,
            keys: name_keys,
            has_3d,
            volatile,
        } = self.analyze(&formula, addr);
        let has_names = !name_keys.is_empty();
        let vis_dep = is_visibility_dependent(&formula);
        let tab_dep = is_tab_order_dependent(&formula);
        let id = self.formulas.insert(FormulaCell {
            addr,
            formula,
            cached: Value::Empty,
            state: CellState::Dirty,
            volatile,
            invalid_text,
            precedents: precedents.clone(),
            strict_precedents: strict_precedents.map(Box::new),
            three_d: crate::formula_cells::SpanSet::boxed(spans.clone(), strict_spans, Vec::new()),
            dynamic_deps: Vec::new(),
            has_3d,
            tab_order_dependent: tab_dep,
            has_names,
            visibility_dependent: vis_dep,
            array: None,
            array_result: None,
        });
        self.deps.add(id, &precedents);
        self.deps.add_spans(id, &spans);
        self.deps.set_volatile(id, volatile);
        self.deps.set_names(id, &name_keys);
        let old = self
            .sheet_mut(addr.sheet)
            .and_then(|s| s.set(addr.row, addr.col, CellContent::Formula(id)));
        self.drop_formula(old);
        self.pending.cells.push(addr);
        self.pending.dirty.push(id);
    }

    /// Parses `text` (CalcA1, en-US) as a formula at `addr`.
    pub fn parse_formula(
        &self,
        text: &str,
        addr: CellAddr,
    ) -> Result<Formula, tally_formula::ParseError> {
        parse(text, addr, Dialect::calc_a1_en_us(), self)
    }

    /// Stores what the user typed: a formula (`=…`), or a value through the
    /// input recognizer, applying its implied number format to cells whose
    /// format is General. A leading `'` forces text; cells formatted `@`
    /// keep the input as text.
    pub fn set_input(&mut self, addr: CellAddr, input: &str) {
        self.set_input_opts(addr, input, true);
    }

    /// What the user typed into one cell (cell edit, formula bar; AOO
    /// `ScViewFunc::EnterData`): [`Self::set_input`], except that input
    /// starting with `+` or `-` that is not a number becomes a formula
    /// (`+B1+B2` is stored as `=+B1+B2`), as in AOO:
    /// - extra leading signs are ignored for the number test (`--5` and
    ///   `+-5` are not formulas; they go through the recognizer, which
    ///   keeps them as text), and a lone `-` / `+` (or `+-`) stays text;
    /// - numbers, percentages, exponents and dates (`-5`, `+5`, `-5%`,
    ///   `+1e3`, `-2020`) stay numbers;
    /// - a Text-formatted (`@`) cell keeps the input as text;
    /// - input that does not compile, or uses an unknown name or function
    ///   (`-abc`, `+foo(1)`), stays text (AOO compiles with extended error
    ///   detection and stores text on an error).
    ///
    /// Paste, CSV import, text to columns and Fill Selection (Alt+Enter)
    /// use [`Self::set_input`], where only `=` starts a formula (AOO
    /// `ScColumn::SetString`).
    pub fn set_typed_input(&mut self, addr: CellAddr, input: &str) {
        match self.signed_formula(addr, input) {
            Some(f) => self.set_formula(addr, Arc::new(f), None),
            None => self.set_input(addr, input),
        }
    }

    /// The formula a typed `+…` / `-…` input becomes, if any (see
    /// [`Self::set_typed_input`]).
    fn signed_formula(&self, addr: CellAddr, input: &str) -> Option<Formula> {
        let sign = input.chars().next().filter(|c| matches!(c, '+' | '-'))?;
        let rest = input[1..].trim_start_matches(['+', '-']);
        if rest.is_empty() || self.styles.get(self.style_at(addr)).is_text_format() {
            return None;
        }
        let collapsed = format!("{sign}{rest}");
        let rec = recognize_input_with(
            &collapsed,
            self.locale,
            &RecognizeContext::for_settings(&self.settings),
        );
        if matches!(rec.value, Value::Number(_)) {
            return None;
        }
        let f = self.parse_formula(&format!("={input}"), addr).ok()?;
        let mut unknown = false;
        f.expr.walk(|e| {
            if matches!(
                e,
                Expr::Name(tally_formula::NameRef::Unresolved(_))
                    | Expr::Call {
                        func: FuncRef::Unknown(_),
                        ..
                    }
            ) {
                unknown = true;
            }
        });
        (!unknown).then_some(f)
    }

    /// Like [`Self::set_input`], but `=…` stays text instead of becoming a
    /// formula (imported CSV: AOO's "Evaluate formulas" is off by default).
    pub fn set_constant_input(&mut self, addr: CellAddr, input: &str) {
        self.set_input_opts(addr, input, false);
    }

    fn set_input_opts(&mut self, addr: CellAddr, input: &str, formulas: bool) {
        self.set_input_inner(addr, input, formulas, None, None);
    }

    /// [`Self::set_input`] (`formulas`) or [`Self::set_constant_input`]
    /// for bulk loads (CSV import): `bulk` holds the recognition context
    /// (today's date, read once per load instead of once per cell) and the
    /// styles already derived by an implied number format; a text result
    /// equal to the input reuses `input`'s allocation. The result is the
    /// same as the per-cell calls.
    pub(crate) fn set_input_bulk(
        &mut self,
        addr: CellAddr,
        input: &Arc<str>,
        formulas: bool,
        bulk: &mut BulkInput,
    ) {
        self.set_input_inner(addr, input, formulas, Some(bulk), Some(input));
    }

    fn set_input_inner(
        &mut self,
        addr: CellAddr,
        input: &str,
        formulas: bool,
        mut bulk: Option<&mut BulkInput>,
        shared: Option<&Arc<str>>,
    ) {
        // `input` as a text value, sharing the caller's allocation if any.
        let as_text = || shared.map_or_else(|| Value::text(input), |a| Value::Text(Arc::clone(a)));
        if input.is_empty() {
            self.clear_content(addr);
            return;
        }
        if !formulas && input.starts_with('=') {
            self.set_value(addr, as_text());
            return;
        }
        let style = self.style_at(addr);
        // A Text-formatted (`@`) cell keeps its input as typed, formulas
        // included (AOO).
        if self.styles.get(style).is_text_format() {
            self.set_value(addr, as_text());
            return;
        }
        if input.starts_with('=') && input.len() > 1 {
            match self.parse_formula(input, addr) {
                Ok(f) => self.set_formula(addr, Arc::new(f), None),
                Err(e) => self.set_formula(
                    addr,
                    Arc::new(Formula::new(Expr::Err(e.kind))),
                    Some(input.into()),
                ),
            }
            return;
        }
        if let Some(n) = plain_number(input, self.locale) {
            // What the recognizer returns for plain decimal numbers,
            // without its allocations (no implied format).
            self.set_value(addr, Value::Number(n));
            return;
        }
        let rec = match &mut bulk {
            Some(b)
                if b.ctx.epoch == self.settings.date_epoch
                    && b.ctx.year2000 == self.settings.year2000() =>
            {
                if let Some(r) = b.recognized.get(input) {
                    r.clone()
                } else {
                    let mut r = recognize_input_with(input, self.locale, &b.ctx);
                    if matches!(&r.value, Value::Text(t) if **t == *input) {
                        r.value = as_text();
                    }
                    if input.len() <= BULK_RECOGNIZED_MAX_LEN
                        && b.recognized.len() < BULK_RECOGNIZED_CAP
                    {
                        let key = shared.map_or_else(|| input.into(), Arc::clone);
                        b.recognized.insert(key, r.clone());
                    }
                    r
                }
            }
            _ => recognize_input_with(
                input,
                self.locale,
                &RecognizeContext::for_settings(&self.settings),
            ),
        };
        self.set_value(addr, rec.value);
        if let Some(code) = rec.implied_format {
            if self.styles.get(style).is_general() {
                let new = match bulk {
                    Some(b) => *b
                        .formats
                        .entry((style, code))
                        .or_insert_with_key(|(st, code)| self.styles.with_format(*st, code)),
                    None => self.styles.with_format(style, &code),
                };
                if let Some(s) = self.sheet_mut(addr.sheet) {
                    if let Some(col) = s.column_mut(addr.col) {
                        col.styles.set(addr.row, addr.row, new);
                    }
                }
            }
        }
    }

    /// Text for the formula bar: the formula in the UI dialect, or the raw text.
    pub fn formula_text(&self, id: FormulaCellId) -> Option<String> {
        let f = self.formulas.get(id)?;
        if let Some(t) = &f.invalid_text {
            return Some(t.to_string());
        }
        Some(print(&f.formula, f.addr, self.ui_dialect(), self))
    }

    /// Marks dirty every visibility-dependent formula (SUBTOTAL…) that
    /// reads one of `sheets` (after rows were hidden, shown or filtered).
    pub fn dirty_visibility_dependents(&mut self, sheets: &[SheetId]) {
        if sheets.is_empty() {
            return;
        }
        let ids: Vec<FormulaCellId> = self
            .formulas
            .iter()
            .filter(|(_, f)| f.visibility_dependent && self.reads_sheet(f, sheets))
            .map(|(id, _)| id)
            .collect();
        for id in &ids {
            if let Some(f) = self.formulas.get_mut(*id) {
                f.state = CellState::Dirty;
                self.pending.cells.push(f.addr);
            }
        }
        self.pending.dirty.extend(ids);
    }

    /// Packs the cell storage of every sheet after a bulk load (same
    /// content, less memory; see [`crate::storage::BTreeColumn::compact`]).
    pub fn compact_storage(&mut self) {
        for s in &mut self.sheets {
            s.compact();
        }
    }

    /// Marks every formula dirty (hard recalc).
    pub fn dirty_all(&mut self) {
        let ids: Vec<FormulaCellId> = self.formulas.keys().collect();
        for id in &ids {
            if let Some(f) = self.formulas.get_mut(*id) {
                f.state = CellState::Dirty;
            }
        }
        self.pending.dirty.extend(ids);
    }

    /// Number of non-empty cells in the workbook.
    pub fn cell_count(&self) -> usize {
        self.sheets.iter().map(Sheet::cell_count).sum()
    }
}

/// A reference argument, possibly in parentheses.
/// What [`Workbook::analyze`] finds out about a formula.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Static single-sheet precedents (names included).
    pub precedents: Vec<CellRange>,
    /// The always-read subset, `None` when that is all of them.
    pub strict: Option<Vec<CellRange>>,
    /// Static 3-D precedents, unexpanded.
    pub spans: Vec<Span3d>,
    /// The always-read 3-D precedents.
    pub strict_spans: Vec<Span3d>,
    /// Dependency keys of the names used (directly or through names).
    pub keys: Vec<String>,
    /// Contains a 3-D reference.
    pub has_3d: bool,
    /// Calls a volatile function (directly or through names).
    pub volatile: bool,
}

/// Single-sheet precedents (sorted, deduplicated) and 3-D spans
/// (deduplicated) of resolved references.
fn split_refs(refs: Vec<ResolvedRef>) -> (Vec<CellRange>, Vec<Span3d>) {
    let mut out = Vec::new();
    let mut spans: Vec<Span3d> = Vec::new();
    for r in refs {
        match r {
            ResolvedRef::Range(cr) => out.push(cr),
            ResolvedRef::Span3d { first, last, area } => {
                let s = Span3d {
                    first,
                    last,
                    rect: area,
                };
                if !spans.contains(&s) {
                    spans.push(s);
                }
            }
            ResolvedRef::Invalid => {}
        }
    }
    out.sort_by_key(|r| (r.sheet, r.r0, r.c0, r.r1, r.c1));
    out.dedup();
    (out, spans)
}

fn is_plain_ref(e: &Expr) -> bool {
    match e {
        Expr::Ref(_) => true,
        Expr::Paren(x) => is_plain_ref(x),
        _ => false,
    }
}

/// `true` if the formula calls a function that skips hidden or filtered
/// rows (SUBTOTAL, AGGREGATE).
pub fn is_visibility_dependent(f: &Formula) -> bool {
    calls_any(f, &["SUBTOTAL", "AGGREGATE"])
}

/// `true` if the formula calls SHEET or SHEETS (results that follow the
/// tab order).
pub fn is_tab_order_dependent(f: &Formula) -> bool {
    calls_any(f, &["SHEET", "SHEETS"])
}

fn calls_any(f: &Formula, names: &[&str]) -> bool {
    let reg = registry();
    let mut v = false;
    f.expr.walk(|e| {
        if let Expr::Call {
            func: FuncRef::Known(id),
            ..
        } = e
        {
            if reg
                .get(FuncId(id.0))
                .is_some_and(|d| names.contains(&d.name))
            {
                v = true;
            }
        }
    });
    v
}

/// `true` if the formula calls a volatile function.
pub fn is_volatile(f: &Formula) -> bool {
    let reg = registry();
    let mut v = false;
    f.expr.walk(|e| {
        if let Expr::Call {
            func: FuncRef::Known(id),
            ..
        } = e
        {
            if reg
                .get(FuncId(id.0))
                .is_some_and(|d| d.flags.contains(FnFlags::VOLATILE))
            {
                v = true;
            }
        }
    });
    v
}

/// The workbook as a resolver for OpenFormula text read from files: ODF
/// stored function names win (`DURATION`, `CONVERT` and `WEEKNUM` are the
/// add-ins there, `ISOWEEKNUM` the built-in WEEKNUM), see
/// `Registry::lookup_odf_first`.
pub struct OdfResolver<'a>(pub &'a Workbook);

impl Resolver for OdfResolver<'_> {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.0.sheet_by_name(name).or_else(|| {
            if self.0.load_aliases.is_empty() {
                None
            } else {
                self.0.load_aliases.get(&name.to_lowercase()).copied()
            }
        })
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.0.sheet_name(id)
    }

    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        self.0.name(name, sheet)
    }

    fn name_text(&self, id: NameId) -> Option<Arc<str>> {
        self.0.name_text(id)
    }

    fn function(&self, name: &str) -> Option<AstFuncId> {
        registry().lookup_odf_first(name).map(|id| AstFuncId(id.0))
    }

    fn function_name(&self, id: AstFuncId) -> Option<Arc<str>> {
        self.0.function_name(id)
    }

    fn function_name_in(&self, id: AstFuncId, dialect: Dialect) -> Option<Arc<str>> {
        self.0.function_name_in(id, dialect)
    }
}

impl Resolver for Workbook {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.sheet_by_name_ci(name)
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.sheet(id).map(|s| s.name.clone())
    }

    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        self.names.visible(name, sheet)
    }

    fn name_text(&self, id: NameId) -> Option<Arc<str>> {
        self.names.get(id).map(|e| e.name.clone())
    }

    fn function(&self, name: &str) -> Option<AstFuncId> {
        registry().lookup_any(name).map(|id| AstFuncId(id.0))
    }

    fn function_name(&self, id: AstFuncId) -> Option<Arc<str>> {
        registry().get(FuncId(id.0)).map(|d| Arc::from(d.name))
    }

    fn function_name_in(&self, id: AstFuncId, dialect: Dialect) -> Option<Arc<str>> {
        let d = registry().get(FuncId(id.0))?;
        Some(Arc::from(if dialect.is_open_formula() {
            d.odf_name
        } else {
            d.name
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namer_is_unique_and_case_insensitive() {
        let mut n = SheetNamer::new(["Data", "data_2"]);
        assert_eq!(n.claim("DATA"), "DATA_3");
        assert_eq!(n.claim("Data"), "Data_4");
        assert_eq!(n.claim("Other"), "Other");
        let mut n = SheetNamer::new(["Sheet1", "sheet2"]);
        assert_eq!(n.claim_numbered(1), "Sheet3");
        assert_eq!(n.claim_numbered(2), "Sheet4");
        assert_eq!(n.claim_numbered(10), "Sheet10");
        assert_eq!(n.claim("Sheet10"), "Sheet10_2");
    }

    #[test]
    fn sheets_and_names() {
        let mut wb = Workbook::new();
        assert_eq!(wb.sheets.len(), 1);
        let s2 = wb.add_sheet(None, None);
        assert_eq!(
            wb.sheet(s2).map(|s| s.name.to_string()),
            Some("Sheet2".into())
        );
        let s3 = wb.add_sheet(Some("sheet2"), Some(0));
        assert_eq!(
            wb.sheet(s3).map(|s| s.name.to_string()),
            Some("sheet2_2".into())
        );
        assert_eq!(wb.tab_of(s3), Some(0));
        assert_eq!(wb.move_sheet(s3, 5), Some(0));
        assert_eq!(wb.tab_of(s3), Some(2));
        assert!(wb.remove_sheet(s2).is_some());
        let s4 = wb.add_sheet(None, None);
        assert!(s4 > s3, "ids are never reused");
    }

    #[test]
    fn rng_in_unit_interval() {
        let mut r = Rng::new(42);
        for _ in 0..1000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
