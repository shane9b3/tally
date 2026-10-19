//! A HashMap-backed [`Resolver`] for tests and the CLI.

use std::collections::HashMap;
use std::sync::Arc;

use tally_types::SheetId;

use crate::ast::{FuncId, NameId};
use crate::Resolver;

/// Function names registered by [`SimpleResolver::with_default_functions`]:
/// the Phase A batch-0 set plus a few common extras.
pub const DEFAULT_FUNCTIONS: &[&str] = &[
    "SUM",
    "PRODUCT",
    "ROUND",
    "ROUNDUP",
    "ROUNDDOWN",
    "INT",
    "TRUNC",
    "ABS",
    "SIGN",
    "MOD",
    "POWER",
    "SQRT",
    "RAND",
    "SUMIF",
    "SUMIFS",
    "COUNTIF",
    "COUNTBLANK",
    "AVERAGEIF",
    "AVERAGE",
    "COUNT",
    "COUNTA",
    "MIN",
    "MAX",
    "MEDIAN",
    "AND",
    "OR",
    "NOT",
    "IF",
    "TRUE",
    "FALSE",
    "IFERROR",
    "IFNA",
    "CONCATENATE",
    "LEFT",
    "RIGHT",
    "MID",
    "LEN",
    "UPPER",
    "LOWER",
    "PROPER",
    "TRIM",
    "TEXT",
    "VALUE",
    "FIND",
    "SEARCH",
    "SUBSTITUTE",
    "TODAY",
    "NOW",
    "DATE",
    "YEAR",
    "MONTH",
    "DAY",
    "WEEKDAY",
    "DATEVALUE",
    "VLOOKUP",
    "HLOOKUP",
    "INDEX",
    "MATCH",
    "ROW",
    "COLUMN",
    "ISBLANK",
    "ISNUMBER",
    "ISTEXT",
    "ISERROR",
    "ISNA",
    "NA",
    "SUMPRODUCT",
    "RANDBETWEEN",
    "LOG10",
    "ERROR.TYPE",
    "ADDRESS",
    "PI",
];

/// A simple in-memory [`Resolver`]. Lookups are ASCII case-insensitive.
#[derive(Clone, Debug, Default)]
pub struct SimpleResolver {
    sheets: Vec<(SheetId, Arc<str>)>,
    functions: HashMap<String, FuncId>,
    function_names: Vec<Arc<str>>,
    names: HashMap<(String, Option<SheetId>), NameId>,
    name_texts: Vec<Arc<str>>,
}

impl SimpleResolver {
    /// An empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// A resolver with sheet 0 named `Sheet1` and [`DEFAULT_FUNCTIONS`].
    pub fn with_default_functions() -> Self {
        let mut r = Self::new();
        r.add_sheet(0, "Sheet1");
        for f in DEFAULT_FUNCTIONS {
            r.add_function(f);
        }
        r
    }

    /// Adds (or renames) a sheet.
    pub fn add_sheet(&mut self, id: SheetId, name: &str) -> &mut Self {
        self.sheets.retain(|(i, _)| *i != id);
        self.sheets.push((id, Arc::from(name)));
        self
    }

    /// Removes a sheet (its id then resolves to no name).
    pub fn remove_sheet(&mut self, id: SheetId) -> &mut Self {
        self.sheets.retain(|(i, _)| *i != id);
        self
    }

    /// Registers a function and returns its id (existing id if present).
    pub fn add_function(&mut self, name: &str) -> FuncId {
        let key = name.to_ascii_uppercase();
        if let Some(id) = self.functions.get(&key) {
            return *id;
        }
        let id = FuncId(u32::try_from(self.function_names.len()).unwrap_or(u32::MAX));
        self.function_names.push(Arc::from(key.as_str()));
        self.functions.insert(key, id);
        id
    }

    /// Defines a named range, workbook-wide (`scope == None`) or on a sheet.
    pub fn add_name(&mut self, name: &str, scope: Option<SheetId>) -> NameId {
        let key = (name.to_ascii_uppercase(), scope);
        if let Some(id) = self.names.get(&key) {
            return *id;
        }
        let id = NameId(u32::try_from(self.name_texts.len()).unwrap_or(u32::MAX));
        self.name_texts.push(Arc::from(name));
        self.names.insert(key, id);
        id
    }
}

impl Resolver for SimpleResolver {
    fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.sheets
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(id, _)| *id)
    }

    fn sheet_name(&self, id: SheetId) -> Option<Arc<str>> {
        self.sheets
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.clone())
    }

    fn name(&self, name: &str, sheet: SheetId) -> Option<NameId> {
        let up = name.to_ascii_uppercase();
        self.names
            .get(&(up.clone(), Some(sheet)))
            .or_else(|| self.names.get(&(up, None)))
            .copied()
    }

    fn function(&self, name: &str) -> Option<FuncId> {
        self.functions.get(&name.to_ascii_uppercase()).copied()
    }

    fn name_text(&self, id: NameId) -> Option<Arc<str>> {
        self.name_texts.get(id.0 as usize).cloned()
    }

    fn function_name(&self, id: FuncId) -> Option<Arc<str>> {
        self.function_names.get(id.0 as usize).cloned()
    }
}
