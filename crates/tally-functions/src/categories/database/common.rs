//! Shared evaluation for the D* functions: locate the field, evaluate the
//! criteria table, and collect the selected field's values of matching rows.
//!
//! Semantics follow Apache OpenOffice Calc 4.1 (`sc/source/core/tool/interpr1.cxx`,
//! `GetDBParams` / `DBIterateParameters`, and `ScDocument::CreateQueryParam`
//! for advanced-filter style criteria), Apache-2.0:
//!
//! - The first row of the database holds field names; the remaining rows are records.
//! - `field` is a text name (case-insensitive), a 1-based column index, or a
//!   reference to a header cell (which arrives as its text). `0` (or an
//!   omitted field) selects "no field" and is accepted by DCOUNT/DCOUNTA only.
//!   An unknown name or an out-of-range index is `Err:502`.
//! - The first row of the criteria range holds field names, each matched to
//!   the first database column with the same name (case-insensitive). The
//!   following rows are alternatives (OR); the conditions inside one row are
//!   all required (AND). A criterion cell is parsed with [`Criterion`], so it
//!   may be `>5`, `<>x`, `=`, a bare value, or a wildcard/regex pattern per the
//!   document settings. An empty criterion cell is ignored, a row without
//!   conditions matches every record, and a criteria table with no rows
//!   below its header matches every record. A criteria column whose header
//!   matches no database field is ignored (as in AOO's query builder).

use std::collections::HashMap;
use std::ops::ControlFlow;

use tally_types::{CellAddr, CellRange, ErrorKind, Value};

use crate::criteria::Criterion;
use crate::registry::{Param, ParamKind};
use crate::{Args, EvalContext};

/// Parameters shared by every D* function.
pub(super) const DB_PARAMS: &[Param] = &[
    Param::required("database", ParamKind::Reference),
    Param::required("database_field", ParamKind::Any),
    Param::required("search_criteria", ParamKind::Reference),
];

/// The records that satisfied the criteria table.
pub(super) struct Selection {
    /// Number of matching records.
    pub matched: usize,
    /// The selected field's value in each matching record (empty when the
    /// field was `0`/omitted).
    pub values: Vec<Value>,
}

fn header_text(v: &Value) -> Option<String> {
    match v {
        Value::Text(t) => Some(t.to_lowercase()),
        Value::Number(n) => Some(crate::coerce::number_to_text(*n).to_lowercase()),
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

/// Snapshot of a range's non-empty cells keyed by (row, col).
fn snapshot(ctx: &mut dyn EvalContext, r: &CellRange) -> HashMap<(u32, u32), Value> {
    let mut m = HashMap::new();
    ctx.for_each_in_range(r, true, &mut |addr: CellAddr, v| {
        if !v.is_empty() {
            m.insert((addr.row, addr.col), v.clone());
        }
        ControlFlow::Continue(())
    });
    m
}

/// Evaluates the three arguments. `zero_ok` allows field `0`/omitted
/// (DCOUNT, DCOUNTA).
pub(super) fn select(a: &mut Args, zero_ok: bool) -> Result<Selection, ErrorKind> {
    let db = a.range(0)?;
    let field = a.value(1);
    let crit = a.range(2)?;

    let db = a
        .ctx()
        .used_bounds(&db)
        .unwrap_or(CellRange::new(db.sheet, db.r0, db.c0, db.r0, db.c1));
    let crit = a.ctx().used_bounds(&crit).unwrap_or(CellRange::new(
        crit.sheet, crit.r0, crit.c0, crit.r0, crit.c1,
    ));

    let data = snapshot(a.ctx(), &db);
    let cdata = snapshot(a.ctx(), &crit);
    let ncols = db.c1 - db.c0 + 1;

    // Header lookup: lower-cased name -> first column offset.
    let mut names: HashMap<String, u32> = HashMap::new();
    for c in 0..ncols {
        if let Some(Some(n)) = data.get(&(db.r0, db.c0 + c)).map(header_text) {
            names.entry(n).or_insert(c);
        }
    }

    // Resolve the field to a column offset (None = no field).
    let field_col: Option<u32> = match &field {
        Value::Error(e) => return Err(*e),
        Value::Empty => None,
        Value::Text(_) => {
            let key = header_text(&field).unwrap_or_default();
            Some(*names.get(&key).ok_or(ErrorKind::InvalidArgument)?)
        }
        other => {
            let n = crate::coerce::value_to_number(other, a.ctx().settings())?.trunc();
            if n == 0.0 {
                None
            } else if n >= 1.0 && n <= f64::from(ncols) {
                Some(n as u32 - 1)
            } else {
                return Err(ErrorKind::InvalidArgument);
            }
        }
    };
    if field_col.is_none() && !zero_ok {
        return Err(ErrorKind::InvalidArgument);
    }

    // Criteria table: per row, a list of (db column offset, criterion).
    let settings = a.ctx().settings().clone();
    let mut alternatives: Vec<Vec<(u32, Criterion)>> = Vec::new();
    for r in (crit.r0 + 1)..=crit.r1 {
        let mut conds = Vec::new();
        for c in crit.c0..=crit.c1 {
            let Some(cell) = cdata.get(&(r, c)) else {
                continue;
            };
            let Some(col) = cdata
                .get(&(crit.r0, c))
                .and_then(header_text)
                .and_then(|n| names.get(&n).copied())
            else {
                continue;
            };
            if let Value::Error(e) = cell {
                return Err(*e);
            }
            conds.push((col, Criterion::parse(cell, &settings)));
        }
        alternatives.push(conds);
    }

    let mut sel = Selection {
        matched: 0,
        values: Vec::new(),
    };
    for r in (db.r0 + 1)..=db.r1 {
        let cell = |c: u32| data.get(&(r, db.c0 + c)).cloned().unwrap_or(Value::Empty);
        let ok = alternatives.is_empty()
            || alternatives
                .iter()
                .any(|conds| conds.iter().all(|(c, k)| k.matches(&cell(*c))));
        if ok {
            sel.matched += 1;
            if let Some(c) = field_col {
                sel.values.push(cell(c));
            }
        }
    }
    Ok(sel)
}

/// The numeric values of a selection (booleans count as numbers, text and
/// empties are skipped); an error value propagates.
pub(super) fn numbers(sel: &Selection) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for v in &sel.values {
        match v {
            Value::Number(n) => out.push(*n),
            Value::Bool(b) => out.push(if *b { 1.0 } else { 0.0 }),
            Value::Error(e) => return Err(*e),
            _ => {}
        }
    }
    Ok(out)
}

/// Shared test fixture: the AOO help-example style table in A1:E11.
#[cfg(test)]
pub(super) fn sample(c: &mut crate::mock::MockEvalContext) {
    let rows: [(&str, f64, f64, f64, f64); 10] = [
        ("Frank", 1.0, 12.0, 4.0, 40.0),
        ("Bernd", 2.0, 14.0, 5.0, 50.0),
        ("Andrew", 2.0, 14.0, 6.0, 60.0),
        ("Carl", 3.0, 15.0, 8.0, 80.0),
        ("Dora", 3.0, 16.0, 9.0, 90.0),
        ("Erik", 4.0, 17.0, 10.0, 100.0),
        ("Fay", 4.0, 18.0, 10.0, 110.0),
        ("Gus", 5.0, 19.0, 12.0, 120.0),
        ("Hal", 5.0, 20.0, 13.0, 130.0),
        ("Ivy", 6.0, 21.0, 14.0, 140.0),
    ];
    for (i, h) in ["Name", "Grade", "Age", "Distance", "Weight"]
        .iter()
        .enumerate()
    {
        c.set(0, &format!("{}1", (b'A' + i as u8) as char), *h);
    }
    for (i, (n, g, a, d, w)) in rows.iter().enumerate() {
        let r = i + 2;
        c.set(0, &format!("A{r}"), *n)
            .set(0, &format!("B{r}"), *g)
            .set(0, &format!("C{r}"), *a)
            .set(0, &format!("D{r}"), *d)
            .set(0, &format!("E{r}"), *w);
    }
}
