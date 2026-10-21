//! Named ranges and expressions (plan §3.5/§3.6, inventory §20).
//!
//! - Workbook- and sheet-scoped names live in a [`NameTable`]. A name's
//!   [`NameId`] is never reused: deleting a name leaves a tombstone, so
//!   formulas that use it keep printing its text (and show `#NAME?`), and
//!   defining it again revives the same id.
//! - Formulas keep `Expr::Name` nodes and names are evaluated *by
//!   reference*: the evaluator evaluates a name's own parsed expression
//!   where it is used, once per cell evaluation (memoized, see
//!   [`crate::eval::EvalEnv`]), with cycle detection and a nesting limit of
//!   [`MAX_NAME_DEPTH`]. Nothing is inlined, so names that use other names
//!   many times cannot blow up exponentially. Dependency analysis visits
//!   each reachable definition once ([`collect_names`]). A name's relative
//!   references are relative to the cell using it (OpenOffice semantics);
//!   names entered through the API are usually absolute
//!   (`$Sheet1.$A$1:$B$5`).
//! - The dependency index maps each name key (lowercase text) to the
//!   formulas using it, directly or through other names, so (re)defining or
//!   deleting a name dirties exactly those formulas.

use std::collections::HashMap;
use std::sync::Arc;

use tally_api::ApiError;
use tally_formula::{Dialect, Expr, Formula, NameId, NameRef};
use tally_types::{CellAddr, ErrorKind, SheetId};

use crate::deps::DepIndex;
use crate::formula_cells::{CellState, FormulaCellId};
use crate::workbook::Workbook;

/// Deepest chain of names referring to names; deeper gives `#NAME?`.
pub const MAX_NAME_DEPTH: u32 = 16;

/// Most names per workbook.
pub const MAX_NAMES: usize = 100_000;

/// A name's definition.
#[derive(Clone, Debug, PartialEq)]
pub struct NameDef {
    /// The expression, relative to `base`.
    pub formula: Arc<Formula>,
    /// The cell the expression was parsed at.
    pub base: CellAddr,
}

/// One name (defined or a tombstone).
#[derive(Clone, Debug, PartialEq)]
pub struct NameEntry {
    /// The name as defined.
    pub name: Arc<str>,
    /// Sheet scope, or `None` for the workbook.
    pub scope: Option<SheetId>,
    /// The definition; `None` after deletion.
    pub def: Option<NameDef>,
}

/// The workbook's names. Each part sits behind an `Arc` and is edited
/// copy-on-write (the meta pattern of `ops_v03`), so an undo snapshot of
/// the table is three pointer copies (security review 4, MEDIUM-T6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameTable {
    entries: Arc<Vec<NameEntry>>,
    index: Arc<HashMap<(String, Option<SheetId>), u32>>,
    /// Lowercase alias → entry: the original spelling of an imported name
    /// that was renamed because it reads as a cell reference in Tally's
    /// 16,384-column grid (review 3, H4), so the file's formulas still
    /// resolve it.
    aliases: Arc<HashMap<(String, Option<SheetId>), u32>>,
}

/// The dependency key of a name.
pub fn name_key(name: &str) -> String {
    name.to_lowercase()
}

impl NameTable {
    /// The entry with `id`.
    pub fn get(&self, id: NameId) -> Option<&NameEntry> {
        self.entries.get(id.0 as usize)
    }

    /// The id of `name` in exactly `scope` (defined or not).
    pub fn lookup(&self, name: &str, scope: Option<SheetId>) -> Option<NameId> {
        let key = (name_key(name), scope);
        self.index
            .get(&key)
            .or_else(|| self.aliases.get(&key))
            .map(|i| NameId(*i))
    }

    /// Registers `alias` (an original spelling from a file) for `id`.
    pub fn add_alias(&mut self, alias: &str, scope: Option<SheetId>, id: NameId) {
        Arc::make_mut(&mut self.aliases).insert((name_key(alias), scope), id.0);
    }

    /// The name visible from `sheet`: a defined sheet-scoped name first,
    /// then a defined workbook name, then a tombstone (sheet, workbook).
    pub fn visible(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        let local = self.lookup(name, Some(sheet));
        let global = self.lookup(name, None);
        let defined = |id: &NameId| self.get(*id).is_some_and(|e| e.def.is_some());
        local
            .filter(defined)
            .or_else(|| global.filter(defined))
            .or(local)
            .or(global)
    }

    /// The definition used for `n` in a formula on `sheet`. A deleted
    /// (tombstone) id falls back to the name visible from `sheet` with the
    /// same text, as Excel does when a sheet-local name goes away and a
    /// global one remains (review 3, M9).
    pub fn resolve(&self, n: &NameRef, sheet: SheetId) -> Option<&NameEntry> {
        let id = match n {
            NameRef::Id(id) => self.fallback(*id, sheet),
            NameRef::Unresolved(t) => self.visible(t, sheet)?,
        };
        self.get(id)
    }

    /// `id`, or the defined name of the same text visible from `sheet`
    /// when `id` is a tombstone.
    pub fn fallback(&self, id: NameId, sheet: SheetId) -> NameId {
        match self.get(id) {
            Some(e) if e.def.is_none() => self.visible(&e.name, sheet).unwrap_or(id),
            _ => id,
        }
    }

    /// Defines `name` in `scope` (reviving or replacing an entry).
    pub fn define(&mut self, name: &str, scope: Option<SheetId>, def: NameDef) -> NameId {
        if let Some(id) = self.lookup(name, scope) {
            if let Some(e) = Arc::make_mut(&mut self.entries).get_mut(id.0 as usize) {
                e.name = name.into();
                e.def = Some(def);
            }
            return id;
        }
        let id = self.entries.len() as u32;
        Arc::make_mut(&mut self.entries).push(NameEntry {
            name: name.into(),
            scope,
            def: Some(def),
        });
        Arc::make_mut(&mut self.index).insert((name_key(name), scope), id);
        NameId(id)
    }

    /// Deletes the definition of `id` (the entry stays as a tombstone).
    pub fn undefine(&mut self, id: NameId) -> bool {
        if self
            .entries
            .get(id.0 as usize)
            .is_none_or(|e| e.def.is_none())
        {
            return false;
        }
        match Arc::make_mut(&mut self.entries).get_mut(id.0 as usize) {
            Some(e) if e.def.is_some() => {
                e.def = None;
                true
            }
            _ => false,
        }
    }

    /// Defined names with their ids.
    pub fn defined(&self) -> impl Iterator<Item = (NameId, &NameEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.def.is_some())
            .map(|(i, e)| (NameId(i as u32), e))
    }

    /// Number of defined names.
    pub fn len(&self) -> usize {
        self.defined().count()
    }

    /// `true` if no name is defined.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mutable access to every definition (structure changes, moves).
    pub fn defs_mut(&mut self) -> impl Iterator<Item = &mut NameDef> {
        Arc::make_mut(&mut self.entries)
            .iter_mut()
            .filter_map(|e| e.def.as_mut())
    }

    /// Drops the definitions scoped to `sheet` (when it is deleted).
    pub fn undefine_scope(&mut self, sheet: SheetId) {
        if !self
            .entries
            .iter()
            .any(|e| e.scope == Some(sheet) && e.def.is_some())
        {
            return;
        }
        for e in Arc::make_mut(&mut self.entries).iter_mut() {
            if e.scope == Some(sheet) {
                e.def = None;
            }
        }
    }
}

/// `true` if the expression contains a name.
pub fn has_names(e: &Expr) -> bool {
    let mut found = false;
    e.walk(|x| found |= matches!(x, Expr::Name(_)));
    found
}

/// The definitions of every name an expression reaches (directly or
/// through names used by names), resolved for a formula on one sheet.
///
/// Names are evaluated *by reference* (see [`crate::eval::EvalEnv`]): each
/// definition is kept once, however often it is used, so a chain of names
/// that each use the previous one several times costs linear, not
/// exponential, time and memory (security review 2, CRITICAL-1).
#[derive(Clone, Debug, Default)]
pub struct NameDefs {
    by_id: HashMap<u32, Arc<Formula>>,
    /// Unresolved name text (lowercase) → the id it resolves to.
    by_text: HashMap<String, u32>,
    /// Ids in discovery order.
    order: Vec<u32>,
}

impl NameDefs {
    /// `true` if no defined name is reachable.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The id and definition of `n`, if it is defined.
    pub fn resolve(&self, n: &NameRef) -> Option<(u32, &Arc<Formula>)> {
        let id = match n {
            NameRef::Id(id) => id.0,
            NameRef::Unresolved(t) => match self.by_text.get(t.as_ref()) {
                Some(id) => *id,
                None => *self.by_text.get(&name_key(t))?,
            },
        };
        self.by_id.get(&id).map(|f| (id, f))
    }

    /// Every reachable definition, in discovery order.
    pub fn formulas(&self) -> impl Iterator<Item = &Arc<Formula>> {
        self.order.iter().filter_map(|id| self.by_id.get(id))
    }
}

/// Collects the names reachable from `e` (a formula on `sheet`): their
/// definitions and the dependency keys of every name met, defined or not
/// (so defining one later refreshes the formula). Each name is visited
/// once, so the cost is linear in the total size of the distinct
/// definitions.
pub fn collect_names(names: &NameTable, e: &Expr, sheet: SheetId) -> (NameDefs, Vec<String>) {
    let mut defs = NameDefs::default();
    let mut keys: Vec<String> = Vec::new();
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_text: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pending: Vec<Arc<Formula>> = Vec::new();
    let mut visit = |x: &Expr, defs: &mut NameDefs, pending: &mut Vec<Arc<Formula>>| {
        let Expr::Name(n) = x else {
            return;
        };
        let (id, key) = match n {
            NameRef::Id(id) => {
                let fb = names.fallback(*id, sheet);
                if fb != *id {
                    // Tombstone: alias the written id to the visible
                    // definition (review 3, M9).
                    if let Some(def) = names.get(fb).and_then(|e| e.def.as_ref()) {
                        defs.by_id
                            .entry(id.0)
                            .or_insert_with(|| def.formula.clone());
                    }
                }
                (Some(fb.0), names.get(*id).map(|e| name_key(&e.name)))
            }
            NameRef::Unresolved(t) => {
                let k = name_key(t);
                if !seen_text.insert(k.clone()) {
                    return;
                }
                let id = names.visible(t, sheet).map(|i| i.0);
                if let Some(id) = id {
                    defs.by_text.insert(k.clone(), id);
                }
                (id, Some(k))
            }
        };
        if let Some(k) = key {
            if seen_keys.insert(k.clone()) {
                keys.push(k);
            }
        }
        let Some(id) = id else {
            return;
        };
        if defs.by_id.contains_key(&id) {
            return;
        }
        if let Some(def) = names.get(NameId(id)).and_then(|e| e.def.as_ref()) {
            defs.by_id.insert(id, def.formula.clone());
            defs.order.push(id);
            pending.push(def.formula.clone());
        }
    };
    e.walk(|x| visit(x, &mut defs, &mut pending));
    while let Some(f) = pending.pop() {
        f.expr.walk(|x| visit(x, &mut defs, &mut pending));
    }
    (defs, keys)
}

/// `e` with parentheses stripped and names followed (at most
/// [`MAX_NAME_DEPTH`] of them): what a name box, INDIRECT or a validation
/// list sees as a plain reference. A missing name gives `#NAME?`.
pub fn resolve_chain<'a>(names: &'a NameTable, mut e: &'a Expr, sheet: SheetId) -> &'a Expr {
    static NAME_ERR: Expr = Expr::Err(ErrorKind::Name);
    let mut depth = 0;
    loop {
        match e {
            Expr::Paren(x) => e = x,
            Expr::Name(n) => {
                if depth >= MAX_NAME_DEPTH {
                    return &NAME_ERR;
                }
                depth += 1;
                match names.resolve(n, sheet).and_then(|x| x.def.as_ref()) {
                    Some(d) => e = &d.formula.expr,
                    None => return &NAME_ERR,
                }
            }
            other => return other,
        }
    }
}

/// Checks a name: `[A-Za-z_][A-Za-z0-9_.]*`, at most 255 characters, not a
/// cell or range reference, not `TRUE`/`FALSE`, not R1C1-like.
pub fn validate_name(wb: &Workbook, name: &str) -> Result<(), ApiError> {
    let bad = |why: &str| Err(ApiError::invalid_argument(format!("{name:?} {why}")));
    let mut chars = name.chars();
    match chars.next() {
        // Letters of any script, as the formula parser reads them (AOO
        // `SC_COMPILER_C_CHAR_NAME`; correctness review 4, M18).
        Some(c) if c.is_alphabetic() || c == '_' => {}
        Some(_) => return bad("must start with a letter or _"),
        None => return Err(ApiError::invalid_argument("the name is empty")),
    }
    if !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return bad("may contain only letters, digits, _ and .");
    }
    if name.len() > 255 {
        return bad("is too long");
    }
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        return bad("is reserved");
    }
    let upper = name.to_ascii_uppercase();
    let r1c1 = upper.strip_prefix('R').is_some_and(|rest| {
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        rest.is_empty()
            || rest
                .strip_prefix('C')
                .is_some_and(|x| x.chars().all(|c| c.is_ascii_digit()))
    }) || upper
        .strip_prefix('C')
        .is_some_and(|x| x.chars().all(|c| c.is_ascii_digit()));
    if r1c1 {
        return bad("looks like an R1C1 reference");
    }
    // Anything the formula parser would not read back as a name (a cell
    // reference such as AB12, a sheet-qualified reference…) is refused.
    let origin = CellAddr::new(wb.sheets.first().map_or(1, |s| s.id), 0, 0);
    match tally_formula::parse(
        &format!("={name}"),
        origin,
        Dialect::calc_a1_en_us(),
        &NoNames(wb),
    ) {
        Ok(f) if matches!(f.expr, Expr::Name(_)) => Ok(()),
        _ => bad("is a cell reference or not a valid name"),
    }
}

/// A resolver over the workbook that knows no names (for validation).
struct NoNames<'a>(&'a Workbook);

impl tally_formula::Resolver for NoNames<'_> {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.0.sheet_by_name_ci(name)
    }
    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.0.sheet(id).map(|s| s.name.clone())
    }
    fn name(&self, _name: &str, _sheet: SheetId) -> Option<NameId> {
        None
    }
    fn function(&self, name: &str) -> Option<tally_formula::FuncId> {
        tally_formula::Resolver::function(self.0, name)
    }
}

/// `true` if `name` is a legal name in AOO 4.1's 1,024-column grid (letters,
/// digits, `_` and `.`; not a reference there: a leading letter run of a
/// column past `AMJ` followed by digits is a name, `AB12` is not), but reads
/// as a cell reference in Tally's 16,384-column grid.
pub fn is_aoo_only_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        || name.len() > 255
    {
        return false;
    }
    let letters: String = name
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let rest = &name[letters.len()..];
    if letters.is_empty() || rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let Some(col) = tally_types::letters_to_col(&letters) else {
        return false;
    };
    // AMJ = 1,023 (0-based): AOO's last column.
    col > 1023 && col < tally_types::MAX_COLS
}

/// The model's base cell of a definition (`None` for A1 of the first
/// sheet, the model default).
pub fn base_cell_of(wb: &Workbook, def: &NameDef) -> Option<(u32, u32, u32)> {
    let first = wb.sheets.first().map(|s| s.id).unwrap_or(1);
    if def.base.sheet == first && def.base.row == 0 && def.base.col == 0 {
        return None;
    }
    let tab = wb.tab_of(def.base.sheet)? as u32;
    Some((tab, def.base.row, def.base.col))
}

/// Loads the names of the interchange model (`Workbook::from_data`):
/// relative references are parsed against `base_cell` (ODS
/// `table:base-cell-address`, review 3, H3). Names that read as cell
/// references only in Tally's wider grid are kept under `<name>_` with
/// the original spelling as an alias, and a warning lists them (H4);
/// invalid names are dropped with a warning.
pub fn load_from_data(wb: &mut Workbook, names: &[tally_model::NameData], ids: &[SheetId]) {
    let mut renamed: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for n in names.iter().take(MAX_NAMES) {
        let scope = match n.sheet {
            None => None,
            Some(i) => match ids.get(i as usize) {
                Some(id) => Some(*id),
                None => continue,
            },
        };
        let (name, alias) = if validate_name(wb, &n.name).is_ok() {
            (n.name.clone(), None)
        } else if is_aoo_only_name(&n.name) {
            let mut candidate = format!("{}_", n.name);
            while validate_name(wb, &candidate).is_err() {
                candidate.push('_');
                if candidate.len() > 255 {
                    break;
                }
            }
            if validate_name(wb, &candidate).is_err() {
                dropped.push(n.name.clone());
                continue;
            }
            (candidate, Some(n.name.clone()))
        } else {
            dropped.push(n.name.clone());
            continue;
        };
        if wb
            .names
            .lookup(&name, scope)
            .and_then(|id| wb.names.get(id))
            .is_some_and(|e| e.def.is_some())
        {
            continue;
        }
        let first = ids.first().copied().unwrap_or(1);
        let base = match n.base_cell {
            Some((tab, r, c)) if r < tally_types::MAX_ROWS && c < tally_types::MAX_COLS => {
                CellAddr::new(ids.get(tab as usize).copied().unwrap_or(first), r, c)
            }
            _ => CellAddr::new(first, 0, 0),
        };
        let text = if n.formula.starts_with("of:") || n.formula.starts_with("oooc:") {
            n.formula.clone()
        } else {
            format!("of:={}", n.formula.trim_start_matches('='))
        };
        let Ok(f) = tally_formula::parse(
            &text,
            base,
            Dialect::open_formula(),
            &crate::workbook::OdfResolver(wb),
        ) else {
            continue;
        };
        let id = wb.names.define(
            &name,
            scope,
            NameDef {
                formula: Arc::new(f),
                base,
            },
        );
        if let Some(a) = alias {
            wb.names.add_alias(&a, scope, id);
            renamed.push(format!("{a} → {name}"));
        }
    }
    if !renamed.is_empty() {
        wb.import_warnings.push(format!(
            "Names that read as cell references in Tally's 16,384-column grid were \
             renamed (formulas from the file still resolve them): {}",
            renamed.join(", ")
        ));
    }
    if !dropped.is_empty() {
        wb.import_warnings.push(format!(
            "Invalid names were dropped: {}",
            dropped.join(", ")
        ));
    }
}

impl Workbook {
    /// The base cell for a name in `scope`: A1 of that sheet, else of the
    /// first sheet.
    pub fn name_base(&self, scope: Option<SheetId>) -> CellAddr {
        let sheet = scope
            .filter(|s| self.sheet(*s).is_some())
            .or_else(|| self.sheets.first().map(|s| s.id))
            .unwrap_or(1);
        CellAddr::new(sheet, 0, 0)
    }

    /// Parses a name's `refers_to` text (CalcA1, with or without `=`).
    pub fn parse_name_expr(
        &self,
        refers_to: &str,
        scope: Option<SheetId>,
    ) -> Result<NameDef, ApiError> {
        let t = refers_to.trim();
        let t = t.strip_prefix('=').unwrap_or(t);
        if t.is_empty() {
            return Err(ApiError::invalid_argument("the name refers to nothing"));
        }
        let base = self.name_base(scope);
        let f = self
            .parse_formula(&format!("={t}"), base)
            .map_err(|e| ApiError::invalid_argument(format!("bad expression {t:?}: {e}")))?;
        Ok(NameDef {
            formula: Arc::new(f),
            base,
        })
    }

    /// The `refers_to` text of a definition (CalcA1, without `=`).
    pub fn name_text_of(&self, def: &NameDef) -> String {
        let t = tally_formula::print(&def.formula, def.base, Dialect::calc_a1_en_us(), self);
        t.strip_prefix('=').map(str::to_owned).unwrap_or(t)
    }

    /// Re-registers the dependencies of the formulas using `key` and marks
    /// them dirty (after a name was defined, changed or deleted).
    pub fn refresh_name_dependents(&mut self, key: &str) {
        for id in self.deps.name_dependents(key) {
            self.refresh_formula(id);
        }
    }

    /// Like [`Self::refresh_name_dependents`] for every formula using names.
    pub fn refresh_all_name_dependents(&mut self) {
        let mut ids = self.deps.all_name_dependents();
        ids.extend(
            self.formulas
                .iter()
                .filter(|(_, f)| f.has_names)
                .map(|(id, _)| id),
        );
        ids.sort();
        ids.dedup();
        for id in ids {
            self.refresh_formula(id);
        }
    }

    /// Recomputes the precedents (and name registrations) of formula `id`
    /// from its AST and marks it dirty.
    pub fn refresh_formula(&mut self, id: FormulaCellId) {
        let Some(fc) = self.formulas.get(id) else {
            return;
        };
        let (formula, addr) = (fc.formula.clone(), fc.addr);
        let crate::workbook::Analysis {
            precedents: p,
            strict,
            keys,
            has_3d,
            volatile: vol,
        } = self.analyze(&formula, addr);
        let Some(fc) = self.formulas.get(id) else {
            return;
        };
        let dynamic = fc.dynamic_deps.clone();
        self.deps.remove(id);
        let mut all = p.clone();
        all.extend(dynamic);
        self.deps.add(id, &all);
        self.deps.set_volatile(id, vol);
        self.deps.set_names(id, &keys);
        if let Some(fc) = self.formulas.get_mut(id) {
            fc.precedents = p;
            fc.strict_precedents = strict;
            fc.has_names = !keys.is_empty() || has_names(&formula.expr);
            fc.has_3d = has_3d;
            fc.volatile = vol;
            fc.state = CellState::Dirty;
        }
        self.pending.dirty.push(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recalc::recalc;
    use tally_types::Value;

    fn a(s: &str) -> CellAddr {
        CellAddr::from_a1(1, s).unwrap()
    }

    #[test]
    fn validation() {
        let wb = Workbook::new();
        for ok in ["Total", "_x", "tax.rate", "Q1_sales", "A1B"] {
            assert!(validate_name(&wb, ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "1abc",
            "A1",
            "XFD1048576",
            "a b",
            "TRUE",
            "R1C1",
            "R2",
            "C3",
            "x-y",
        ] {
            assert!(validate_name(&wb, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn table_scopes_and_tombstones() {
        let wb = Workbook::new();
        let mut t = NameTable::default();
        let def = wb.parse_name_expr("$Sheet1.$A$1", None).unwrap();
        let g = t.define("Total", None, def.clone());
        let l = t.define("total", Some(1), def.clone());
        assert_ne!(g, l);
        assert_eq!(t.visible("TOTAL", 1), Some(l));
        assert_eq!(t.visible("TOTAL", 2), Some(g));
        assert!(t.undefine(l));
        assert_eq!(
            t.visible("TOTAL", 1),
            Some(g),
            "defined global wins over tombstone"
        );
        assert_eq!(t.define("TOTAL", Some(1), def), l, "id is revived");
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn expansion_and_evaluation() {
        let mut wb = Workbook::new();
        wb.set_input(a("A1"), "2");
        wb.set_input(a("A2"), "3");
        let def = wb.parse_name_expr("$Sheet1.$A$1:$A$2", None).unwrap();
        wb.names.define("Data", None, def);
        let rate = wb.parse_name_expr("Data*0+0.5", None).unwrap();
        wb.names.define("Rate", None, rate);
        wb.set_input(a("B1"), "=SUM(Data)*Rate");
        recalc(&mut wb);
        assert_eq!(wb.value(a("B1")), Value::Number(2.5));
        let (_, f) = wb.formula_at(a("B1")).unwrap();
        let (_, keys) = collect_names(&wb.names, &f.formula.expr, 1);
        assert!(keys.contains(&"data".to_string()) && keys.contains(&"rate".to_string()));
        assert_eq!(
            wb.formula_text(wb.formula_at(a("B1")).unwrap().0).unwrap(),
            "=SUM(Data)*Rate"
        );
        // Editing a cell inside the name's range recalculates.
        wb.set_input(a("A2"), "13");
        recalc(&mut wb);
        assert_eq!(wb.value(a("B1")), Value::Number(7.5));
    }

    #[test]
    fn self_referential_names_stop() {
        let mut wb = Workbook::new();
        let d = wb.parse_name_expr("Loop+1", None).unwrap();
        wb.names.define("Loop", None, d);
        wb.set_input(a("A1"), "=Loop");
        recalc(&mut wb);
        assert_eq!(wb.value(a("A1")), Value::Error(ErrorKind::Name));
    }
}
