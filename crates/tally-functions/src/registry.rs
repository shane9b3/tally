//! The function registry: [`FunctionDef`] metadata, [`FuncId`] handles and
//! lookup by canonical, ODF and OOXML names.
//!
//! Registration is explicit: every category module exposes
//! `pub fn register(r: &mut Registry)`, and [`crate::registry()`] builds the
//! global registry once. See `docs/contributing-functions.md`.

use std::collections::HashMap;
use std::fmt;
use std::ops::BitOr;

use crate::context::{ArgList, EvalContext, EvalValue};

/// Signature of a function implementation.
///
/// Implementations receive the evaluation context and the argument list;
/// most wrap both in [`crate::Args`] and use [`crate::run`] so they can
/// propagate errors with `?`. The result is an [`EvalValue`] so that
/// reference-returning functions (IF, INDEX, OFFSET, CHOOSE) can hand a
/// range back to an enclosing function; scalar functions return
/// `EvalValue::Scalar` via [`crate::run`].
pub type EvalFn = fn(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue;

/// Index of a function in the [`Registry`].
///
/// Ids are dense and stable within one process (they follow registration
/// order), but they are **not** stable across builds: persist names, never ids.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, PartialOrd, Ord)]
pub struct FuncId(pub u32);

/// Function categories, matching the Apache OpenOffice function wizard.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Category {
    /// Database functions (`DSUM`, `DCOUNT`, …).
    Database,
    /// Date and time functions.
    DateTime,
    /// Financial functions.
    Financial,
    /// Information functions (`ISBLANK`, `NA`, …).
    Information,
    /// Logical functions (`IF`, `AND`, …).
    Logical,
    /// Mathematical functions (`SUM`, `ROUND`, …).
    Mathematical,
    /// Array (matrix) functions (`SUMPRODUCT`, `MMULT`, …).
    Array,
    /// Statistical functions (`AVERAGE`, `COUNT`, …).
    Statistical,
    /// Spreadsheet functions (`VLOOKUP`, `INDEX`, …).
    Spreadsheet,
    /// Text functions.
    Text,
    /// Functions from the Analysis and Date add-ins (`RANDBETWEEN`, …).
    AddIn,
}

impl Category {
    /// Every category, in AOO wizard order.
    pub const ALL: [Category; 11] = [
        Category::Database,
        Category::DateTime,
        Category::Financial,
        Category::Information,
        Category::Logical,
        Category::Mathematical,
        Category::Array,
        Category::Statistical,
        Category::Spreadsheet,
        Category::Text,
        Category::AddIn,
    ];

    /// The AOO category name as used in `docs/calc-functions.json` and
    /// `docs/functions/master-list.toml` (`"Date&Time"`, `"Add-in"`, …).
    pub const fn aoo_name(self) -> &'static str {
        match self {
            Category::Database => "Database",
            Category::DateTime => "Date&Time",
            Category::Financial => "Financial",
            Category::Information => "Information",
            Category::Logical => "Logical",
            Category::Mathematical => "Mathematical",
            Category::Array => "Array",
            Category::Statistical => "Statistical",
            Category::Spreadsheet => "Spreadsheet",
            Category::Text => "Text",
            Category::AddIn => "Add-in",
        }
    }

    /// The source directory / conformance directory name (`"datetime"`, `"addin"`, …).
    pub const fn dir_name(self) -> &'static str {
        match self {
            Category::Database => "database",
            Category::DateTime => "datetime",
            Category::Financial => "financial",
            Category::Information => "information",
            Category::Logical => "logical",
            Category::Mathematical => "mathematical",
            Category::Array => "array",
            Category::Statistical => "statistical",
            Category::Spreadsheet => "spreadsheet",
            Category::Text => "text",
            Category::AddIn => "addin",
        }
    }

    /// Parses an AOO category name (see [`Category::aoo_name`]).
    pub fn from_aoo_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.aoo_name() == s)
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.aoo_name())
    }
}

/// How the evaluator prepares one argument before the function sees it.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ParamKind {
    /// A number. Ranges are reduced by implicit intersection; in array
    /// context, arrays and ranges are lifted elementwise.
    Number,
    /// An integer (a number that the function truncates); same preparation as `Number`.
    Integer,
    /// Text; same preparation as `Number`.
    Text,
    /// A boolean; same preparation as `Number`.
    Bool,
    /// A scalar of any type; ranges are reduced by implicit intersection,
    /// but no array lifting happens.
    Any,
    /// SUM-style: references and arrays are passed through and flattened by
    /// [`crate::coerce::number_seq`].
    NumberSeq,
    /// COUNT/COUNTA-style: references and arrays are passed through and
    /// visited by [`crate::coerce::values_seq`].
    ValueSeq,
    /// Must be a reference (ROW, COLUMN, OFFSET, the SUMIF ranges).
    Reference,
    /// A matrix is accepted as is (SUMPRODUCT, MMULT; OOo "ForceArray").
    /// The engine evaluates this argument in array context.
    Array,
    /// A COUNTIF-family criterion; prepared like `Any`.
    Criteria,
    /// Not evaluated by the evaluator; the function evaluates it on demand
    /// (IF/IFERROR/CHOOSE branches). Use with [`FnFlags::LAZY_ARGS`].
    Lazy,
}

impl ParamKind {
    /// `true` for kinds that take a single value (ranges are intersected).
    pub const fn is_scalar(self) -> bool {
        matches!(
            self,
            ParamKind::Number
                | ParamKind::Integer
                | ParamKind::Text
                | ParamKind::Bool
                | ParamKind::Any
                | ParamKind::Criteria
        )
    }

    /// The kind's name as serialized in [`crate::ParamMeta::kind`]
    /// (`"number"`, `"numberSeq"`, `"reference"`, …).
    pub const fn name(self) -> &'static str {
        match self {
            ParamKind::Number => "number",
            ParamKind::Integer => "integer",
            ParamKind::Text => "text",
            ParamKind::Bool => "bool",
            ParamKind::Any => "any",
            ParamKind::NumberSeq => "numberSeq",
            ParamKind::ValueSeq => "valueSeq",
            ParamKind::Reference => "reference",
            ParamKind::Array => "array",
            ParamKind::Criteria => "criteria",
            ParamKind::Lazy => "lazy",
        }
    }

    /// `true` for kinds that are lifted elementwise in array context.
    pub const fn is_liftable(self) -> bool {
        matches!(
            self,
            ParamKind::Number | ParamKind::Integer | ParamKind::Text | ParamKind::Bool
        )
    }
}

/// One declared parameter.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Param {
    /// Display name, e.g. `"sum_range"`.
    pub name: &'static str,
    /// How the evaluator prepares the argument.
    pub kind: ParamKind,
    /// Whether the argument may be omitted.
    pub optional: bool,
    /// Part of the trailing repeating group (`SUM(number; ...)`,
    /// `SUMIFS(…; criteria_range; criteria; ...)`).
    pub repeat: bool,
}

impl Param {
    /// A required parameter.
    pub const fn required(name: &'static str, kind: ParamKind) -> Self {
        Self {
            name,
            kind,
            optional: false,
            repeat: false,
        }
    }

    /// An optional parameter.
    pub const fn optional(name: &'static str, kind: ParamKind) -> Self {
        Self {
            name,
            kind,
            optional: true,
            repeat: false,
        }
    }

    /// A parameter of the trailing repeating group.
    pub const fn repeat(name: &'static str, kind: ParamKind) -> Self {
        Self {
            name,
            kind,
            optional: false,
            repeat: true,
        }
    }
}

/// What a function returns (metadata for the UI and assist; the runtime result is an [`EvalValue`]).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum ReturnKind {
    /// A number.
    Number,
    /// Text.
    Text,
    /// A boolean.
    Bool,
    /// Depends on the arguments (IF, INDEX).
    Any,
    /// A matrix.
    Array,
    /// A reference (OFFSET, INDIRECT).
    Reference,
}

/// Function flags (a small bit set).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Default)]
pub struct FnFlags(pub u8);

impl FnFlags {
    /// No flags.
    pub const NONE: FnFlags = FnFlags(0);
    /// Recalculated on every recalc (NOW, RAND, …).
    pub const VOLATILE: FnFlags = FnFlags(1);
    /// Has [`ParamKind::Lazy`] parameters that the function evaluates on demand.
    /// Functions with this flag are never array-lifted by the evaluator.
    pub const LAZY_ARGS: FnFlags = FnFlags(1 << 1);
    /// May return a reference (OFFSET, INDIRECT, INDEX).
    pub const RETURNS_REF: FnFlags = FnFlags(1 << 2);
    /// Uses `ctx.origin()` beyond implicit intersection (ROW(), COLUMN()).
    pub const NEEDS_ORIGIN: FnFlags = FnFlags(1 << 3);
    /// Evaluates all of its arguments in array context (SUMPRODUCT).
    pub const FORCE_ARRAY: FnFlags = FnFlags(1 << 4);
    /// Its [`ParamKind::Reference`] arguments are used for their address
    /// only, never their values (ROW, COLUMN), so a reference to the
    /// formula's own cell is not circular.
    pub const REF_ADDRESS_ONLY: FnFlags = FnFlags(1 << 5);

    /// `true` if every flag in `other` is set.
    pub const fn contains(self, other: FnFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Union of two flag sets (usable in `const` items).
    pub const fn union(self, other: FnFlags) -> FnFlags {
        FnFlags(self.0 | other.0)
    }
}

impl BitOr for FnFlags {
    type Output = FnFlags;
    fn bitor(self, rhs: FnFlags) -> FnFlags {
        self.union(rhs)
    }
}

/// `max_args` value meaning "unbounded".
pub const VARIADIC: u8 = 255;

/// Everything the engine, parser and UI need to know about one function.
#[derive(Copy, Clone, Debug)]
pub struct FunctionDef {
    /// Canonical English name in upper case, e.g. `"SUMIF"`.
    pub name: &'static str,
    /// Name stored in ODF (`"SUMIF"`, `"COM.MICROSOFT.IFERROR"`, `"ORG.OPENOFFICE.WEEKS"`).
    pub odf_name: &'static str,
    /// Name stored in OOXML when Excel spells it differently (`"_xlfn.IFS"`,
    /// `"EFFECT"` for EFFECTIVE) or for Analysis add-ins (`"EDATE"`); `None`
    /// means "same as `name`" or "Excel lacks it". The built-in table is in
    /// `categories/ooxml.rs`.
    pub ooxml_name: Option<&'static str>,
    /// AOO category.
    pub category: Category,
    /// Minimum number of arguments.
    pub min_args: u8,
    /// Maximum number of arguments; [`VARIADIC`] (255) means unbounded.
    pub max_args: u8,
    /// Declared parameters; the trailing `repeat` params form the repeating group.
    pub params: &'static [Param],
    /// Result kind (metadata).
    pub returns: ReturnKind,
    /// Flags.
    pub flags: FnFlags,
    /// `true` for functions that are not in AOO 4.1 (IFERROR, IFNA).
    pub extension: bool,
    /// The implementation.
    pub eval: EvalFn,
    /// One-line description (long docs live in `docs/functions`).
    pub help: &'static str,
    /// One realistic example call in the en-US UI dialect, e.g.
    /// `=SUMIF(A1:A10;">5";B1:B10)`, for the Function Wizard. Filled from
    /// the generated table in `categories/examples.rs` (see
    /// `docs/functions/wizard-metadata.md`); `""` until then.
    pub example: &'static str,
}

impl FunctionDef {
    /// Creates a definition with arity derived from `params`: `min_args` is
    /// the number of required params (the repeating group counts once) and
    /// `max_args` is `params.len()`, or [`VARIADIC`] if any param repeats.
    /// The ODF name defaults to `name`.
    pub const fn new(
        name: &'static str,
        category: Category,
        params: &'static [Param],
        eval: EvalFn,
    ) -> Self {
        let mut min = 0u8;
        let mut variadic = false;
        let mut i = 0;
        while i < params.len() {
            if !params[i].optional {
                min += 1;
            }
            if params[i].repeat {
                variadic = true;
            }
            i += 1;
        }
        Self {
            name,
            odf_name: name,
            ooxml_name: None,
            category,
            min_args: min,
            max_args: if variadic {
                VARIADIC
            } else {
                params.len() as u8
            },
            params,
            returns: ReturnKind::Number,
            flags: FnFlags::NONE,
            extension: false,
            eval,
            help: "",
            example: "",
        }
    }

    /// Sets the ODF name.
    pub const fn odf(mut self, odf_name: &'static str) -> Self {
        self.odf_name = odf_name;
        self
    }

    /// Sets the OOXML name.
    pub const fn ooxml(mut self, ooxml_name: &'static str) -> Self {
        self.ooxml_name = Some(ooxml_name);
        self
    }

    /// Overrides the derived arity.
    pub const fn arity(mut self, min_args: u8, max_args: u8) -> Self {
        self.min_args = min_args;
        self.max_args = max_args;
        self
    }

    /// Sets the return kind.
    pub const fn returns(mut self, returns: ReturnKind) -> Self {
        self.returns = returns;
        self
    }

    /// Sets the flags.
    pub const fn flags(mut self, flags: FnFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Marks the function as an extension (not in AOO 4.1).
    pub const fn extension(mut self) -> Self {
        self.extension = true;
        self
    }

    /// Sets the one-line help text.
    pub const fn help(mut self, help: &'static str) -> Self {
        self.help = help;
        self
    }

    /// Sets the example call (`=NAME(...)` in the en-US UI dialect).
    pub const fn example(mut self, example: &'static str) -> Self {
        self.example = example;
        self
    }

    /// `true` if `max_args` is unbounded.
    pub const fn is_variadic(&self) -> bool {
        self.max_args == VARIADIC
    }

    /// The declared parameter for argument `i`, following the trailing
    /// repeating group for variadic functions. `None` past the declared
    /// params of a non-variadic function.
    pub fn param(&self, i: usize) -> Option<&'static Param> {
        let params = self.params;
        if i < params.len() {
            return params.get(i);
        }
        let group = params.iter().rev().take_while(|p| p.repeat).count();
        if group == 0 {
            return None;
        }
        let start = params.len() - group;
        params.get(start + (i - start) % group)
    }

    /// The parameter kind for argument `i` ([`ParamKind::Any`] if undeclared).
    pub fn param_kind(&self, i: usize) -> ParamKind {
        self.param(i).map_or(ParamKind::Any, |p| p.kind)
    }

    /// Whether the engine must evaluate argument `i` in array context: the
    /// function has [`FnFlags::FORCE_ARRAY`] or the param is [`ParamKind::Array`].
    pub fn param_forces_array(&self, i: usize) -> bool {
        self.flags.contains(FnFlags::FORCE_ARRAY) || self.param_kind(i) == ParamKind::Array
    }

    /// Renders the signature, e.g. `SUMIF(range; criteria; [sum_range])` or
    /// `SUM(number; ...)`.
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(self.params.len() + 1);
        for p in self.params {
            if p.optional {
                parts.push(format!("[{}]", p.name));
            } else {
                parts.push(p.name.to_owned());
            }
        }
        if self.params.iter().any(|p| p.repeat) {
            parts.push("...".to_owned());
        }
        format!("{}({})", self.name, parts.join("; "))
    }
}

/// A name that was registered twice (a programming error, caught by tests).
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct NameConflict {
    /// Which name table the conflict is in: `"name"`, `"odf"` or `"ooxml"`.
    pub table: &'static str,
    /// The conflicting (upper-cased) name.
    pub name: String,
    /// The function that keeps the name (registered first).
    pub kept: FuncId,
    /// The function whose registration lost the name.
    pub rejected: FuncId,
}

/// The function table.
#[derive(Default, Debug)]
pub struct Registry {
    defs: Vec<FunctionDef>,
    by_name: HashMap<String, FuncId>,
    by_odf: HashMap<String, FuncId>,
    by_ooxml: HashMap<String, FuncId>,
    conflicts: Vec<NameConflict>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a function and returns its id.
    ///
    /// Names are matched ASCII case-insensitively. If a name is already
    /// taken, the first registration keeps it and the clash is recorded in
    /// [`Registry::conflicts`] (tests assert it is empty) instead of panicking.
    pub fn register(&mut self, def: FunctionDef) -> FuncId {
        let id = FuncId(self.defs.len() as u32);
        Self::insert(&mut self.by_name, &mut self.conflicts, "name", def.name, id);
        Self::insert(
            &mut self.by_odf,
            &mut self.conflicts,
            "odf",
            def.odf_name,
            id,
        );
        if let Some(ooxml) = def.ooxml_name {
            Self::insert(&mut self.by_ooxml, &mut self.conflicts, "ooxml", ooxml, id);
        }
        self.defs.push(def);
        id
    }

    fn insert(
        map: &mut HashMap<String, FuncId>,
        conflicts: &mut Vec<NameConflict>,
        table: &'static str,
        name: &str,
        id: FuncId,
    ) {
        let key = name.to_ascii_uppercase();
        match map.get(&key) {
            Some(&kept) => conflicts.push(NameConflict {
                table,
                name: key,
                kept,
                rejected: id,
            }),
            None => {
                map.insert(key, id);
            }
        }
    }

    /// Sets (or replaces) the OOXML name of the function registered as
    /// `name`; a clash is recorded in [`Registry::conflicts`] and an unknown
    /// `name` is recorded as a conflict with itself (both are bugs the
    /// registry tests catch).
    pub(crate) fn set_ooxml_name(&mut self, name: &str, ooxml: &'static str) {
        let Some(id) = self.lookup(name) else {
            self.conflicts.push(NameConflict {
                table: "ooxml",
                name: name.to_ascii_uppercase(),
                kept: FuncId(u32::MAX),
                rejected: FuncId(u32::MAX),
            });
            return;
        };
        let def = &mut self.defs[id.0 as usize];
        if let Some(old) = def.ooxml_name.replace(ooxml) {
            self.by_ooxml.remove(&old.to_ascii_uppercase());
        }
        Self::insert(&mut self.by_ooxml, &mut self.conflicts, "ooxml", ooxml, id);
    }

    /// Sets the example call of the function registered as `name`. An
    /// unknown `name` is recorded as a conflict in the `"example"` table
    /// (a stale generated table; the registry tests catch it).
    pub(crate) fn set_example(&mut self, name: &str, example: &'static str) {
        match self.lookup(name) {
            Some(id) => self.defs[id.0 as usize].example = example,
            None => self.conflicts.push(NameConflict {
                table: "example",
                name: name.to_ascii_uppercase(),
                kept: FuncId(u32::MAX),
                rejected: FuncId(u32::MAX),
            }),
        }
    }

    /// Name clashes seen during registration (should be empty).
    pub fn conflicts(&self) -> &[NameConflict] {
        &self.conflicts
    }

    /// Number of registered functions.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// `true` if nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// The definition for `id`.
    pub fn get(&self, id: FuncId) -> Option<&FunctionDef> {
        self.defs.get(id.0 as usize)
    }

    /// Looks up a canonical name (`"sumif"` and `"SUMIF"` both work).
    pub fn lookup(&self, name: &str) -> Option<FuncId> {
        self.by_name.get(&name.to_ascii_uppercase()).copied()
    }

    /// Looks up an ODF stored name (`"COM.MICROSOFT.IFERROR"`).
    pub fn lookup_odf(&self, odf_name: &str) -> Option<FuncId> {
        self.by_odf.get(&odf_name.to_ascii_uppercase()).copied()
    }

    /// Looks up an OOXML stored name (`"_xlfn.IFS"`). Names without a
    /// registered OOXML alias fall back to the canonical name, so `"SUM"`
    /// and `"_xlfn.SUM"`-style prefixes on canonical names also resolve.
    pub fn lookup_ooxml(&self, ooxml_name: &str) -> Option<FuncId> {
        let key = ooxml_name.to_ascii_uppercase();
        if let Some(&id) = self.by_ooxml.get(&key) {
            return Some(id);
        }
        let bare = key
            .strip_prefix("_XLFN.")
            .or_else(|| key.strip_prefix("_XLWS."))
            .unwrap_or(&key);
        self.by_name.get(bare).copied()
    }

    /// Looks up any spelling: canonical, then ODF, then OOXML.
    pub fn lookup_any(&self, name: &str) -> Option<FuncId> {
        self.lookup(name)
            .or_else(|| self.lookup_odf(name))
            .or_else(|| self.lookup_ooxml(name))
    }

    /// Looks up a function name read from an OpenFormula (ODF) formula:
    /// ODF stored names first, then [`Registry::lookup_any`]. Use this for
    /// `.ods` import: with `lookup_any` a canonical name wins, so ODF
    /// `DURATION` (the add-in) or `ISOWEEKNUM` (the built-in WEEKNUM) would
    /// resolve to the function of that canonical name instead.
    pub fn lookup_odf_first(&self, name: &str) -> Option<FuncId> {
        self.by_odf
            .get(&name.to_ascii_uppercase())
            .copied()
            .or_else(|| self.lookup_any(name))
    }

    /// Looks up a function name read from an OOXML (Excel) formula: OOXML
    /// stored names first, then [`Registry::lookup_any`]. Use this for
    /// `.xlsx` import, where plain `WEEKNUM`, `CONVERT` and `DURATION` are
    /// the Analysis add-in functions.
    pub fn lookup_ooxml_first(&self, name: &str) -> Option<FuncId> {
        self.lookup_ooxml(name).or_else(|| self.lookup_any(name))
    }

    /// Looks up a canonical name and returns the definition.
    pub fn def_by_name(&self, name: &str) -> Option<&FunctionDef> {
        self.lookup(name).and_then(|id| self.get(id))
    }

    /// All functions in registration (id) order.
    pub fn iter(&self) -> impl Iterator<Item = (FuncId, &FunctionDef)> + '_ {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (FuncId(i as u32), d))
    }
}
