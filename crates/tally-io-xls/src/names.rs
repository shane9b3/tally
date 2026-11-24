//! Defined names of a BIFF8 workbook: `NAME` records with the
//! `EXTERNSHEET` / `SUPBOOK` tables their 3-D references go through.
//!
//! The formatting pass collects the records ([`NameRecs::name`],
//! [`NameRecs::externsheet`], [`NameRecs::supbook`]); once the sheet names
//! are known, [`NameRecs::resolve`] turns each name into a
//! [`NameData`]. A name's formula is decoded from its parsed expression
//! (`rgce`, reverse Polish) into Excel text, which the XLSX reader's
//! defined-name conversion turns into the model's OpenFormula.
//!
//! Read are the formulas made of references to this workbook's sheets
//! (`ptgRef3d`, `ptgArea3d`), numbers, text, booleans, other names, the
//! operators (union, intersection and range included), function calls and
//! array constants. A function is a built-in one by its index (`ptgFunc`,
//! `ptgFuncVar`, the table in `ftab.rs`; `tAttrSum` is `SUM` with one
//! argument), or one called by name (`ptgFuncVar` with index 255, whose
//! first operand names it): an add-in function (`ptgNameX` to an
//! `EXTERNNAME` of the add-in `SUPBOOK`) or a function newer than the
//! format (`ptgName` to a hidden function `NAME` spelled `_xlfn.…`). The
//! text carries Excel's function names, and the conversion decides
//! whether Tally has the function, as for an `.xlsx`. The tokens are a
//! tree: operands get the parentheses the text needs to be read as that
//! tree. A name used in a formula is a record; [`NameRecs::resolve`]
//! sees to it that its spelling means that record in the model, or
//! leaves the name out. A name whose
//! formula holds anything else (a function that is not in the table or
//! that Tally does not have, a macro function, an error, an empty or
//! error element in an array, a reference to another workbook or to a
//! deleted sheet, a reference without a sheet) is left out and counted.
//! Built-in names (print ranges, filter ranges), function and macro names
//! are not names of the model (the print ranges are read by the
//! formatting pass itself).
//!
//! Hostile input: every length is checked against the record, a name
//! keeps at most [`MAX_FORMULA_TEXT`] bytes of formula text, the decoder
//! is a loop over the tokens with at most [`MAX_STACK`] pending operands
//! (a function takes its arguments from them or the name is left out),
//! the names kept and their bytes are bounded by [`Limits`], and the text
//! moved while decoding is bounded by [`WORK_BUDGET`] for the workbook, so
//! deeply nested expressions cannot make the pass quadratic.

use std::collections::{HashMap, HashSet};

use tally_model::NameData;
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::ftab;
use crate::prescan::{u16_at, u32_at};
use crate::Limits;

/// Longest Excel text of one name's formula.
const MAX_FORMULA_TEXT: usize = 8 * 1024;
/// Most operands waiting for their operator or function while one formula
/// is decoded (a call has at most 30 arguments in a file Excel wrote).
const MAX_STACK: usize = 1024;
/// Most bytes of text moved while decoding the names of a workbook.
const WORK_BUDGET: u64 = 64 * 1024 * 1024;
/// The formula of a name that stands for one that is not imported (see
/// [`NameRecs::resolve`]).
const NO_VALUE: &str = "of:=#NAME?";
/// Most (sheet, used name) pairs looked at for the names of a workbook
/// while a base sheet is searched beyond the usual ones.
const SCAN_BUDGET: u64 = 4 * 1024 * 1024;
/// Most `SUPBOOK` records looked at.
const MAX_SUPBOOKS: usize = 65_536;
/// Most `EXTERNSHEET` entries looked at (the index is 16 bits wide).
const MAX_XTIS: usize = 65_536;

/// One `NAME` record.
#[derive(Debug, Default)]
struct RawName {
    /// The name; empty for a record that is not a name of the model (a
    /// built-in, a function, a macro, a truncated record). Such a record
    /// still takes its index.
    name: String,
    /// The sheet the name is local to (1-based); 0 = the workbook.
    itab: u16,
    /// The parsed expression; `None` when the record does not hold all of
    /// it.
    rgce: Option<Vec<u8>>,
    /// What follows the expression in the record: the values of its array
    /// constants.
    extra: Vec<u8>,
    /// For a hidden function name spelled `_xlfn.…` (a function newer
    /// than the format, called by name): that spelling. Such a record is
    /// not a name of the model.
    func: Option<String>,
}

/// What a `SUPBOOK` stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Book {
    /// The workbook itself.
    Own,
    /// The add-in functions; its `EXTERNNAME` records name them.
    AddIn,
    /// Another workbook, a DDE or OLE link.
    Other,
}

/// An `EXTERNSHEET` entry: the `SUPBOOK` and the sheets it stands for.
#[derive(Clone, Copy, Debug)]
struct Xti {
    book: u16,
    first: u16,
    last: u16,
}

/// The name records of a workbook.
#[derive(Debug, Default)]
pub(crate) struct NameRecs {
    names: Vec<RawName>,
    /// Bytes of names and expressions kept.
    bytes: u64,
    xtis: Vec<Xti>,
    /// Per `SUPBOOK`: what it is, and for the add-in one the function
    /// names of its `EXTERNNAME` records (empty for a record that does
    /// not name a function; it still takes its index).
    books: Vec<(Book, Vec<String>)>,
    /// The last `SUPBOOK` is kept and is the add-in one.
    addin_open: bool,
    /// `EXTERNNAME` records kept.
    extern_names: usize,
    /// A limit cut the list.
    pub dropped: bool,
}

impl NameRecs {
    /// Notes a `SUPBOOK` record: `ctab`, then 0x0401 for the workbook
    /// itself or 0x3A01 for the add-in functions.
    pub fn supbook(&mut self, d: &[u8]) {
        self.addin_open = false;
        if self.books.len() < MAX_SUPBOOKS {
            let book = match (d.len(), u16_at(d, 2)) {
                (4, Some(0x0401)) => Book::Own,
                (4, Some(0x3A01)) => Book::AddIn,
                _ => Book::Other,
            };
            self.addin_open = book == Book::AddIn;
            self.books.push((book, Vec::new()));
        }
    }

    /// Notes an `EXTERNNAME` record of the add-in `SUPBOOK`: flags, four
    /// bytes not used, the function's name. The records of other
    /// `SUPBOOK`s (names in other workbooks, DDE and OLE links) are not
    /// looked at.
    pub fn externname(&mut self, d: &[u8], limits: &Limits) {
        if !self.addin_open {
            return;
        }
        let Some((_, list)) = self.books.last_mut() else {
            return;
        };
        if self.extern_names >= limits.max_names {
            self.dropped = true;
            self.addin_open = false;
            return;
        }
        self.extern_names += 1;
        // A record that is not a function name keeps its index.
        let mut name = extern_function(d).unwrap_or_default();
        if self.bytes.saturating_add(name.len() as u64) > limits.max_name_bytes {
            self.dropped = true;
            name.clear();
        }
        self.bytes = self.bytes.saturating_add(name.len() as u64);
        list.push(name);
    }

    /// Reads an `EXTERNSHEET` record (with its continues in `d`): a count,
    /// then (SUPBOOK, first sheet, last sheet) each. A later record
    /// replaces an earlier one.
    pub fn externsheet(&mut self, d: &[u8]) {
        let n = usize::from(u16_at(d, 0).unwrap_or(0))
            .min(d.len().saturating_sub(2) / 6)
            .min(MAX_XTIS);
        self.xtis.clear();
        for k in 0..n {
            let at = 2 + 6 * k;
            if let (Some(book), Some(first), Some(last)) =
                (u16_at(d, at), u16_at(d, at + 2), u16_at(d, at + 4))
            {
                self.xtis.push(Xti { book, first, last });
            }
        }
    }

    /// Notes a `NAME` record: flags, shortcut key, the name's length, the
    /// expression's length, a reserved word, the sheet, four lengths of
    /// texts Excel no longer writes, the name, the expression.
    pub fn name(&mut self, d: &[u8], limits: &Limits) {
        if self.names.len() >= limits.max_names {
            self.dropped = true;
            return;
        }
        let raw = raw_name(d).unwrap_or_default();
        let bytes = (raw.name.len()
            + raw.rgce.as_ref().map_or(0, Vec::len)
            + raw.extra.len()
            + raw.func.as_ref().map_or(0, String::len)) as u64;
        if self.bytes.saturating_add(bytes) > limits.max_name_bytes {
            // The record keeps its index (other names may refer to it).
            self.dropped = true;
            self.names.push(RawName::default());
            return;
        }
        self.bytes += bytes;
        self.names.push(raw);
    }

    /// The names of the model, in file order; how many were left out
    /// because their formula is not one this reader decodes or because
    /// they are not names Tally accepts; and whether names were left out
    /// because the names and formulas kept reached
    /// [`Limits::max_name_bytes`]. `sheets` are the workbook's sheet names
    /// in `BOUNDSHEET` order.
    ///
    /// A formula names another name by its record; the model's formula
    /// text names it by its spelling. The engine reads that spelling when
    /// it loads the name, from the sheet of the name's base cell: that
    /// sheet's own name if one is defined by then (the names load in this
    /// order), else the workbook's; a spelling that is not defined yet is
    /// looked up again from each cell that uses the name. So that the
    /// text means the record wherever the name is used:
    /// - a name that uses an earlier name of a sheet gets its base cell
    ///   on that sheet (`NameData::base_cell`; the references of a name
    ///   all name their sheet, so nothing else depends on it);
    /// - a name that uses an earlier name of the workbook gets its base
    ///   cell on a sheet without a name of that spelling;
    /// - a name that uses a later name (or itself) is read from the cells
    ///   that use it: the later name must be of the same sheet, or of the
    ///   workbook with no namesake on a sheet the user is visible from;
    /// - a record that is not the first of its spelling and scope is not
    ///   the one that is defined.
    ///
    /// A name for which no base sheet does all of that is left out. And a
    /// name of a sheet that is left out while the workbook has a name of
    /// the same spelling is kept as a name without a value (`#NAME?`), so
    /// that the cells of that sheet which use it (their formulas are
    /// text) do not take the workbook's instead.
    pub fn resolve(&self, sheets: &[String], limits: &Limits) -> (Vec<NameData>, usize, bool) {
        // Per `NAME` record: the name it becomes and the records its
        // formula uses.
        let mut found: Vec<Option<(NameData, Vec<usize>)>> = Vec::new();
        found.resize_with(self.names.len(), || None);
        let mut skipped = 0usize;
        let mut cut = false;
        let mut kept_bytes = 0u64;
        let mut work = WORK_BUDGET;
        let converter = tally_io_xlsx::ExcelNameConverter::new(sheets);
        // A name Tally would read as something else in a formula (a cell
        // of the BIFF8 sheet such as `A1`, `TRUE`, an R1C1 reference: none
        // of them a name Excel allows) is not imported, and neither is a
        // name that uses it: written into formula text it would mean that
        // other thing. A name spelled like a cell outside the BIFF8 sheet
        // (`TAX2024`: legal in Excel 97-2003, a cell in the larger grid)
        // is one: the conversion reads it as a name, and the engine keeps
        // it under another spelling with this one as an alias.
        //
        // For a hidden function name (`_xlfn.…`): it is a function Excel
        // 2007 added, or one an `.xlsx` formula spells with that prefix.
        // Not `_xlfn.SUM`, and not a function only Tally has.
        let valid: Vec<bool> = self
            .names
            .iter()
            .map(|n| match &n.func {
                Some(f) => {
                    f.get(6..).is_some_and(ftab::is_future) || converter.is_prefixed_function(f)
                }
                None => !n.name.is_empty() && converter.is_name(&n.name),
            })
            .collect();
        for (slot, (raw, ok)) in found.iter_mut().zip(self.names.iter().zip(&valid)) {
            if raw.name.is_empty() {
                continue;
            }
            if !ok {
                skipped += 1;
                continue;
            }
            let sheet = match raw.itab {
                0 => None,
                i if usize::from(i) <= sheets.len() => Some(u32::from(i) - 1),
                // Local to a sheet that is not there.
                _ => {
                    skipped += 1;
                    continue;
                }
            };
            // No formula: an empty name, or one the pre-scan found damaged
            // and emptied (it has its own warning). Not a name to import.
            let Some(rgce) = raw.rgce.as_deref() else {
                continue;
            };
            // Relative references are offsets from the cell that uses the
            // name. They are written for a formula in A1 when none points
            // up or left of it, else for one in the middle of the BIFF
            // sheet, from where every offset has a cell.
            let mut base = (0u32, 0u32);
            let formula = Tokens {
                rgce,
                extra: &raw.extra,
            };
            let mut decoded = self.excel_text(formula, sheets, &valid, base, &mut work);
            if decoded.as_ref().is_some_and(|d| d.outside) {
                base = REBASE;
                decoded = self.excel_text(formula, sheets, &valid, base, &mut work);
            }
            let formula = decoded.and_then(|d| {
                let of = converter.to_of_at(&d.text, sheet.map(|s| s as usize), base)?;
                Some((of, d.uses))
            });
            match formula {
                Some((formula, uses)) => {
                    // The text kept for the workbook is bounded: a small
                    // file can spell a long name many times in many
                    // formulas.
                    kept_bytes = kept_bytes
                        .saturating_add(raw.name.len() as u64)
                        .saturating_add(formula.len() as u64);
                    if kept_bytes > limits.max_name_bytes {
                        cut = true;
                        continue;
                    }
                    let name = NameData {
                        name: raw.name.clone(),
                        sheet,
                        formula,
                        base_cell: (base != (0, 0)).then(|| (sheet.unwrap_or(0), base.0, base.1)),
                        usage: Default::default(),
                    };
                    *slot = Some((name, uses));
                }
                None => skipped += 1,
            }
        }
        // What the spelling of a name means (see above). The names a
        // sheet may have of its own: the ones read, and the ones that may
        // be kept without a value.
        let key = |n: &NameData| (n.name.to_lowercase(), n.sheet);
        let mut first: HashMap<(String, Option<u32>), usize> = HashMap::new();
        for (i, f) in found.iter().enumerate() {
            if let Some((n, _)) = f {
                first.entry(key(n)).or_insert(i);
            }
        }
        let mut on_sheet: HashSet<(String, u32)> = HashSet::new();
        let mut on_any: HashSet<String> = HashSet::new();
        for (raw, ok) in self.names.iter().zip(&valid) {
            let s = usize::from(raw.itab);
            if *ok && !raw.name.is_empty() && (1..=sheets.len()).contains(&s) {
                on_sheet.insert((raw.name.to_lowercase(), s as u32 - 1));
                on_any.insert(raw.name.to_lowercase());
            }
        }
        let scan = std::cell::Cell::new(SCAN_BUDGET);
        // The sheet for the base cell of the name of record `x`, on which
        // every name it uses means its record; `None` when there is none.
        let base_sheet = |x: usize, n: &NameData, uses: &[usize]| -> Option<u32> {
            let mut forced: Option<u32> = None;
            for &u in uses {
                // A record that is not imported is dealt with below.
                let Some(Some((t, _))) = found.get(u) else {
                    continue;
                };
                if first.get(&key(t)) != Some(&u) {
                    return None;
                }
                match t.sheet {
                    // An earlier name of a sheet: read from that sheet.
                    Some(s) if u < x => {
                        if forced.is_some_and(|f| f != s) {
                            return None;
                        }
                        forced = Some(s);
                    }
                    // A later one: read from the cells that use the name.
                    Some(s) if n.sheet != Some(s) => return None,
                    Some(_) => {}
                    None if u < x => {}
                    None => {
                        let spelling = t.name.to_lowercase();
                        let shadowed = match n.sheet {
                            Some(s) => on_sheet.contains(&(spelling, s)),
                            None => on_any.contains(&spelling),
                        };
                        if shadowed {
                            return None;
                        }
                    }
                }
            }
            // The earlier names of the workbook: not behind a name of the
            // base sheet.
            let clear = |b: u32| {
                uses.iter().all(|&u| match found.get(u) {
                    Some(Some((t, _))) if t.sheet.is_none() && u < x => {
                        !on_sheet.contains(&(t.name.to_lowercase(), b))
                    }
                    _ => true,
                })
            };
            let usual = n.base_cell.map_or(0, |b| b.0);
            match forced {
                Some(b) => clear(b).then_some(b),
                // The sheet it would have anyway, its own, the first; else
                // any sheet, while the workbook's budget for looking lasts.
                None => [usual, n.sheet.unwrap_or(0), 0]
                    .into_iter()
                    .find(|b| clear(*b))
                    .or_else(|| {
                        (0..sheets.len() as u32).find(|b| {
                            let left = scan.get().checked_sub(uses.len() as u64);
                            scan.set(left.unwrap_or(0));
                            left.is_some() && clear(*b)
                        })
                    }),
            }
        };
        let bases: Vec<Option<Option<u32>>> = found
            .iter()
            .enumerate()
            .map(|(x, f)| f.as_ref().map(|(n, uses)| base_sheet(x, n, uses)))
            .collect();
        for (slot, base) in found.iter_mut().zip(bases) {
            match (slot.as_mut(), base) {
                (Some((n, _)), Some(Some(b))) => {
                    let (_, row, col) = n.base_cell.unwrap_or((0, 0, 0));
                    n.base_cell = ((b, row, col) != (0, 0, 0)).then_some((b, row, col));
                }
                (Some(_), Some(None)) => {
                    *slot = None;
                    skipped += 1;
                }
                _ => {}
            }
        }
        // A name that uses a name that is not imported would be a formula
        // with an unknown name in it: it is left out too, and so on along
        // the chain (each record is visited once).
        let mut users: Vec<Vec<u32>> = vec![Vec::new(); found.len()];
        for (i, f) in found.iter().enumerate() {
            for &u in f.iter().flat_map(|f| &f.1) {
                if let Some(list) = users.get_mut(u) {
                    list.push(i as u32);
                }
            }
        }
        let mut gone: Vec<usize> = (0..found.len()).filter(|i| found[*i].is_none()).collect();
        while let Some(i) = gone.pop() {
            for user in std::mem::take(&mut users[i]) {
                if found[user as usize].take().is_some() {
                    skipped += 1;
                    gone.push(user as usize);
                }
            }
        }
        // A sheet's name that is not imported, where the workbook has a
        // name of the same spelling: without a name of the sheet, the
        // cells of that sheet which use it (their formulas are text)
        // would take the workbook's value. It stays a name of the sheet
        // whose value is `#NAME?`.
        let mut global: HashSet<String> = HashSet::new();
        let mut kept: HashSet<(String, u32)> = HashSet::new();
        for (n, _) in found.iter().flatten() {
            match n.sheet {
                Some(s) => kept.insert((n.name.to_lowercase(), s)),
                None => global.insert(n.name.to_lowercase()),
            };
        }
        for (slot, (raw, ok)) in found.iter_mut().zip(self.names.iter().zip(&valid)) {
            let s = u32::from(raw.itab);
            if slot.is_some() || !ok || raw.name.is_empty() || s == 0 || s as usize > sheets.len() {
                continue;
            }
            let spelling = raw.name.to_lowercase();
            if global.contains(&spelling) && kept.insert((spelling, s - 1)) {
                let name = NameData {
                    name: raw.name.clone(),
                    sheet: Some(s - 1),
                    formula: NO_VALUE.to_string(),
                    base_cell: None,
                    usage: Default::default(),
                };
                *slot = Some((name, Vec::new()));
            }
        }
        let names = found.into_iter().flatten().map(|f| f.0).collect();
        (names, skipped, cut)
    }

    /// The sheet of an `EXTERNSHEET` index, quoted for Excel text: one
    /// sheet of this workbook.
    fn sheet_ref(&self, ixti: u16, sheets: &[String]) -> Option<String> {
        let x = self.xtis.get(usize::from(ixti))?;
        if self.books.get(usize::from(x.book))?.0 != Book::Own || x.first != x.last {
            return None;
        }
        let name = sheets.get(usize::from(x.first))?;
        // A control character cannot be written in a formula.
        if name.is_empty() || name.chars().any(char::is_control) {
            return None;
        }
        Some(format!("'{}'!", name.replace('\'', "''")))
    }

    /// The function name a `ptgName` or `ptgNameX` stands for when it is
    /// the first operand of a call by name: `book` is the `SUPBOOK` (the
    /// workbook itself for a `ptgName`), `index` the 0-based record.
    fn function_name(&self, book: usize, index: usize, valid: &[bool]) -> Option<&str> {
        match self.books.get(book)? {
            (Book::Own, _) if valid.get(index) != Some(&true) => None,
            (Book::Own, _) => self.names.get(index)?.func.as_deref(),
            (Book::AddIn, list) => list.get(index).map(String::as_str),
            (Book::Other, _) => None,
        }
        .filter(|n| !n.is_empty())
    }

    /// Decodes the reverse Polish expression of `formula` into Excel
    /// formula text for a formula in the cell `base` (row, column); `None`
    /// for an expression with a token this reader does not decode, a
    /// malformed one, one that uses a name that is not one (`valid`, per
    /// `NAME` record), or one past the text, operand or work limits.
    fn excel_text(
        &self,
        formula: Tokens,
        sheets: &[String],
        valid: &[bool],
        base: (u32, u32),
        work: &mut u64,
    ) -> Option<Decoded> {
        let Tokens { rgce, extra } = formula;
        let mut stack: Vec<Item> = Vec::new();
        let mut outside = false;
        let mut uses: Vec<usize> = Vec::new();
        // Where the next array constant's values are in `extra`; `None`
        // once that is not known any more.
        let mut next_extra = Some(0usize);
        // Charges `n` bytes of moved text to the workbook's budget.
        // Once it is spent it stays spent.
        let mut charge = |n: usize| -> Option<()> {
            let left = work.checked_sub(n as u64);
            *work = left.unwrap_or(0);
            left.map(|_| ())
        };
        let mut p = 0usize;
        while let Some(&ptg) = rgce.get(p) {
            // Operand tokens come in three classes (reference, value,
            // array) that only differ in bits 5 and 6.
            let id = if ptg >= 0x20 {
                (ptg & 0x1F) | 0x20
            } else {
                ptg
            };
            let step = match id {
                0x03..=0x0F | 0x11 => {
                    let (op, prec) = match id {
                        0x03 => ("+", ADD),
                        0x04 => ("-", ADD),
                        0x05 => ("*", MUL),
                        0x06 => ("/", MUL),
                        0x07 => ("^", POW),
                        0x08 => ("&", CONCAT),
                        0x09 => ("<", COMPARE),
                        0x0A => ("<=", COMPARE),
                        0x0B => ("=", COMPARE),
                        0x0C => (">=", COMPARE),
                        0x0D => (">", COMPARE),
                        0x0E => ("<>", COMPARE),
                        0x0F => (" ", REFERENCE),
                        _ => (":", REFERENCE),
                    };
                    charge(binary(&mut stack, op, prec)?)?;
                    1
                }
                // Union: Excel text wants it in parentheses.
                0x10 => {
                    charge(binary(&mut stack, ",", REFERENCE)?)?;
                    charge(wrap(stack.last_mut()?))?;
                    1
                }
                // Unary plus and minus, percent, parentheses. An operand
                // that binds less than the operator gets parentheses, so
                // the text means what the tokens do.
                0x12..=0x15 => {
                    let a = stack.last_mut()?;
                    if a.kind != Kind::Value {
                        return None;
                    }
                    let own = match id {
                        0x12 | 0x13 => PREFIX,
                        0x14 => POSTFIX,
                        _ => ATOM,
                    };
                    // The file's own parentheses are kept as they are.
                    if a.prec < own || id == 0x15 {
                        charge(wrap(a))?;
                    }
                    if id != 0x15 {
                        charge(a.text.len())?;
                    }
                    match id {
                        0x12 => a.text.insert(0, '+'),
                        0x13 => a.text.insert(0, '-'),
                        0x14 => a.text.push('%'),
                        _ => {}
                    }
                    a.prec = own;
                    1
                }
                // An argument that is left out: `IF(A,,B)`.
                0x16 => {
                    stack.push(Item {
                        text: String::new(),
                        kind: Kind::Missing,
                        prec: ATOM,
                    });
                    1
                }
                // Text: a length, option flags, the characters.
                0x17 => {
                    let cch = usize::from(*rgce.get(p + 1)?);
                    let wide = rgce.get(p + 2)? & 1 != 0;
                    let len = if wide { 2 * cch } else { cch };
                    let chars = rgce.get(p + 3..(p + 3).checked_add(len)?)?;
                    stack.push(Item::value(quoted(chars, wide)?));
                    3 + len
                }
                // Attributes: a type and a word. Spaces and "volatile"
                // change nothing; `IF` and `CHOOSE` come with jumps for
                // Excel's evaluation (the call itself follows as a
                // function token), `CHOOSE` with a table of them after
                // the word; "sum" is `SUM` with one argument.
                0x19 => {
                    let kind = *rgce.get(p + 1)?;
                    let word = usize::from(u16_at(rgce, p + 2)?);
                    match kind {
                        0x01 | 0x02 | 0x08 | 0x40 | 0x41 => 4,
                        0x04 => word.checked_add(1)?.checked_mul(2)?.checked_add(4)?,
                        0x10 => {
                            charge(call(&mut stack, "SUM", 1)?)?;
                            4
                        }
                        _ => return None,
                    }
                }
                0x1D => {
                    stack.push(Item::value(
                        if *rgce.get(p + 1)? != 0 {
                            "TRUE"
                        } else {
                            "FALSE"
                        }
                        .to_string(),
                    ));
                    2
                }
                0x1E => {
                    stack.push(Item::value(u16_at(rgce, p + 1)?.to_string()));
                    3
                }
                0x1F => {
                    // A negative number is a number behind a minus sign.
                    let text = number(rgce.get(p + 1..p + 9)?)?;
                    let prec = if text.starts_with('-') { PREFIX } else { ATOM };
                    stack.push(Item {
                        text,
                        kind: Kind::Value,
                        prec,
                    });
                    9
                }
                // An array constant: seven bytes not used; its values
                // follow the expression.
                0x20 => {
                    rgce.get(p + 1..p + 8)?;
                    let text = array(extra, &mut next_extra)?;
                    charge(text.len())?;
                    stack.push(Item::value(text));
                    8
                }
                // A built-in function with a fixed number of arguments.
                0x21 => {
                    let (name, min, max) = ftab::function(u16_at(rgce, p + 1)?)?;
                    if min != max {
                        return None;
                    }
                    charge(call(&mut stack, name, usize::from(min))?)?;
                    3
                }
                // A function with its number of arguments (bit 7: a
                // prompt, not part of the count) and its index (bit 15: a
                // macro command, not a function).
                0x22 => {
                    let argc = usize::from(rgce.get(p + 1)? & 0x7F);
                    let iftab = u16_at(rgce, p + 2)?;
                    if iftab == 255 {
                        // A call by name: the first operand is the name.
                        let at = stack.len().checked_sub(argc)?;
                        let first = stack.get(at)?;
                        if first.kind != Kind::Func {
                            return None;
                        }
                        let name = first.text.clone();
                        charge(call(&mut stack, &name, argc.checked_sub(1)?)?)?;
                        let result = stack.pop()?;
                        *stack.get_mut(at)? = result;
                    } else {
                        let (name, min, max) = ftab::function(iftab)?;
                        if !(usize::from(min)..=usize::from(max)).contains(&argc) {
                            return None;
                        }
                        charge(call(&mut stack, name, argc)?)?;
                    }
                    4
                }
                // Another name, by its 1-based index; or the name of a
                // function for a call by name.
                0x23 => {
                    let i = usize::try_from(u32_at(rgce, p + 1)?).ok()?.checked_sub(1)?;
                    let raw = self.names.get(i)?;
                    if valid.get(i) != Some(&true) {
                        return None;
                    }
                    if let Some(f) = &raw.func {
                        charge(f.len())?;
                        stack.push(Item {
                            text: f.clone(),
                            kind: Kind::Func,
                            prec: ATOM,
                        });
                    } else {
                        charge(raw.name.len())?;
                        stack.push(Item::value(raw.name.clone()));
                        uses.push(i);
                    }
                    5
                }
                // ptgMemArea, ptgMemErr, ptgMemNoMem and ptgMemFunc only
                // announce the reference expression that follows.
                // ptgMemArea has its rectangles (a count, eight bytes
                // each) where the array constants have their values.
                0x26 => {
                    next_extra = next_extra.and_then(|at| {
                        let n = usize::from(u16_at(extra, at)?);
                        let end = at.checked_add(2)?.checked_add(n.checked_mul(8)?)?;
                        (end <= extra.len()).then_some(end)
                    });
                    7
                }
                0x27 | 0x28 => 7,
                0x29 => 3,
                // ptgNameX: an `EXTERNSHEET` index and a 1-based index
                // into the records of its `SUPBOOK`. Read for the name of
                // a function only.
                0x39 => {
                    let x = self.xtis.get(usize::from(u16_at(rgce, p + 1)?))?;
                    let i = usize::try_from(u32_at(rgce, p + 3)?).ok()?.checked_sub(1)?;
                    let name = self.function_name(usize::from(x.book), i, valid)?;
                    charge(name.len())?;
                    stack.push(Item {
                        text: name.to_string(),
                        kind: Kind::Func,
                        prec: ATOM,
                    });
                    7
                }
                // ptgRef3d: sheet, row, column.
                0x3A => {
                    let sheet = self.sheet_ref(u16_at(rgce, p + 1)?, sheets)?;
                    let (row, col) = (u16_at(rgce, p + 3)?, u16_at(rgce, p + 5)?);
                    let mut t = sheet;
                    let cell = Corner::new(row, col, base);
                    outside |= cell.outside;
                    cell.push(&mut t);
                    stack.push(Item::value(t));
                    7
                }
                // ptgArea3d: sheet, rows, columns.
                0x3B => {
                    let sheet = self.sheet_ref(u16_at(rgce, p + 1)?, sheets)?;
                    let (r0, r1) = (u16_at(rgce, p + 3)?, u16_at(rgce, p + 5)?);
                    let (c0, c1) = (u16_at(rgce, p + 7)?, u16_at(rgce, p + 9)?);
                    let mut t = sheet;
                    outside |= push_area(&mut t, (r0, r1), (c0, c1), base);
                    stack.push(Item::value(t));
                    11
                }
                _ => return None,
            };
            if stack.len() > MAX_STACK
                || stack
                    .last()
                    .is_some_and(|t| t.text.len() > MAX_FORMULA_TEXT)
            {
                return None;
            }
            p = p.checked_add(step)?;
        }
        // Exactly the declared bytes, and one result.
        let text = stack.pop()?.into_value()?;
        (p == rgce.len() && stack.is_empty()).then_some(Decoded {
            text,
            outside,
            uses,
        })
    }
}

/// A formula of a `NAME` record: the parsed expression and what follows
/// it in the record.
#[derive(Clone, Copy)]
struct Tokens<'a> {
    rgce: &'a [u8],
    extra: &'a [u8],
}

/// What an operand of the decoder is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// An expression.
    Value,
    /// An argument that is left out: only an argument of a call.
    Missing,
    /// The name of a function: only the first operand of a call by name.
    Func,
}

/// How tightly the outermost operator of an operand's text binds, in the
/// order of the formula parser the text goes to (`tally_formula`:
/// comparisons, `&`, `+ -`, `* /`, `^`, prefix signs, `%`). Text without
/// an operator outside parentheses (a constant, a reference, a name, a
/// call, an array, anything in parentheses) is an [`ATOM`]. The result
/// of a reference operator (range, intersection) is [`REFERENCE`], lower
/// than everything: it gets parentheses under any operator.
const REFERENCE: u8 = 0;
const COMPARE: u8 = 1;
const CONCAT: u8 = 2;
const ADD: u8 = 3;
const MUL: u8 = 4;
const POW: u8 = 5;
const PREFIX: u8 = 6;
const POSTFIX: u8 = 7;
const ATOM: u8 = u8::MAX;

/// An operand waiting for its operator or function.
#[derive(Debug)]
struct Item {
    text: String,
    kind: Kind,
    /// See [`ATOM`].
    prec: u8,
}

impl Item {
    /// A constant, a reference, a name, a call.
    fn value(text: String) -> Self {
        Self {
            text,
            kind: Kind::Value,
            prec: ATOM,
        }
    }

    /// The text of an expression.
    fn into_value(self) -> Option<String> {
        (self.kind == Kind::Value).then_some(self.text)
    }
}

/// Puts an operand in parentheses; the bytes of text moved.
fn wrap(item: &mut Item) -> usize {
    item.text.insert(0, '(');
    item.text.push(')');
    item.prec = ATOM;
    item.text.len()
}

/// Replaces the last two operands by `left op right`; the bytes of text
/// moved. The tokens are a tree, the text is read by precedence: an
/// operand whose own operator binds less than `op` gets parentheses, and
/// so does a right operand that binds the same (`1-(2-3)`), a left one
/// too under `^`, and anything that is not an [`ATOM`] under `^` or a
/// reference operator (no reliance on how `-2^2` or `2^3^2` is read).
/// Parentheses the file has (`ptgParen`) make an atom, so they are not
/// doubled.
fn binary(stack: &mut Vec<Item>, op: &str, prec: u8) -> Option<usize> {
    let mut b = stack.pop()?;
    let a = stack.last_mut()?;
    if a.kind != Kind::Value || b.kind != Kind::Value {
        return None;
    }
    let (left, right) = match prec {
        REFERENCE | POW => (a.prec != ATOM, b.prec != ATOM),
        _ => (a.prec < prec, b.prec <= prec),
    };
    let mut moved = op.len();
    if left {
        moved = moved.saturating_add(wrap(a));
    }
    if right {
        wrap(&mut b);
    }
    a.text.push_str(op);
    a.text.push_str(&b.text);
    a.prec = prec;
    Some(moved.saturating_add(b.text.len()))
}

/// Replaces the last `argc` operands by the call `name(…)` with them as
/// its arguments; the bytes of text moved. `None` when there are not that
/// many operands, one of them is a function's name, the text would be
/// over the limit, or the only argument is a left-out one (as text that
/// would be a call without arguments).
fn call(stack: &mut Vec<Item>, name: &str, argc: usize) -> Option<usize> {
    let at = stack.len().checked_sub(argc)?;
    let args = stack.get(at..)?;
    if args.iter().any(|a| a.kind == Kind::Func)
        || (argc == 1 && args.iter().any(|a| a.kind == Kind::Missing))
    {
        return None;
    }
    // The name, the parentheses, the commas, the arguments.
    let len = args
        .iter()
        .fold(name.len().saturating_add(2).saturating_add(argc), |n, a| {
            n.saturating_add(a.text.len())
        });
    if len > MAX_FORMULA_TEXT {
        return None;
    }
    let mut text = String::with_capacity(len);
    text.push_str(name);
    text.push('(');
    for (i, a) in stack.drain(at..).enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&a.text);
    }
    text.push(')');
    stack.push(Item::value(text));
    Some(len)
}

/// Characters of a formula (8-bit, or UTF-16 when `wide`) as a text
/// constant of Excel formula text; `None` for text that cannot be written
/// in a formula.
fn quoted(chars: &[u8], wide: bool) -> Option<String> {
    let text: String = if wide {
        let units: Vec<u16> = chars
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&units).ok()?
    } else {
        chars.iter().map(|b| char::from(*b)).collect()
    };
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(format!("\"{}\"", text.replace('"', "\"\"")))
}

/// The eight bytes of a number as formula text; `None` for one that is
/// not finite.
fn number(b: &[u8]) -> Option<String> {
    let v = f64::from_le_bytes(b.try_into().ok()?);
    if !v.is_finite() {
        return None;
    }
    // Exponent form keeps a huge or tiny number short.
    Some(if v != 0.0 && !(1e-5..1e15).contains(&v.abs()) {
        format!("{v:E}")
    } else {
        v.to_string()
    })
}

/// The array constant at `*next` of `extra` as Excel text (`{1,2;3,4}`),
/// moving `*next` past it: columns − 1 (a byte), rows − 1 (a word), then
/// row by row a type byte with a number (1), a text (2: a length word,
/// option flags, the characters) or a boolean (4), the fixed ones eight
/// bytes wide. `None` for an array that is cut, that has an empty or an
/// error element (not read), or whose text is over the limit. Every
/// element read takes bytes of `extra`, which bounds the loop.
fn array(extra: &[u8], next: &mut Option<usize>) -> Option<String> {
    let mut at = (*next)?;
    let cols = usize::from(*extra.get(at)?) + 1;
    let rows = usize::from(u16_at(extra, at.checked_add(1)?)?) + 1;
    at = at.checked_add(3)?;
    let mut t = String::from("{");
    for r in 0..rows {
        if r > 0 {
            t.push(';');
        }
        for c in 0..cols {
            if c > 0 {
                t.push(',');
            }
            let body = at.checked_add(1)?;
            let fixed = body.checked_add(8)?;
            at = match *extra.get(at)? {
                0x01 => {
                    t.push_str(&number(extra.get(body..fixed)?)?);
                    fixed
                }
                0x02 => {
                    let cch = usize::from(u16_at(extra, body)?);
                    let wide = extra.get(body.checked_add(2)?)? & 1 != 0;
                    let from = body.checked_add(3)?;
                    let to = from.checked_add(if wide { cch.checked_mul(2)? } else { cch })?;
                    t.push_str(&quoted(extra.get(from..to)?, wide)?);
                    to
                }
                0x04 => {
                    let b = extra.get(body..fixed)?;
                    t.push_str(if b.first()? & 1 != 0 { "TRUE" } else { "FALSE" });
                    fixed
                }
                _ => return None,
            };
            if t.len() > MAX_FORMULA_TEXT {
                return None;
            }
        }
    }
    t.push('}');
    *next = Some(at);
    Some(t)
}

/// The base cell (row, column) for names with a relative reference that
/// points up or left: every offset of a BIFF8 name (a signed 16-bit row,
/// a signed 8-bit column) has a cell from here.
const REBASE: (u32, u32) = (0x8000, 0x80);

/// A decoded name formula.
#[derive(Debug, PartialEq, Eq)]
struct Decoded {
    text: String,
    /// A relative reference points above or left of the base cell: the
    /// text is not usable, the formula wants a base further in.
    outside: bool,
    /// The `NAME` records (0-based) the formula uses.
    uses: Vec<usize>,
}

/// Whether `name` can be written as a function's name in formula text:
/// ASCII letters, digits, `.` and `_`, not starting with a digit or a
/// dot.
fn is_function_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')
}

/// Characters of a record (8-bit, or UTF-16 when `wide`) as text.
fn record_text(chars: &[u8], wide: bool) -> String {
    if wide {
        let units: Vec<u16> = chars
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        chars.iter().map(|b| char::from(*b)).collect()
    }
}

/// The function an `EXTERNNAME` record of the add-in `SUPBOOK` names;
/// `None` for a record that is cut, is a DDE or OLE link, or whose name
/// cannot be a function's.
fn extern_function(d: &[u8]) -> Option<String> {
    // Built-in, "wants advice", picture, OLE and OLE link.
    if u16_at(d, 0)? & 0x001F != 0 {
        return None;
    }
    let cch = usize::from(*d.get(6)?);
    let wide = d.get(7)? & 1 != 0;
    let len = if wide { 2 * cch } else { cch };
    let name = record_text(d.get(8..8 + len)?, wide);
    // One of the add-in functions of Excel 97-2003, as it spells them:
    // not a built-in function's name, and not one only Tally has.
    ftab::is_addin(&name).then_some(name)
}

/// The fields of a `NAME` record; `None` when it is too short for them.
fn raw_name(d: &[u8]) -> Option<RawName> {
    let flags = u16_at(d, 0)?;
    let cch = usize::from(*d.get(3)?);
    let cce = usize::from(u16_at(d, 4)?);
    let itab = u16_at(d, 8)?;
    // Built-in and "big" names.
    if flags & (0x0020 | 0x1000) != 0 || cch == 0 {
        return None;
    }
    let wide = d.get(14)? & 1 != 0;
    let len = if wide { 2 * cch } else { cch };
    let name = record_text(d.get(15..15 + len)?, wide);
    // Function, Visual Basic and macro names are not names of the model.
    // One spelled `_xlfn.…` is how a formula calls a function newer than
    // the format; the others are macros.
    if flags & (0x0002 | 0x0004 | 0x0008) != 0 {
        // Exactly one prefix, and a name after it; which functions are
        // read is decided when the names are resolved.
        let future = is_function_name(&name)
            && name.strip_prefix("_xlfn.").is_some_and(|rest| {
                !rest.is_empty()
                    && !rest
                        .get(..6)
                        .is_some_and(|p| p.eq_ignore_ascii_case("_xlfn."))
            });
        return future.then(|| RawName {
            func: Some(name),
            ..RawName::default()
        });
    }
    if name.chars().any(char::is_control) {
        return None;
    }
    let at = 15 + len;
    let rgce = (cce > 0)
        .then(|| d.get(at..at + cce))
        .flatten()
        .map(<[u8]>::to_vec);
    let extra = match &rgce {
        Some(_) => d.get(at + cce..).unwrap_or_default().to_vec(),
        None => Vec::new(),
    };
    Some(RawName {
        name,
        itab,
        rgce,
        extra,
        func: None,
    })
}

/// Appends a column's letters.
fn push_col(t: &mut String, col: u32) {
    let mut letters = [0u8; 8];
    let mut n = 0;
    let mut c = col + 1;
    while c > 0 && n < letters.len() {
        letters[n] = b'A' + ((c - 1) % 26) as u8;
        c = (c - 1) / 26;
        n += 1;
    }
    t.extend(letters[..n].iter().rev().map(|b| char::from(*b)));
}

/// One cell of a reference in a name: where it is for a formula in the
/// base cell, and what is relative.
///
/// The column word's bits 14 and 15 say "relative" (column, row). In a
/// name a relative part is not a position but an offset from the cell
/// that uses the name: the row word as a signed 16-bit number, the low
/// byte of the column word as a signed 8-bit one (OpenOffice
/// `ExcelToSc8::ExcRelToScRel8` with `bName`).
#[derive(Clone, Copy)]
struct Corner {
    row: u32,
    col: u32,
    row_rel: bool,
    col_rel: bool,
    /// An offset points above or left of the base cell.
    outside: bool,
}

impl Corner {
    fn new(row: u16, col: u16, base: (u32, u32)) -> Self {
        let (col_rel, row_rel) = (col & 0x4000 != 0, col & 0x8000 != 0);
        let at = |rel: bool, base: u32, offset: i64, position: u32| -> (u32, bool) {
            if !rel {
                return (position, false);
            }
            match u32::try_from(i64::from(base) + offset) {
                Ok(v) => (v, false),
                Err(_) => (0, true),
            }
        };
        let (r, out_r) = at(row_rel, base.0, i64::from(row as i16), u32::from(row));
        let (c, out_c) = at(
            col_rel,
            base.1,
            i64::from(col as u8 as i8),
            u32::from(col & 0xFF),
        );
        Self {
            row: r,
            col: c,
            row_rel,
            col_rel,
            outside: out_r || out_c,
        }
    }

    fn push(&self, t: &mut String) {
        if !self.col_rel {
            t.push('$');
        }
        push_col(t, self.col);
        if !self.row_rel {
            t.push('$');
        }
        t.push_str(&(self.row + 1).to_string());
    }
}

/// Appends an area; `true` when a relative corner is outside the sheet
/// from `base`. An absolute area over all 65,536 rows or all 256 columns
/// of the BIFF8 sheet is a whole column or row of the model's larger
/// sheet, as in OpenOffice (`ExcelToSc8`: `SetComplCol`, `SetComplRow`).
fn push_area(t: &mut String, (r0, r1): (u16, u16), (c0, c1): (u16, u16), base: (u32, u32)) -> bool {
    let (mut a, mut b) = (Corner::new(r0, c0, base), Corner::new(r1, c1, base));
    if !a.row_rel && !b.row_rel && a.row == 0 && b.row == 0xFFFF {
        b.row = MAX_ROWS - 1;
    } else if !a.col_rel && !b.col_rel && a.col == 0 && b.col == 0xFF {
        b.col = MAX_COLS - 1;
    }
    // Corners out of order are put in order, each part with its flag.
    if a.row > b.row {
        std::mem::swap(&mut a.row, &mut b.row);
        std::mem::swap(&mut a.row_rel, &mut b.row_rel);
    }
    if a.col > b.col {
        std::mem::swap(&mut a.col, &mut b.col);
        std::mem::swap(&mut a.col_rel, &mut b.col_rel);
    }
    a.push(t);
    t.push(':');
    b.push(t);
    a.outside || b.outside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recs() -> NameRecs {
        let mut n = NameRecs::default();
        n.supbook(&[2, 0, 1, 4]);
        // Entry 0: sheet 0; entry 1: sheet 1; entry 2: sheets 0–1; entry
        // 3: another workbook; entry 4: a deleted sheet.
        let mut d = vec![5, 0];
        for (book, first, last) in [
            (0u16, 0u16, 0u16),
            (0, 1, 1),
            (0, 0, 1),
            (1, 0, 0),
            (0, 0xFFFF, 0xFFFF),
        ] {
            for w in [book, first, last] {
                d.extend_from_slice(&w.to_le_bytes());
            }
        }
        n.externsheet(&d);
        n
    }

    fn sheets() -> Vec<String> {
        vec!["One".to_string(), "It's two".to_string()]
    }

    /// Decoded for a formula in A1, every name being a good one.
    fn text(n: &NameRecs, rgce: &[u8]) -> Option<String> {
        text_with(n, rgce, &[])
    }

    /// [`text`] for an expression followed by `extra` in its record.
    fn text_with(n: &NameRecs, rgce: &[u8], extra: &[u8]) -> Option<String> {
        let valid = vec![true; n.names.len()];
        let formula = Tokens { rgce, extra };
        n.excel_text(formula, &sheets(), &valid, (0, 0), &mut WORK_BUDGET.clone())
            .map(|d| d.text)
    }

    fn area(ixti: u16, r0: u16, r1: u16, c0: u16, c1: u16) -> Vec<u8> {
        let mut d = vec![0x3B];
        for w in [ixti, r0, r1, c0, c1] {
            d.extend_from_slice(&w.to_le_bytes());
        }
        d
    }

    #[test]
    fn references_numbers_text_and_operators() {
        let n = recs();
        assert_eq!(
            text(&n, &area(0, 0, 9, 0, 1)).as_deref(),
            Some("'One'!$A$1:$B$10")
        );
        // Relative column and row, a quoted sheet name.
        assert_eq!(
            text(&n, &[0x3A, 1, 0, 4, 0, 2, 0xC0]).as_deref(),
            Some("'It''s two'!C5")
        );
        // Whole columns and whole rows.
        assert_eq!(
            text(&n, &area(0, 0, 0xFFFF, 1, 2)).as_deref(),
            Some("'One'!$B$1:$C$1048576")
        );
        assert_eq!(
            text(&n, &area(0, 2, 3, 0, 255)).as_deref(),
            Some("'One'!$A$3:$XFD$4")
        );
        // 2 * 0.5 + "a""b" in parentheses, negated: RPN.
        let mut rgce = vec![0x1E, 2, 0, 0x1F];
        rgce.extend_from_slice(&0.5f64.to_le_bytes());
        rgce.extend_from_slice(&[0x05, 0x17, 3, 0, b'a', b'"', b'b', 0x08, 0x15, 0x13]);
        assert_eq!(text(&n, &rgce).as_deref(), Some("-(2*0.5&\"a\"\"b\")"));
        // A union under ptgMemFunc.
        let (a, b) = (area(0, 0, 0, 0, 0), area(1, 1, 1, 1, 1));
        let mut rgce = vec![0x29, (a.len() + b.len() + 1) as u8, 0];
        rgce.extend_from_slice(&a);
        rgce.extend_from_slice(&b);
        rgce.push(0x10);
        assert_eq!(
            text(&n, &rgce).as_deref(),
            Some("('One'!$A$1:$A$1,'It''s two'!$B$2:$B$2)")
        );
        assert_eq!(text(&n, &[0x1D, 1]).as_deref(), Some("TRUE"));
        let mut big = vec![0x1F];
        big.extend_from_slice(&1e300f64.to_le_bytes());
        assert_eq!(text(&n, &big).as_deref(), Some("1E300"));
    }

    #[test]
    fn what_is_not_decoded_is_none() {
        let n = recs();
        // A 3-D span, another workbook, a deleted sheet, an entry that is
        // not there.
        for ixti in [2u16, 3, 4, 5, 0xFFFF] {
            assert_eq!(text(&n, &area(ixti, 0, 0, 0, 0)), None, "ixti {ixti}");
        }
        // A macro-sheet function, an error, a 2-D reference, an array
        // without its values, a missing argument and `IF`'s attribute on
        // their own, an attribute that is not one.
        for rgce in [
            &[0x21u8, 54, 0][..],
            &[0x1C, 7],
            &[0x24, 0, 0, 0, 0],
            &[0x20, 0, 0, 0, 0, 0, 0, 0],
            &[0x16],
            &[0x19, 0x02, 0, 0],
            &[0x1E, 1, 0, 0x19, 0x20, 0, 0],
        ] {
            assert_eq!(text(&n, rgce), None, "{rgce:?}");
        }
        // Operators without operands, two results, nothing.
        assert_eq!(text(&n, &[0x03]), None);
        assert_eq!(text(&n, &[0x1E, 1, 0, 0x03]), None);
        assert_eq!(text(&n, &[0x1E, 1, 0, 0x1E, 2, 0]), None);
        assert_eq!(text(&n, &[]), None);
        // NaN, a control character in text, a name index without a name.
        let mut nan = vec![0x1F];
        nan.extend_from_slice(&f64::NAN.to_le_bytes());
        assert_eq!(text(&n, &nan), None);
        assert_eq!(text(&n, &[0x17, 1, 0, 7]), None);
        assert_eq!(text(&n, &[0x23, 0, 0, 0, 0]), None);
        assert_eq!(text(&n, &[0x23, 9, 0, 0, 0]), None);
    }

    #[test]
    fn every_cut_and_every_byte() {
        let n = recs();
        let mut full = area(0, 0, 9, 0, 1);
        full.extend_from_slice(&[0x17, 2, 1, b'a', 0, b'b', 0, 0x08, 0x1E, 1, 0, 0x03, 0x15]);
        assert!(text(&n, &full).is_some());
        for cut in 0..full.len() {
            let _ = text(&n, &full[..cut]);
        }
        for b in 0..=255u8 {
            let _ = text(&n, &[b]);
            let _ = text(&n, &[b, b, b, b, b, b, b, b, b, b, b, b]);
            let mut after = area(0, 0, 0, 0, 0);
            after.push(b);
            let _ = text(&n, &after);
            for at in 0..full.len() {
                let mut m = full.clone();
                m[at] = b;
                let _ = text(&n, &m);
            }
        }
    }

    #[test]
    fn nesting_is_bounded_by_the_text_and_work_limits() {
        let n = recs();
        // 1, then 20,000 × "(…)": the text limit ends it.
        let mut deep = vec![0x1E, 1, 0];
        deep.extend(std::iter::repeat_n(0x15, 20_000));
        assert_eq!(text(&n, &deep), None);
        // Within the text limit, the work is charged: a small budget ends
        // the decoding, and what was charged never exceeds the budget.
        let mut parens = vec![0x1E, 1, 0];
        parens.extend(std::iter::repeat_n(0x15, 1_000));
        assert!(text(&n, &parens).is_some());
        let mut work = 10_000u64;
        assert_eq!(
            n.excel_text(
                Tokens {
                    rgce: &parens,
                    extra: &[]
                },
                &sheets(),
                &[],
                (0, 0),
                &mut work
            ),
            None
        );
        assert!(work <= 10_000);
    }

    /// The data of a `NAME` record of the workbook: `name` = `rgce`.
    fn record(name: &str, rgce: &[u8]) -> Vec<u8> {
        let mut d = vec![0, 0, 0, name.len() as u8, rgce.len() as u8];
        d.extend_from_slice(&[0; 10]);
        d.extend_from_slice(name.as_bytes());
        d.extend_from_slice(rgce);
        d
    }

    #[test]
    fn relative_references_are_offsets() {
        let n = recs();
        let at = |rgce: &[u8], base: (u32, u32)| {
            let formula = Tokens { rgce, extra: &[] };
            n.excel_text(formula, &sheets(), &[], base, &mut WORK_BUDGET.clone())
                .map(|d| (d.text, d.outside))
        };
        // Row −1 (0xFFFF), same column, both relative: no cell from A1,
        // the cell above from the base in the middle of the sheet.
        let up = [0x3A, 0, 0, 0xFF, 0xFF, 0x00, 0xC0];
        assert_eq!(at(&up, (0, 0)).map(|d| d.1), Some(true));
        assert_eq!(at(&up, REBASE), Some(("'One'!DY32768".to_string(), false)));
        // Column −1 (0xFF), absolute row 4.
        let left = [0x3A, 0, 0, 3, 0, 0xFF, 0x40];
        assert_eq!(at(&left, (0, 0)).map(|d| d.1), Some(true));
        assert_eq!(at(&left, REBASE), Some(("'One'!DX$4".to_string(), false)));
        // The extremes stay on the BIFF sheet from the base.
        let far = [0x3A, 0, 0, 0x00, 0x80, 0x80, 0xC0];
        assert_eq!(at(&far, REBASE), Some(("'One'!A1".to_string(), false)));
        let near = [0x3A, 0, 0, 0xFF, 0x7F, 0x7F, 0xC0];
        assert_eq!(
            at(&near, REBASE),
            Some(("'One'!IV65536".to_string(), false))
        );
        // An area from one row up to one row down, columns absolute: not a
        // whole column although the row words are 0xFFFF and 1.
        let mut area = vec![0x3B, 0, 0, 0xFF, 0xFF, 1, 0];
        area.extend_from_slice(&[0, 0x80, 1, 0x80]);
        assert_eq!(at(&area, (0, 0)).map(|d| d.1), Some(true));
        assert_eq!(
            at(&area, REBASE),
            Some(("'One'!$A32768:$B32770".to_string(), false))
        );
        // An absolute row 0xFFFF is row 65,536, not an offset.
        assert_eq!(
            at(&[0x3A, 0, 0, 0xFF, 0xFF, 0, 0], (0, 0)),
            Some(("'One'!$A$65536".to_string(), false))
        );
    }

    #[test]
    fn names_with_a_relative_reference_get_a_base_cell() {
        let mut n = recs();
        let limits = Limits::default();
        n.name(
            &record("Up", &[0x3A, 0, 0, 0xFF, 0xFF, 0x00, 0xC0]),
            &limits,
        );
        n.name(&record("Down", &[0x3A, 0, 0, 1, 0, 0x00, 0xC0]), &limits);
        let (names, skipped, _) = n.resolve(&sheets(), &Limits::default());
        assert_eq!(skipped, 0);
        let got: Vec<_> = names
            .iter()
            .map(|n| (n.name.as_str(), n.formula.as_str(), n.base_cell))
            .collect();
        assert_eq!(
            got,
            [
                // The cell above the one that uses the name.
                ("Up", "of:=[$One.DY32768]", Some((0, 0x8000, 0x80))),
                // Offsets down and right keep A1 as the base.
                ("Down", "of:=[$One.A2]", None),
            ]
        );
    }

    #[test]
    fn names_that_read_as_something_else_are_left_out() {
        let mut n = recs();
        let limits = Limits::default();
        let uses = |i: u8| vec![0x23, i, 0, 0, 0, 0x1E, 1, 0, 0x03];
        // 1: a cell address, 2: a name on it, 3: a name on that one.
        n.name(&record("A1", &[0x1E, 6, 0]), &limits);
        n.name(&record("OnCell", &uses(1)), &limits);
        n.name(&record("OnOnCell", &uses(2)), &limits);
        // 4–8: a boolean, R1C1 references, a cell of the larger grid, and
        // names on them.
        n.name(&record("TRUE", &[0x1E, 1, 0]), &limits);
        n.name(&record("R1C1", &[0x1E, 1, 0]), &limits);
        n.name(&record("TAX2024", &[0x1E, 1, 0]), &limits);
        n.name(&record("OnTrue", &uses(4)), &limits);
        n.name(&record("OnTax", &uses(6)), &limits);
        // 9, 10: good names.
        n.name(&record("Good", &[0x1E, 2, 0]), &limits);
        n.name(&record("OnGood", &uses(9)), &limits);
        let (names, skipped, _) = n.resolve(&sheets(), &Limits::default());
        let got: Vec<_> = names
            .iter()
            .map(|n| (n.name.as_str(), n.formula.as_str()))
            .collect();
        // "TAX2024" is a cell only in the larger grid: a name of the file,
        // kept with the name that uses it.
        assert_eq!(
            got,
            [
                ("TAX2024", "of:=1"),
                ("OnTax", "of:=TAX2024+1"),
                ("Good", "of:=2"),
                ("OnGood", "of:=Good+1")
            ]
        );
        assert_eq!(skipped, 6);

        // Along a chain, whatever the order of the records: 1 uses 2, 2
        // uses 3, 3 is an error value. A name that uses itself is left out
        // with them only when it also uses one of them.
        let mut n = recs();
        n.name(&record("First", &uses(2)), &limits);
        n.name(&record("Second", &uses(3)), &limits);
        n.name(&record("Third", &[0x1C, 0x07]), &limits);
        n.name(&record("Loop", &uses(4)), &limits);
        let (names, skipped, _) = n.resolve(&sheets(), &Limits::default());
        let got: Vec<_> = names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(got, ["Loop"]);
        assert_eq!(skipped, 3);
    }

    #[test]
    fn name_records_cut_anywhere() {
        // "Rate" (8-bit) = 5, workbook scope.
        let mut d = vec![0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        d.extend_from_slice(b"Rate");
        d.extend_from_slice(&[0x1E, 5, 0]);
        let full = raw_name(&d).unwrap();
        assert_eq!((full.name.as_str(), full.itab), ("Rate", 0));
        assert_eq!(full.rgce.as_deref(), Some(&[0x1E, 5, 0][..]));
        for cut in 0..d.len() {
            if let Some(r) = raw_name(&d[..cut]) {
                // The name is whole; the expression is whole or absent.
                assert_eq!(r.name, "Rate");
                assert_eq!(r.rgce, None);
            }
        }
        // A formula length beyond the record.
        let mut long = d.clone();
        long[4] = 200;
        assert_eq!(raw_name(&long).unwrap().rgce, None);
        // Built-in, function, macro and Visual Basic names; no name.
        for flags in [0x20u8, 0x02, 0x04, 0x08] {
            let mut m = d.clone();
            m[0] = flags;
            assert!(raw_name(&m).is_none());
        }
        let mut empty = d.clone();
        empty[3] = 0;
        assert!(raw_name(&empty).is_none());
    }

    /// The data of an `EXTERNNAME` record that names a function.
    fn extern_name(name: &str) -> Vec<u8> {
        let mut d = vec![0, 0, 0, 0, 0, 0, name.len() as u8, 0];
        d.extend_from_slice(name.as_bytes());
        // Its formula: `#REF!`.
        d.extend_from_slice(&[2, 0, 0x1C, 0x17]);
        d
    }

    /// [`recs`] with the add-in `SUPBOOK` (its functions: 1 `EDATE`, 2 a
    /// record that is not a function, 3 `NOSUCHFUNCTION`; the
    /// `EXTERNSHEET` entry 3 points at it) and the names 1 `Rate` and
    /// 2 `_xlfn.IFERROR`, a hidden function name.
    fn recs_with_functions() -> NameRecs {
        let mut n = recs();
        let limits = Limits::default();
        n.supbook(&[1, 0, 1, 0x3A]);
        n.externname(&extern_name("EDATE"), &limits);
        n.externname(&extern_name("not a function"), &limits);
        n.externname(&extern_name("NOSUCHFUNCTION"), &limits);
        n.name(&record("Rate", &[0x1E, 5, 0]), &limits);
        let mut future = record("_xlfn.IFERROR", &[0x1C, 0x1D]);
        future[0] = 0x0F;
        n.name(&future, &limits);
        n
    }

    const ONE: [u8; 3] = [0x1E, 1, 0];
    const TWO: [u8; 3] = [0x1E, 2, 0];
    const THREE: [u8; 3] = [0x1E, 3, 0];
    /// `ptgNameX` to the add-in function `i` (1-based).
    fn addin(i: u8) -> [u8; 7] {
        [0x39, 3, 0, i, 0, 0, 0]
    }

    #[test]
    fn functions() {
        let n = recs_with_functions();
        let t = |parts: &[&[u8]]| text(&n, &parts.concat());
        // `SUM` with one argument is an attribute.
        assert_eq!(
            t(&[&area(0, 0, 9, 0, 1), &[0x19, 0x10, 0, 0]]).as_deref(),
            Some("SUM('One'!$A$1:$B$10)")
        );
        // With more, a function with its count; in each token class.
        for ptg in [0x22u8, 0x42, 0x62] {
            assert_eq!(
                t(&[&ONE, &TWO, &[ptg, 2, 4, 0]]).as_deref(),
                Some("SUM(1,2)")
            );
        }
        // Fixed counts, nested: ABS(ROUND(1.5,0)).
        for ptg in [0x21u8, 0x41, 0x61] {
            let mut rgce = vec![0x1F];
            rgce.extend_from_slice(&1.5f64.to_le_bytes());
            rgce.extend_from_slice(&[0x1E, 0, 0, ptg, 27, 0, ptg, 24, 0]);
            assert_eq!(text(&n, &rgce).as_deref(), Some("ABS(ROUND(1.5,0))"));
        }
        assert_eq!(t(&[&[0x21, 74, 0]]).as_deref(), Some("NOW()"));
        // IF with its jumps, also behind "volatile" and spaces.
        assert_eq!(
            t(&[
                &[0x19, 0x01, 0, 0],
                &ONE,
                &[0x19, 0x02, 7, 0],
                &[0x19, 0x40, 0, 1],
                &TWO,
                &[0x19, 0x08, 10, 0],
                &THREE,
                &[0x19, 0x08, 3, 0],
                &[0x22, 3, 1, 0],
            ])
            .as_deref(),
            Some("IF(1,2,3)")
        );
        // CHOOSE with its jump table (two choices: three words).
        assert_eq!(
            t(&[
                &ONE,
                &[0x19, 0x04, 2, 0, 6, 0, 13, 0, 20, 0],
                &TWO,
                &[0x19, 0x08, 10, 0],
                &THREE,
                &[0x19, 0x08, 3, 0],
                &[0x22, 3, 100, 0],
            ])
            .as_deref(),
            Some("CHOOSE(1,2,3)")
        );
        // An argument that is left out; the prompt bit is not a count.
        assert_eq!(
            t(&[&ONE, &[0x16], &THREE, &[0x22, 0x83, 1, 0]]).as_deref(),
            Some("IF(1,,3)")
        );
        // Functions called by name: an add-in one, a newer one.
        assert_eq!(
            t(&[&addin(1), &ONE, &TWO, &[0x22, 3, 255, 0]]).as_deref(),
            Some("EDATE(1,2)")
        );
        assert_eq!(
            t(&[&[0x23, 2, 0, 0, 0], &ONE, &TWO, &[0x22, 3, 255, 0]]).as_deref(),
            Some("_xlfn.IFERROR(1,2)")
        );
        // … inside an expression, with a name as an argument.
        assert_eq!(
            t(&[
                &ONE,
                &addin(1),
                &[0x23, 1, 0, 0, 0],
                &TWO,
                &[0x22, 3, 255, 0],
                &[0x03],
            ])
            .as_deref(),
            Some("1+EDATE(Rate,2)")
        );
    }

    #[test]
    fn array_constants() {
        let n = recs();
        let array = [0x20u8, 0, 0, 0, 0, 0, 0, 0];
        // Two columns, two rows: 1, "a"; TRUE, 2.5.
        let mut extra = vec![1, 1, 0, 0x01];
        extra.extend_from_slice(&1f64.to_le_bytes());
        extra.extend_from_slice(&[0x02, 1, 0, 0, b'a']);
        extra.extend_from_slice(&[0x04, 1, 0, 0, 0, 0, 0, 0, 0, 0x01]);
        extra.extend_from_slice(&2.5f64.to_le_bytes());
        assert_eq!(
            text_with(&n, &array, &extra).as_deref(),
            Some("{1,\"a\";TRUE,2.5}")
        );
        // Every cut of the values.
        for cut in 0..extra.len() {
            assert_eq!(text_with(&n, &array, &extra[..cut]), None, "{cut}");
        }
        // Two arrays take their values in order, also in the other token
        // classes; a third has none.
        let mut two = extra.clone();
        two.extend_from_slice(&[0, 0, 0, 0x02, 1, 0, 1, b'b', 0]);
        let mut rgce = array.to_vec();
        rgce.extend_from_slice(&[0x40, 0, 0, 0, 0, 0, 0, 0, 0x08]);
        assert_eq!(
            text_with(&n, &rgce, &two).as_deref(),
            Some("{1,\"a\";TRUE,2.5}&{\"b\"}")
        );
        rgce.extend_from_slice(&[0x60, 0, 0, 0, 0, 0, 0, 0, 0x08]);
        assert_eq!(text_with(&n, &rgce, &two), None);
        // After a ptgMemArea with one rectangle, whose bytes come first.
        let mut mem = vec![0x26, 0, 0, 0, 0, 0, 0];
        mem.extend_from_slice(&array);
        let mut after = vec![1, 0, 9, 9, 9, 9, 9, 9, 9, 9];
        after.extend_from_slice(&[0, 0, 0, 0x04, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(text_with(&n, &mem, &after).as_deref(), Some("{FALSE}"));
        // … and not when the rectangles are not all there.
        assert_eq!(text_with(&n, &mem, &after[..9]), None);
        // An empty element, an error element, an unknown type; a token
        // that is cut.
        for kind in [0x00u8, 0x10, 0x03, 0xFF] {
            let e = [0, 0, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0];
            assert_eq!(text_with(&n, &array, &e), None, "{kind}");
        }
        assert_eq!(text_with(&n, &array[..7], &extra), None);
        // 256 columns × 65,536 rows announced over a few bytes.
        let mut big = vec![0xFF, 0xFF, 0xFF];
        for _ in 0..100 {
            big.push(0x04);
            big.extend_from_slice(&[0; 8]);
        }
        assert_eq!(text_with(&n, &array, &big), None);
        // Text that cannot be written in a formula; an array whose text
        // is over the limit.
        assert_eq!(text_with(&n, &array, &[0, 0, 0, 0x02, 1, 0, 0, 7]), None);
        let mut long = vec![0xFF, 0xFF, 0x00];
        for _ in 0..20_000 {
            long.push(0x01);
            long.extend_from_slice(&1f64.to_le_bytes());
        }
        assert_eq!(text_with(&n, &array, &long), None);
    }

    #[test]
    fn calls_that_are_not_decoded() {
        let n = recs_with_functions();
        let t = |parts: &[&[u8]]| text(&n, &parts.concat());
        // An index that is not a worksheet function: a macro-sheet one,
        // one past the table, a macro command (bit 15).
        for i in [53u16, 54, 151, 380, 500, 0x7FFF] {
            let [lo, hi] = i.to_le_bytes();
            assert_eq!(t(&[&ONE, &[0x21, lo, hi]]), None, "{i}");
            assert_eq!(t(&[&ONE, &[0x22, 1, lo, hi]]), None, "{i}");
        }
        assert_eq!(t(&[&ONE, &[0x22, 1, 4, 0x80]]), None);
        // More arguments than operands.
        assert_eq!(t(&[&[0x21, 24, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x21, 27, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x22, 5, 4, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x22, 0x7F, 4, 0]]), None);
        assert_eq!(t(&[&[0x19, 0x10, 0, 0]]), None);
        // A count the function does not take: ABS with two arguments, IF
        // with one, a fixed-count token for a function without a fixed
        // count.
        assert_eq!(t(&[&ONE, &TWO, &[0x22, 2, 24, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x22, 1, 1, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x21, 4, 0]]), None);
        // Operands left over.
        assert_eq!(t(&[&ONE, &TWO, &[0x21, 24, 0]]), None);
        // A call by name without a name: no operand, a number as the
        // first one, the name not first, a name that is not a function's.
        assert_eq!(t(&[&[0x22, 0, 255, 0]]), None);
        assert_eq!(t(&[&[0x22, 1, 255, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x22, 1, 255, 0]]), None);
        assert_eq!(t(&[&ONE, &addin(1), &[0x22, 2, 255, 0]]), None);
        assert_eq!(t(&[&[0x23, 1, 0, 0, 0], &ONE, &[0x22, 2, 255, 0]]), None);
        // An add-in record that is not a function, one that is not
        // there, index 0; a `ptgNameX` into this workbook or into no
        // `SUPBOOK`.
        for namex in [
            addin(2),
            addin(4),
            addin(0),
            [0x39, 0, 0, 1, 0, 0, 0],
            [0x39, 9, 0, 1, 0, 0, 0],
        ] {
            assert_eq!(t(&[&namex, &ONE, &[0x22, 2, 255, 0]]), None, "{namex:?}");
        }
        // A function's name is only that: not a result, an operand or an
        // argument.
        assert_eq!(t(&[&addin(1)]), None);
        assert_eq!(t(&[&addin(1), &[0x15]]), None);
        assert_eq!(t(&[&addin(1), &ONE, &[0x03]]), None);
        assert_eq!(t(&[&ONE, &addin(1), &[0x03]]), None);
        assert_eq!(t(&[&addin(1), &[0x19, 0x10, 0, 0]]), None);
        assert_eq!(t(&[&addin(1), &addin(1), &[0x22, 2, 255, 0]]), None);
        // A left-out argument is only an argument, and not the only one.
        assert_eq!(t(&[&ONE, &[0x16], &[0x03]]), None);
        assert_eq!(t(&[&[0x16], &[0x12]]), None);
        assert_eq!(t(&[&[0x16], &[0x22, 1, 4, 0]]), None);
        assert_eq!(t(&[&[0x16], &[0x19, 0x10, 0, 0]]), None);
        assert_eq!(
            t(&[&[0x16], &[0x16], &[0x22, 2, 4, 0]]).as_deref(),
            Some("SUM(,)")
        );
        // A jump table that is not all there, or far longer than the
        // formula.
        assert_eq!(t(&[&ONE, &[0x19, 0x04, 2, 0, 6, 0]]), None);
        assert_eq!(t(&[&ONE, &[0x19, 0x04, 0xFF, 0xFF]]), None);
        assert_eq!(t(&[&ONE, &[0x19, 0x04]]), None);
    }

    #[test]
    fn calls_cut_anywhere_and_every_byte() {
        let n = recs_with_functions();
        // IF(SUM(area), EDATE(1,), {1}) & _xlfn.IFERROR(CHOOSE(1,2,3), 2)
        let full = [
            &area(0, 0, 9, 0, 1)[..],
            &[0x19, 0x10, 0, 0],
            &[0x19, 0x02, 7, 0],
            &addin(1),
            &ONE,
            &[0x16],
            &[0x22, 3, 255, 0],
            &[0x19, 0x08, 10, 0],
            &[0x20, 0, 0, 0, 0, 0, 0, 0],
            &[0x22, 3, 1, 0],
            &[0x23, 2, 0, 0, 0],
            &ONE,
            &[0x19, 0x04, 2, 0, 6, 0, 13, 0, 20, 0],
            &TWO,
            &THREE,
            &[0x22, 3, 100, 0],
            &TWO,
            &[0x22, 3, 255, 0],
            &[0x08],
        ]
        .concat();
        let mut extra = vec![0, 0, 0, 0x01];
        extra.extend_from_slice(&1f64.to_le_bytes());
        assert_eq!(
            text_with(&n, &full, &extra).as_deref(),
            Some("IF(SUM('One'!$A$1:$B$10),EDATE(1,),{1})&_xlfn.IFERROR(CHOOSE(1,2,3),2)")
        );
        // A cut between two tokens can leave a whole formula; no cut
        // leaves this one.
        for cut in 0..full.len() {
            let got = text_with(&n, &full[..cut], &extra);
            assert!(got.is_none_or(|t| !t.contains("IFERROR")), "{cut}");
        }
        for at in 0..full.len() {
            for b in 0..=255u8 {
                let mut m = full.clone();
                m[at] = b;
                let _ = text_with(&n, &m, &extra);
            }
        }
        for at in 0..extra.len() {
            for b in 0..=255u8 {
                let mut m = extra.clone();
                m[at] = b;
                let _ = text_with(&n, &full, &m);
            }
        }
    }

    #[test]
    fn nested_calls_and_pending_operands_are_bounded() {
        let n = recs_with_functions();
        // 1 in 30,000 × ABS(…): the text limit ends it.
        let nested = |depth: usize| {
            let mut rgce = ONE.to_vec();
            for _ in 0..depth {
                rgce.extend_from_slice(&[0x21, 24, 0]);
            }
            rgce
        };
        assert_eq!(text(&n, &nested(30_000)), None);
        // Within the text limit the decoding is a loop, and the work is
        // charged.
        assert!(text(&n, &nested(1_500)).is_some());
        let mut work = 10_000u64;
        let formula = Tokens {
            rgce: &nested(1_500),
            extra: &[],
        };
        assert_eq!(
            n.excel_text(formula, &sheets(), &[], (0, 0), &mut work),
            None
        );
        assert!(work <= 10_000);
        // Operands that pile up: left-out arguments, numbers, function
        // names.
        for token in [&[0x16u8][..], &ONE, &addin(1)] {
            let mut rgce = Vec::new();
            for _ in 0..MAX_STACK {
                rgce.extend_from_slice(token);
            }
            let _ = text(&n, &rgce);
            rgce.extend_from_slice(token);
            rgce.extend_from_slice(&[0x22, 30, 4, 0]);
            assert_eq!(text(&n, &rgce), None);
        }
        // The widest call there is: 127 arguments is more than a function
        // takes.
        let mut wide = Vec::new();
        for _ in 0..127 {
            wide.extend_from_slice(&ONE);
        }
        wide.extend_from_slice(&[0x22, 127, 4, 0]);
        assert_eq!(text(&n, &wide), None);
        // A deeply nested name goes through the conversion without a deep
        // stack: imported or left out, never a crash.
        let mut deep = recs();
        let limits = Limits::default();
        let rgce = nested(1_500);
        let mut d = vec![0, 0, 0, 4];
        d.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
        d.extend_from_slice(&[0; 9]);
        d.extend_from_slice(b"Deep");
        d.extend_from_slice(&rgce);
        deep.name(&d, &limits);
        let (names, skipped, _) = deep.resolve(&sheets(), &Limits::default());
        assert_eq!(names.len() + skipped, 1);
    }

    #[test]
    fn names_with_functions_resolve() {
        let mut n = recs_with_functions();
        let limits = Limits::default();
        let name = |n: &mut NameRecs, name: &str, parts: &[&[u8]]| {
            n.name(&record(name, &parts.concat()), &limits);
        };
        // 3: Total = SUM(One!$A$1:$B$10) * Rate.
        name(
            &mut n,
            "Total",
            &[
                &area(0, 0, 9, 0, 1),
                &[0x19, 0x10, 0, 0],
                &[0x23, 1, 0, 0, 0],
                &[0x05],
            ],
        );
        // 4: Later = EDATE(Rate, 1) (an add-in function).
        name(
            &mut n,
            "Later",
            &[&addin(1), &[0x23, 1, 0, 0, 0], &ONE, &[0x22, 3, 255, 0]],
        );
        // 5: Safe = IFERROR(Total, 0) (a newer function).
        name(
            &mut n,
            "Safe",
            &[
                &[0x23, 2, 0, 0, 0],
                &[0x23, 3, 0, 0, 0],
                &[0x1E, 0, 0],
                &[0x22, 3, 255, 0],
            ],
        );
        // 6: an add-in function Tally does not have; 7: a worksheet
        // function it does not have (PHONETIC); 8: a name on 7.
        name(&mut n, "NoAddIn", &[&addin(3), &ONE, &[0x22, 2, 255, 0]]);
        name(&mut n, "NoBuiltIn", &[&ONE, &[0x21, 0x68, 0x01]]);
        name(&mut n, "OnNoBuiltIn", &[&[0x23, 7, 0, 0, 0], &[0x12]]);
        let (names, skipped, _) = n.resolve(&sheets(), &Limits::default());
        let got: Vec<_> = names
            .iter()
            .map(|n| (n.name.as_str(), n.formula.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Rate", "of:=5"),
                ("Total", "of:=SUM([$One.$A$1:.$B$10])*Rate"),
                ("Later", "of:=EDATE(Rate;1)"),
                ("Safe", "of:=COM.MICROSOFT.IFERROR(Total;0)"),
            ]
        );
        // The function name itself is not a name and is not counted.
        assert_eq!(skipped, 3);
    }

    #[test]
    fn function_names_of_the_file() {
        // Hidden function names: only the `_xlfn.` ones are functions,
        // with any of the function, Visual Basic and macro flags.
        let named = |name: &str, flags: u8| {
            let mut d = record(name, &[0x1C, 0x1D]);
            d[0] = flags;
            raw_name(&d)
        };
        for flags in [0x02u8, 0x08, 0x0E, 0x0F] {
            let r = named("_xlfn.IFERROR", flags).unwrap();
            assert_eq!(r.func.as_deref(), Some("_xlfn.IFERROR"));
            assert!(r.name.is_empty() && r.rgce.is_none());
        }
        // Not: a macro, no name after the prefix, two prefixes, a
        // prefix in other letters, characters a function name has not.
        for name in [
            "MyMacro",
            "_xlfn.",
            "_xlfn._xlfn.SUM",
            "_xlfn._XLFN.SUM",
            "_XLFN.IFERROR",
            "_xlfn.A(",
            "_xlfn.A B",
            "xlfn.IFERROR",
        ] {
            assert!(named(name, 0x0E).is_none(), "{name}");
        }
        // Without those flags it is a name like any other.
        assert_eq!(named("_xlfn.IFERROR", 0).unwrap().func, None);
        // EXTERNNAME: cut anywhere, links, names that are not functions.
        let full = extern_name("EDATE");
        assert_eq!(extern_function(&full).as_deref(), Some("EDATE"));
        for cut in 0..13 {
            assert_eq!(extern_function(&full[..cut]), None, "{cut}");
        }
        for flags in [0x01u8, 0x02, 0x04, 0x08, 0x10] {
            let mut d = full.clone();
            d[0] = flags;
            assert_eq!(extern_function(&d), None);
        }
        // Only the add-in functions of Excel 97-2003 as it spells them:
        // not a built-in function, not a spelling only Tally has, not a
        // function of a later Excel, not another case.
        for name in [
            "",
            "1A",
            "A1(",
            "A B",
            "A,B",
            ".A",
            "É",
            "SUM",
            "IF",
            "TRUE",
            "PI",
            "N",
            "AND",
            "EASTERSUNDAY",
            "MULTIPLE.OPERATIONS",
            "CONVERT_ADD",
            "ERRORTYPE",
            "IFERROR",
            "_xlfn.IFERROR",
            "edate",
        ] {
            assert_eq!(extern_function(&extern_name(name)), None, "{name}");
        }
        let mut wide = vec![0, 0, 0, 0, 0, 0, 3, 1];
        wide.extend_from_slice(&[b'G', 0, b'C', 0, b'D', 0]);
        assert_eq!(extern_function(&wide).as_deref(), Some("GCD"));
        // The records of a `SUPBOOK` that is not the add-in one are not
        // kept, and the limits hold.
        let mut n = NameRecs::default();
        n.externname(&full, &Limits::default());
        n.supbook(&[2, 0, 1, 4]);
        n.externname(&full, &Limits::default());
        n.supbook(&[1, 0, 1, 0x3A]);
        let two = Limits {
            max_names: 2,
            ..Limits::default()
        };
        for _ in 0..5 {
            n.externname(&full, &two);
        }
        let kept: Vec<usize> = n.books.iter().map(|b| b.1.len()).collect();
        assert_eq!(kept, [0, 2]);
        assert!(n.dropped);
        let mut n = NameRecs::default();
        n.supbook(&[1, 0, 1, 0x3A]);
        let tiny = Limits {
            max_name_bytes: 7,
            ..Limits::default()
        };
        n.externname(&full, &tiny);
        n.externname(&full, &tiny);
        assert_eq!(n.books[0].1, ["EDATE".to_string(), String::new()]);
        assert!(n.dropped);
    }

    /// The value-only names `1`, `2` and `3` and the operators after them.
    fn rpn(n: &NameRecs, tokens: &[&[u8]]) -> Option<String> {
        text(n, &tokens.concat())
    }

    #[test]
    fn operands_get_the_parentheses_the_text_needs() {
        let n = recs();
        let t = |tokens: &[&[u8]]| rpn(&n, tokens);
        let (add, sub, mul, div, pow, cat) =
            ([0x03u8], [0x04u8], [0x05u8], [0x06u8], [0x07u8], [0x08u8]);
        let (neg, plus, pct, paren, lt) = ([0x13u8], [0x12u8], [0x14u8], [0x15u8], [0x09u8]);
        let cases: &[(&[&[u8]], &str)] = &[
            // A lower operator under a higher one.
            (&[&ONE, &TWO, &add, &THREE, &mul], "(1+2)*3"),
            (&[&ONE, &TWO, &THREE, &add, &mul], "1*(2+3)"),
            (&[&ONE, &TWO, &cat, &THREE, &add], "(1&2)+3"),
            (&[&ONE, &TWO, &lt, &THREE, &cat], "(1<2)&3"),
            // The same precedence: the left operand as it is, the right
            // one in parentheses.
            (&[&ONE, &TWO, &sub, &THREE, &sub], "1-2-3"),
            (&[&ONE, &TWO, &THREE, &sub, &sub], "1-(2-3)"),
            (&[&ONE, &TWO, &div, &THREE, &div], "1/2/3"),
            (&[&ONE, &TWO, &THREE, &div, &div], "1/(2/3)"),
            (&[&ONE, &TWO, &THREE, &mul, &div], "1/(2*3)"),
            (&[&ONE, &TWO, &THREE, &add, &add], "1+(2+3)"),
            // A higher operator under a lower one needs none.
            (&[&ONE, &TWO, &mul, &THREE, &add], "1*2+3"),
            (&[&ONE, &TWO, &THREE, &mul, &add], "1+2*3"),
            // Powers and signs: nothing is left to how they are read.
            (&[&TWO, &THREE, &pow, &TWO, &pow], "(2^3)^2"),
            (&[&TWO, &THREE, &TWO, &pow, &pow], "2^(3^2)"),
            (&[&TWO, &TWO, &pow, &neg], "-(2^2)"),
            (&[&TWO, &neg, &TWO, &pow], "(-2)^2"),
            (&[&TWO, &TWO, &neg, &pow], "2^(-2)"),
            (&[&TWO, &pct, &TWO, &pow], "(2%)^2"),
            (&[&ONE, &TWO, &add, &neg], "-(1+2)"),
            (&[&ONE, &TWO, &add, &plus], "+(1+2)"),
            (&[&ONE, &TWO, &mul, &neg], "-(1*2)"),
            (&[&ONE, &neg, &neg], "--1"),
            (&[&ONE, &neg, &TWO, &mul], "-1*2"),
            (&[&ONE, &TWO, &neg, &sub], "1--2"),
            // Percent.
            (&[&ONE, &TWO, &add, &pct], "(1+2)%"),
            (&[&ONE, &neg, &pct], "(-1)%"),
            (&[&ONE, &pct, &neg], "-1%"),
            (&[&ONE, &pct, &pct], "1%%"),
            (&[&ONE, &TWO, &pct, &mul], "1*2%"),
            // The file's own parentheses are kept and not doubled.
            (&[&ONE, &TWO, &add, &paren, &THREE, &mul], "(1+2)*3"),
            (&[&ONE, &paren, &paren], "((1))"),
            (&[&ONE, &TWO, &mul, &paren, &THREE, &add], "(1*2)+3"),
            (&[&TWO, &TWO, &pow, &paren, &neg], "-(2^2)"),
        ];
        for (tokens, want) in cases {
            assert_eq!(t(tokens).as_deref(), Some(*want), "{tokens:?}");
        }
        // A negative number is a sign and a number.
        let minus_two = {
            let mut d = vec![0x1F];
            d.extend_from_slice(&(-2f64).to_le_bytes());
            d
        };
        assert_eq!(t(&[&minus_two, &TWO, &pow]).as_deref(), Some("(-2)^2"));
        assert_eq!(t(&[&minus_two, &pct]).as_deref(), Some("(-2)%"));
        assert_eq!(t(&[&ONE, &minus_two, &sub]).as_deref(), Some("1--2"));
        assert_eq!(t(&[&minus_two, &[0x21, 24, 0]]).as_deref(), Some("ABS(-2)"));
        // Reference operators: their result is in parentheses under any
        // operator, and so is an operand of theirs that is not a plain
        // reference; arguments need none.
        let (a, b) = (area(0, 0, 5, 0, 5), area(0, 1, 1, 1, 1));
        let cell = [0x3Au8, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            t(&[&a, &b, &[0x0F], &ONE, &add]).as_deref(),
            Some("('One'!$A$1:$F$6 'One'!$B$2:$B$2)+1")
        );
        assert_eq!(
            t(&[&a, &b, &[0x0F], &cell, &[0x0F]]).as_deref(),
            Some("('One'!$A$1:$F$6 'One'!$B$2:$B$2) 'One'!$A$1")
        );
        assert_eq!(
            t(&[&cell, &cell, &[0x11], &neg]).as_deref(),
            Some("-('One'!$A$1:'One'!$A$1)")
        );
        assert_eq!(
            t(&[&a, &b, &[0x0F], &[0x19, 0x10, 0, 0]]).as_deref(),
            Some("SUM('One'!$A$1:$F$6 'One'!$B$2:$B$2)")
        );
        assert_eq!(
            t(&[&a, &b, &[0x10], &[0x19, 0x10, 0, 0]]).as_deref(),
            Some("SUM(('One'!$A$1:$F$6,'One'!$B$2:$B$2))")
        );
    }

    /// (name, sheet, formula) of the resolved names, and how many were
    /// left out.
    fn resolved(n: &NameRecs) -> (Vec<(String, Option<u32>, String)>, usize) {
        let (names, skipped, cut) = n.resolve(&sheets(), &Limits::default());
        assert!(!cut);
        let names = names
            .into_iter()
            .map(|n| (n.name, n.sheet, n.formula))
            .collect();
        (names, skipped)
    }

    /// `record` local to the sheet `itab` (1-based).
    fn local(name: &str, itab: u8, rgce: &[u8]) -> Vec<u8> {
        let mut d = record(name, rgce);
        d[8] = itab;
        d
    }

    fn got(names: &[(String, Option<u32>, String)]) -> Vec<(&str, Option<u32>, &str)> {
        names
            .iter()
            .map(|n| (n.0.as_str(), n.1, n.2.as_str()))
            .collect()
    }

    /// (name, sheet, the sheet of its base cell).
    type Based = (String, Option<u32>, Option<u32>);

    #[test]
    fn a_name_means_the_record_it_names() {
        let limits = Limits::default();
        let uses = |i: u8| vec![0x23, i, 0, 0, 0, 0x1E, 1, 0, 0x03];
        let both = |i: u8, k: u8| vec![0x23, i, 0, 0, 0, 0x23, k, 0, 0, 0, 0x03];
        // (name, sheet, the sheet of its base cell) of what is read.
        let bases = |n: &NameRecs| -> (Vec<Based>, usize) {
            let (names, skipped, _) = n.resolve(&sheets(), &Limits::default());
            let names = names
                .into_iter()
                .map(|n| (n.name, n.sheet, n.base_cell.map(|b| b.0)))
                .collect();
            (names, skipped)
        };
        let s = |name: &str, sheet: Option<u32>, base: Option<u32>| (name.to_string(), sheet, base);
        // 1: Loc of sheet 2 = 7; 2: Loc of the workbook = 100. Names on
        // record 1 are read from sheet 2, wherever they are names of: 3
        // Bar of the workbook, 4 Baz of sheet 2, 5 Qux of sheet 1. Names
        // on record 2 are read from a sheet without a Loc of its own: 6
        // Quux of the workbook, 7 Corge of sheet 1, 8 Grault of sheet 2.
        // 9: on Bar. 10: on both Locs, which no one sheet tells apart.
        let mut n = recs();
        n.name(&local("Loc", 2, &[0x1E, 7, 0]), &limits);
        n.name(&record("Loc", &[0x1E, 100, 0]), &limits);
        n.name(&record("Bar", &uses(1)), &limits);
        n.name(&local("Baz", 2, &uses(1)), &limits);
        n.name(&local("Qux", 1, &uses(1)), &limits);
        n.name(&record("Quux", &uses(2)), &limits);
        n.name(&local("Corge", 1, &uses(2)), &limits);
        n.name(&local("Grault", 2, &uses(2)), &limits);
        n.name(&record("OnBar", &uses(3)), &limits);
        n.name(&record("Mixed", &both(1, 2)), &limits);
        let (names, skipped) = bases(&n);
        assert_eq!(
            names,
            [
                s("Loc", Some(1), None),
                s("Loc", None, None),
                s("Bar", None, Some(1)),
                s("Baz", Some(1), Some(1)),
                s("Qux", Some(0), Some(1)),
                s("Quux", None, None),
                s("Corge", Some(0), None),
                s("Grault", Some(1), None),
                s("OnBar", None, None),
            ]
        );
        assert_eq!(skipped, 1);

        // A name of every sheet's own and of the workbook: a name on the
        // workbook's has no sheet to be read from.
        let mut n = recs();
        n.name(&local("Loc", 1, &[0x1E, 7, 0]), &limits);
        n.name(&local("Loc", 2, &[0x1E, 8, 0]), &limits);
        n.name(&record("Loc", &[0x1E, 100, 0]), &limits);
        n.name(&record("Quux", &uses(3)), &limits);
        n.name(&record("Two", &both(1, 2)), &limits);
        let (names, skipped) = bases(&n);
        assert_eq!(names.len(), 3);
        assert_eq!(skipped, 2);

        // A name on a later record is read from the cells that use it:
        // 1 on the later Loc of sheet 2 (from the workbook: left out),
        // 2 the same from sheet 2 (kept), 3 on the later Loc of the
        // workbook from the workbook (sheet 2 has its own: left out), 4
        // the same from sheet 1 (kept), 7 on itself (kept).
        let mut n = recs();
        n.name(&record("Early", &uses(5)), &limits);
        n.name(&local("EarlyOn2", 2, &uses(5)), &limits);
        n.name(&record("EarlyAll", &uses(6)), &limits);
        n.name(&local("EarlyOn1", 1, &uses(6)), &limits);
        n.name(&local("Loc", 2, &[0x1E, 7, 0]), &limits);
        n.name(&record("Loc", &[0x1E, 100, 0]), &limits);
        n.name(&record("Loop", &uses(7)), &limits);
        let (names, skipped) = bases(&n);
        assert_eq!(
            names,
            [
                s("EarlyOn2", Some(1), None),
                s("EarlyOn1", Some(0), None),
                s("Loc", Some(1), None),
                s("Loc", None, None),
                s("Loop", None, None),
            ]
        );
        assert_eq!(skipped, 2);

        // A sheet's name that is not read still stands before the
        // workbook's on its sheet (it is kept without a value): a name
        // on the workbook's is read from the other sheet.
        let mut n = recs();
        n.name(&local("Loc", 1, &[0x1C, 0x07]), &limits);
        n.name(&record("Loc", &[0x1E, 100, 0]), &limits);
        n.name(&record("Quux", &uses(2)), &limits);
        n.name(&local("Corge", 1, &uses(2)), &limits);
        let (names, skipped) = bases(&n);
        assert_eq!(
            names,
            [
                s("Loc", Some(0), None),
                s("Loc", None, None),
                s("Quux", None, Some(1)),
                s("Corge", Some(0), Some(1)),
            ]
        );
        assert_eq!(skipped, 1);

        // Two records of one spelling and scope: the first is the one
        // that is defined, so a name on the second is left out.
        let mut n = recs();
        n.name(&record("Twice", &[0x1E, 1, 0]), &limits);
        n.name(&record("twice", &[0x1E, 2, 0]), &limits);
        n.name(&record("OnFirst", &uses(1)), &limits);
        n.name(&record("OnSecond", &uses(2)), &limits);
        let (names, skipped) = resolved(&n);
        let kept: Vec<&str> = names.iter().map(|n| n.0.as_str()).collect();
        assert_eq!(kept, ["Twice", "twice", "OnFirst"]);
        assert_eq!(skipped, 1);

        // A relative name keeps the row and column of its base cell.
        let mut n = recs();
        n.name(&local("Loc", 2, &[0x1E, 7, 0]), &limits);
        let mut up = vec![0x3A, 0, 0, 0xFF, 0xFF, 0x00, 0xC0];
        up.extend_from_slice(&[0x23, 1, 0, 0, 0, 0x03]);
        n.name(&record("Up", &up), &limits);
        let (names, _, _) = n.resolve(&sheets(), &limits);
        assert_eq!(names[1].base_cell, Some((1, 0x8000, 0x80)));
        assert_eq!(names[1].formula, "of:=[$One.DY32768]+Loc");
    }

    #[test]
    fn a_sheets_name_that_is_left_out_does_not_become_the_workbooks() {
        let limits = Limits::default();
        let error = [0x1Cu8, 0x07];
        // 1: Foo of sheet 1, not read; 2: Foo of the workbook = 5; 3: Foo
        // of sheet 2, read; 4: Solo of sheet 1, not read, without a
        // namesake; 5: Both of sheet 1 and 6: Both of the workbook,
        // neither read; 7: Gone of a sheet that is not there, 8: Gone of
        // the workbook.
        let mut n = recs();
        n.name(&local("Foo", 1, &error), &limits);
        n.name(&record("Foo", &[0x1E, 5, 0]), &limits);
        n.name(&local("Foo", 2, &[0x1E, 6, 0]), &limits);
        n.name(&local("Solo", 1, &error), &limits);
        n.name(&local("Both", 1, &error), &limits);
        n.name(&record("Both", &error), &limits);
        n.name(&local("Gone", 9, &[0x1E, 1, 0]), &limits);
        n.name(&record("Gone", &[0x1E, 2, 0]), &limits);
        let (names, skipped) = resolved(&n);
        assert_eq!(
            got(&names),
            [
                // A name of sheet 1 without a value.
                ("Foo", Some(0), "of:=#NAME?"),
                ("Foo", None, "of:=5"),
                ("Foo", Some(1), "of:=6"),
                ("Gone", None, "of:=2"),
            ]
        );
        // It is still counted as left out.
        assert_eq!(skipped, 5);

        // Also for a name of a sheet that is left out because of what it
        // uses, and for one whose formula the pre-scan emptied; once per
        // sheet and spelling.
        let mut n = recs();
        n.name(&record("Rate", &[0x1E, 5, 0]), &limits);
        n.name(&local("Rate", 1, &[0x23, 9, 0, 0, 0]), &limits);
        n.name(&local("RATE", 1, &error), &limits);
        let mut empty = local("Rate", 2, &[]);
        empty[4] = 0;
        n.name(&empty, &limits);
        let (names, skipped) = resolved(&n);
        assert_eq!(
            got(&names),
            [
                ("Rate", None, "of:=5"),
                ("Rate", Some(0), "of:=#NAME?"),
                ("Rate", Some(1), "of:=#NAME?"),
            ]
        );
        assert_eq!(skipped, 2);
    }

    #[test]
    fn functions_called_by_name_are_the_ones_excel_has() {
        let limits = Limits::default();
        let call = |i: u8| vec![0x23, i, 0, 0, 0, 0x1E, 1, 0, 0x1E, 2, 0, 0x22, 3, 255, 0];
        let names = [
            // Excel 2007's, which an .xlsx spells without the prefix.
            ("_xlfn.IFERROR", true),
            ("_xlfn.SUMIFS", true),
            // Later ones, which an .xlsx spells with it.
            ("_xlfn.BITAND", true),
            ("_xlfn.XOR", true),
            // A built-in function, a Tally spelling, nothing, a function
            // of Excel 2007 in other letters.
            ("_xlfn.SUM", false),
            ("_xlfn.IF", false),
            ("_xlfn.EASTERSUNDAY", false),
            ("_xlfn.ERRORTYPE", false),
            ("_xlfn.NOSUCH", false),
            ("_xlfn.iferror", false),
            ("_xlfn.ORG.OPENOFFICE.CONVERT", false),
        ];
        let mut n = recs();
        for (name, _) in names {
            let mut d = record(name, &[0x1C, 0x1D]);
            d[0] = 0x0E;
            n.name(&d, &limits);
        }
        for i in 0..names.len() {
            n.name(&record(&format!("Call{i}"), &call(i as u8 + 1)), &limits);
        }
        let (got, skipped) = resolved(&n);
        let want: Vec<String> = names
            .iter()
            .enumerate()
            .filter(|(_, n)| n.1)
            .map(|(i, _)| format!("Call{i}"))
            .collect();
        let kept: Vec<&str> = got.iter().map(|n| n.0.as_str()).collect();
        assert_eq!(kept, want);
        assert_eq!(skipped, names.len() - want.len());
        assert_eq!(got[0].2, "of:=COM.MICROSOFT.IFERROR(1;2)");
        assert_eq!(got[2].2, "of:=BITAND(1;2)");
    }

    #[test]
    fn the_text_kept_for_the_names_is_bounded() {
        // A name of 200 letters, and names that spell it 30 times each:
        // about 6 KiB of formula from 150 bytes of record.
        let long = "L".repeat(200);
        let mut rgce = Vec::new();
        for _ in 0..30 {
            rgce.extend_from_slice(&[0x23, 1, 0, 0, 0]);
        }
        rgce.extend_from_slice(&[0x22, 30, 0x50, 0x01]);
        let mut n = recs();
        let limits = Limits::default();
        n.name(&record(&long, &[0x1E, 1, 0]), &limits);
        for i in 0..20 {
            n.name(&record(&format!("Item_{i}"), &rgce), &limits);
        }
        let (names, skipped, cut) = n.resolve(&sheets(), &limits);
        assert_eq!((names.len(), skipped, cut), (21, 0, false));
        let each = names[1].formula.len() as u64;
        assert!(each > 6_000, "{each}");
        // Room for the long name and five of the others.
        let tight = Limits {
            max_name_bytes: 300 + 5 * (each + 6),
            ..Limits::default()
        };
        let (names, skipped, cut) = n.resolve(&sheets(), &tight);
        assert_eq!((names.len(), skipped, cut), (6, 0, true));
    }
}
