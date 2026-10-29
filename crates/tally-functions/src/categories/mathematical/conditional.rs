//! SUMIF, SUMIFS, COUNTIF, COUNTIFS, COUNTBLANK, AVERAGEIF, AVERAGEIFS.
//!
//! Criteria ranges are visited sparsely (`skip_empty = true`), so whole
//! column references cost what is filled. Empty cells are accounted for
//! arithmetically (COUNTIF) or through the sum range (SUMIF family: an empty
//! sum cell adds nothing), which keeps `COUNTIF(A:A; "")` and
//! `SUMIF(A:A; ""; B:B)` correct.
//!
//! Criteria are parsed in the document locale ([`Args::criterion`]), so
//! `">1,5"` and `"<19.10.2026"` are numbers under de-DE (AOO uses the
//! document's number formatter).
//!
//! Unions (`A1:A2~B1:B2`, [`EvalValue::areas`]): COUNTIF, COUNTBLANK and
//! SUMIF/AVERAGEIF without a sum range take every area in turn (AOO
//! `ScCountIf`, `ScCountEmptyCells` and `IterateParametersIf` iterate
//! `svRefList`); SUMIF/AVERAGEIF with a sum range reject a union with
//! `Err:504` (`IterateParametersIf`: `errIllegalParameter` when
//! `bSumExtraRange`). The `*IFS` functions take single areas (`#VALUE!`).
//!
//! 3D references (`Sheet1.A1:Sheet3.A5`): COUNTBLANK counts over every
//! sheet (`ScCountEmptyCells` iterates the tabs); COUNTIF, SUMIF and
//! AVERAGEIF give `Err:504` (`errIllegalParameter` when `Tab1 != Tab2`),
//! the `*IFS` functions `Err:502` (`errIllegalArgument`).

use std::collections::HashMap;
use std::ops::ControlFlow;

use tally_types::{CellAddr, CellRange, ErrorKind, Value, MAX_COLS, MAX_ROWS};

use crate::criteria::Criterion;
use crate::numeric::ApproxSum;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

const COUNTIF_PARAMS: &[Param] = &[
    Param::required("range", ParamKind::Reference),
    Param::required("criteria", ParamKind::Criteria),
];
const SUMIF_PARAMS: &[Param] = &[
    Param::required("range", ParamKind::Reference),
    Param::required("criteria", ParamKind::Criteria),
    Param::optional("sum_range", ParamKind::Reference),
];
const AVERAGEIF_PARAMS: &[Param] = &[
    Param::required("range", ParamKind::Reference),
    Param::required("criteria", ParamKind::Criteria),
    Param::optional("average_range", ParamKind::Reference),
];
const SUMIFS_PARAMS: &[Param] = &[
    Param::required("sum_range", ParamKind::Reference),
    Param::repeat("range", ParamKind::Reference),
    Param::repeat("criteria", ParamKind::Criteria),
];
const COUNTIFS_PARAMS: &[Param] = &[
    Param::repeat("range", ParamKind::Reference),
    Param::repeat("criteria", ParamKind::Criteria),
];
const AVERAGEIFS_PARAMS: &[Param] = &[
    Param::required("average_range", ParamKind::Reference),
    Param::repeat("range", ParamKind::Reference),
    Param::repeat("criteria", ParamKind::Criteria),
];
const COUNTBLANK_PARAMS: &[Param] = &[Param::required("range", ParamKind::Reference)];

/// `SUMIF(range; criteria; [sum_range])`.
pub const SUMIF: FunctionDef =
    FunctionDef::new("SUMIF", Category::Mathematical, SUMIF_PARAMS, sumif)
        .help("Totals the arguments that meet the conditions.");

/// `SUMIFS(sum_range; range; criteria; ...)`.
pub const SUMIFS: FunctionDef =
    FunctionDef::new("SUMIFS", Category::Mathematical, SUMIFS_PARAMS, sumifs).help(
        "Totals the values of cells in a range that meet multiple criteria in multiple ranges.",
    );

/// `COUNTIF(range; criteria)`.
pub const COUNTIF: FunctionDef =
    FunctionDef::new("COUNTIF", Category::Mathematical, COUNTIF_PARAMS, countif)
        .help("Counts the arguments which meet the set conditions.");

/// `COUNTBLANK(range)`.
pub const COUNTBLANK: FunctionDef = FunctionDef::new(
    "COUNTBLANK",
    Category::Mathematical,
    COUNTBLANK_PARAMS,
    countblank,
)
.help("Counts the blank cells in a specified range.");

/// `AVERAGEIF(range; criteria; [average_range])`.
pub const AVERAGEIF: FunctionDef = FunctionDef::new(
    "AVERAGEIF",
    Category::Mathematical,
    AVERAGEIF_PARAMS,
    averageif,
)
.help("Averages the arguments that meet the conditions.");

/// `COUNTIFS(range; criteria; ...)`.
pub const COUNTIFS: FunctionDef = FunctionDef::new(
    "COUNTIFS",
    Category::Mathematical,
    COUNTIFS_PARAMS,
    countifs,
)
.help("Counts the cells that meet multiple criteria in multiple ranges.");

/// `AVERAGEIFS(average_range; range; criteria; ...)`.
pub const AVERAGEIFS: FunctionDef = FunctionDef::new(
    "AVERAGEIFS",
    Category::Mathematical,
    AVERAGEIFS_PARAMS,
    averageifs,
)
.help("Averages the values of cells in a range that meet multiple criteria in multiple ranges.");

/// Which cells of one criteria range satisfy its criterion, keyed by the
/// offset from the range's top-left corner. Only non-empty cells are stored;
/// every other position is empty.
struct Matcher {
    nonempty: HashMap<(u32, u32), bool>,
    empty_ok: bool,
}

impl Matcher {
    fn build(ctx: &mut dyn EvalContext, range: &CellRange, crit: &Criterion) -> Matcher {
        let mut nonempty = HashMap::new();
        ctx.for_each_in_range(range, true, &mut |addr, v| {
            if !v.is_empty() {
                nonempty.insert((addr.row - range.r0, addr.col - range.c0), crit.matches(v));
            }
            ControlFlow::Continue(())
        });
        Matcher {
            nonempty,
            empty_ok: crit.matches(&Value::Empty),
        }
    }

    fn test(&self, off: (u32, u32)) -> bool {
        self.nonempty.get(&off).copied().unwrap_or(self.empty_ok)
    }
}

/// The range of `rows × cols` cells anchored at the top-left of `r`,
/// clipped to the sheet (AOO resizes `sum_range` this way).
fn resized(r: &CellRange, rows: u32, cols: u32) -> CellRange {
    CellRange::new(
        r.sheet,
        r.r0,
        r.c0,
        r.r0.saturating_add(rows - 1).min(MAX_ROWS - 1),
        r.c0.saturating_add(cols - 1).min(MAX_COLS - 1),
    )
}

/// Visits the cells of `target` (non-empty only) whose position passes every
/// matcher, calling `f` with the cell value. Errors are not filtered here.
fn for_each_selected(
    ctx: &mut dyn EvalContext,
    target: &CellRange,
    matchers: &[Matcher],
    mut f: impl FnMut(&Value) -> ControlFlow<ErrorKind>,
) -> Result<(), ErrorKind> {
    let mut failure = None;
    ctx.for_each_in_range(target, true, &mut |addr: CellAddr, v| {
        let off = (addr.row - target.r0, addr.col - target.c0);
        if matchers.iter().all(|m| m.test(off)) {
            if let ControlFlow::Break(e) = f(v) {
                failure = Some(e);
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    failure.map_or(Ok(()), Err)
}

/// Adds the numbers (booleans count as numbers) of `target` at selected
/// positions to `acc`. A selected error cell propagates.
fn accumulate(
    ctx: &mut dyn EvalContext,
    target: &CellRange,
    matchers: &[Matcher],
    acc: &mut ApproxSum,
) -> Result<(), ErrorKind> {
    for_each_selected(ctx, target, matchers, |v| {
        match v {
            Value::Number(x) => acc.add(*x),
            Value::Bool(b) => acc.add(if *b { 1.0 } else { 0.0 }),
            Value::Error(e) => return ControlFlow::Break(*e),
            _ => {}
        }
        ControlFlow::Continue(())
    })
}

/// Shared body of SUMIF and AVERAGEIF: `range; criteria; [target]`.
fn single_criterion(a: &mut Args) -> Result<ApproxSum, ErrorKind> {
    let areas = a.ranges(0)?;
    let crit = a.criterion(1)?;
    let sum_range = if a.is_missing(2) {
        None
    } else {
        if areas.len() > 1 {
            return Err(ErrorKind::ParamList);
        }
        Some(a.range(2)?)
    };
    let mut acc = ApproxSum::new();
    for range in &areas {
        let target = match &sum_range {
            Some(s) => resized(s, range.rows(), range.cols()),
            None => *range,
        };
        let m = Matcher::build(a.ctx(), range, &crit);
        accumulate(a.ctx(), &target, &[m], &mut acc)?;
    }
    Ok(acc)
}

fn sumif(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| Ok(Value::Number(single_criterion(&mut a)?.total())))
}

fn averageif(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let acc = single_criterion(&mut a)?;
        if acc.count() == 0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(acc.total() / acc.count() as f64))
    })
}

/// A range of the `*IFS` functions: a 3D reference is `Err:502` (AOO
/// `IterateParametersIfs`: `errIllegalArgument` when `Tab1 != Tab2`).
fn ifs_range(a: &mut Args, i: usize) -> Result<tally_types::CellRange, ErrorKind> {
    if a.any(i).is_3d() {
        return Err(ErrorKind::InvalidArgument);
    }
    a.range(i)
}

/// All ranges must have the dimensions of `sum_range`, else `Err:502`
/// (AOO `IterateParametersIfs`: `SetError(errIllegalArgument)`).
fn sumifs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let n = a.count();
        // AOO: an even parameter count is errParameterExpected (Err:511).
        if n < 3 || n.is_multiple_of(2) {
            return Err(ErrorKind::MissingVariable2);
        }
        let sum_range = ifs_range(&mut a, 0)?;
        let mut pairs = Vec::with_capacity(n / 2);
        for i in (1..n).step_by(2) {
            let range = ifs_range(&mut a, i)?;
            let crit = a.criterion(i + 1)?;
            if range.rows() != sum_range.rows() || range.cols() != sum_range.cols() {
                return Err(ErrorKind::InvalidArgument);
            }
            pairs.push((range, crit));
        }
        let matchers: Vec<Matcher> = pairs
            .iter()
            .map(|(r, c)| Matcher::build(a.ctx(), r, c))
            .collect();
        let mut acc = ApproxSum::new();
        accumulate(a.ctx(), &sum_range, &matchers, &mut acc)?;
        Ok(Value::Number(acc.total()))
    })
}

/// Reads `(range; criteria)` pairs starting at argument `first`; every range
/// must have `rows × cols` cells, else `Err:502` (AOO `IterateParametersIfs`
/// sets `errIllegalArgument`). An unpaired trailing
/// argument is `Err:511`.
fn criteria_pairs(
    a: &mut Args,
    first: usize,
    rows: u32,
    cols: u32,
) -> Result<Vec<Matcher>, ErrorKind> {
    let n = a.count();
    if n <= first || !(n - first).is_multiple_of(2) {
        return Err(ErrorKind::MissingVariable2);
    }
    let mut pairs = Vec::new();
    for i in (first..n).step_by(2) {
        let range = ifs_range(a, i)?;
        let crit = a.criterion(i + 1)?;
        if range.rows() != rows || range.cols() != cols {
            return Err(ErrorKind::InvalidArgument);
        }
        pairs.push((range, crit));
    }
    Ok(pairs
        .iter()
        .map(|(r, c)| Matcher::build(a.ctx(), r, c))
        .collect())
}

fn countifs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        if a.count() < 2 || !a.count().is_multiple_of(2) {
            return Err(ErrorKind::MissingVariable2);
        }
        let first = ifs_range(&mut a, 0)?;
        let (rows, cols) = (first.rows(), first.cols());
        let matchers = criteria_pairs(&mut a, 0, rows, cols)?;
        // Positions that are non-empty in at least one criteria range are
        // tested one by one; all the others are empty in every range and
        // share one verdict.
        let mut keys: Vec<(u32, u32)> = matchers
            .iter()
            .flat_map(|m| m.nonempty.keys().copied())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let mut hits = keys
            .iter()
            .filter(|k| matchers.iter().all(|m| m.test(**k)))
            .count() as u64;
        if matchers.iter().all(|m| m.empty_ok) {
            hits += first.cell_count() - keys.len() as u64;
        }
        Ok(Value::Number(hits as f64))
    })
}

fn averageifs(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        if a.count() < 3 || a.count() % 2 != 1 {
            return Err(ErrorKind::MissingVariable2);
        }
        let target = ifs_range(&mut a, 0)?;
        let matchers = criteria_pairs(&mut a, 1, target.rows(), target.cols())?;
        let mut acc = ApproxSum::new();
        accumulate(a.ctx(), &target, &matchers, &mut acc)?;
        if acc.count() == 0 {
            return Err(ErrorKind::Div0);
        }
        Ok(Value::Number(acc.total() / acc.count() as f64))
    })
}

/// Counts matching non-empty cells, plus all empty cells when the criterion
/// matches blanks (`""`, `"="`, `"<>5"`, ...).
fn countif(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let areas = a.ranges(0)?;
        let crit = a.criterion(1)?;
        let empty_ok = crit.matches(&Value::Empty);
        let mut hits = 0u64;
        for range in &areas {
            let mut visited = 0u64;
            a.ctx().for_each_in_range(range, true, &mut |_, v| {
                if !v.is_empty() {
                    visited += 1;
                    if crit.matches(v) {
                        hits += 1;
                    }
                }
                ControlFlow::Continue(())
            });
            if empty_ok {
                hits += range.cell_count() - visited;
            }
        }
        Ok(Value::Number(hits as f64))
    })
}

/// Counts empty cells and cells holding the empty string.
fn countblank(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let blank = |v: &Value| match v {
            Value::Empty => true,
            Value::Text(s) => s.is_empty(),
            _ => false,
        };
        let arg = a.any(0);
        if let Some(areas) = arg.areas() {
            let mut blanks = 0u64;
            for range in areas {
                let mut nonblank = 0u64;
                a.ctx().for_each_in_range(range, true, &mut |_, v| {
                    if !blank(v) {
                        nonblank += 1;
                    }
                    ControlFlow::Continue(())
                });
                blanks += range.cell_count() - nonblank;
            }
            return Ok(Value::Number(blanks as f64));
        }
        match arg {
            EvalValue::Array(m) => Ok(Value::Number(
                m.data.iter().filter(|v| blank(v)).count() as f64
            )),
            EvalValue::Scalar(Value::Error(e)) => Err(e),
            _ => Err(ErrorKind::Value),
        }
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SUMIF);
    r.register(SUMIFS);
    r.register(COUNTIF);
    r.register(COUNTBLANK);
    r.register(AVERAGEIF);
    r.register(COUNTIFS);
    r.register(AVERAGEIFS);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn num(x: f64) -> Value {
        Value::Number(x)
    }
    fn err(e: ErrorKind) -> Value {
        Value::Error(e)
    }
    fn t(s: &str) -> EvalValue {
        EvalValue::text(s)
    }

    /// A1:A6 = 1, "x", TRUE, #DIV/0!, empty, 5; B1:B6 = 10, 20, 30, 40, 50, 60.
    fn grid() -> MockEvalContext {
        let mut c = MockEvalContext::new();
        c.set(0, "A1", 1.0)
            .set(0, "A2", "x")
            .set(0, "A3", true)
            .set(0, "A4", Value::Error(ErrorKind::Div0))
            .set(0, "A6", 5.0);
        for (i, v) in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0].iter().enumerate() {
            c.set(0, &format!("B{}", i + 1), *v);
        }
        c
    }

    #[test]
    fn countif() {
        let mut c = grid();
        let a = c.range(0, "A1:A6");
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a.clone(), EvalValue::number(1.0)]),
            num(2.0)
        );
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t(">1")]), num(1.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t("x")]), num(1.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t("")]), num(1.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t("=")]), num(1.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t("<>")]), num(5.0));
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a.clone(), t("#DIV/0!")]),
            num(1.0)
        );
        let col = c.range(0, "A1:A1048576");
        assert_eq!(
            call(&mut c, "COUNTIF", vec![col, t("")]),
            num(1_048_576.0 - 5.0)
        );
        assert_eq!(
            call(&mut c, "COUNTIF", vec![EvalValue::number(1.0), t("1")]),
            err(ErrorKind::Value)
        );
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a, EvalValue::error(ErrorKind::Num)]),
            err(ErrorKind::Num)
        );
    }

    #[test]
    fn sumif_family() {
        let mut c = grid();
        let a = c.range(0, "A1:A6");
        let b = c.range(0, "B1:B6");
        assert_eq!(call(&mut c, "SUMIF", vec![a.clone(), t(">0")]), num(7.0));
        assert_eq!(
            call(&mut c, "SUMIF", vec![a.clone(), t(">0"), b.clone()]),
            num(10.0 + 30.0 + 60.0)
        );
        assert_eq!(
            call(&mut c, "SUMIF", vec![a.clone(), t(""), b.clone()]),
            num(50.0)
        );
        // A column-sized sum_range is resized to the criteria range's shape.
        let b3 = c.range(0, "B3");
        assert_eq!(
            call(&mut c, "SUMIF", vec![a.clone(), t("x"), b3]),
            num(40.0)
        );
        // A selected error cell propagates, an unselected one does not.
        assert_eq!(
            call(&mut c, "SUMIF", vec![b.clone(), t(">0"), a.clone()]),
            err(ErrorKind::Div0)
        );
        assert_eq!(
            call(&mut c, "SUMIF", vec![b.clone(), t("<35"), a.clone()]),
            num(2.0)
        );
        // SUMIFS
        assert_eq!(
            call(
                &mut c,
                "SUMIFS",
                vec![b.clone(), a.clone(), t(">0"), b.clone(), t(">=30")]
            ),
            num(90.0)
        );
        let short = c.range(0, "A1:A5");
        assert_eq!(
            call(&mut c, "SUMIFS", vec![b.clone(), short, t(">0")]),
            err(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "SUMIFS", vec![b.clone(), a.clone()]),
            err(ErrorKind::MissingVariable2)
        );
        // AVERAGEIF
        assert_eq!(
            call(&mut c, "AVERAGEIF", vec![a.clone(), t(">0"), b.clone()]),
            num(100.0 / 3.0)
        );
        assert_eq!(
            call(&mut c, "AVERAGEIF", vec![a.clone(), t(">100"), b]),
            err(ErrorKind::Div0)
        );
        assert_eq!(call(&mut c, "AVERAGEIF", vec![a, t(">0")]), num(7.0 / 3.0));
    }

    /// Review 3 H2: criteria operands are read in the document locale.
    #[test]
    fn criteria_in_document_locale() {
        let mut c = MockEvalContext::new();
        c.set_locale("de-DE");
        c.set(0, "A1", 1.0).set(0, "A2", 2.0).set(0, "A3", 1.7);
        c.set(0, "A4", 46314.0).set(0, "A5", 46204.0); // 19.10.2026, 1.7.2026
        let a = c.range(0, "A1:A3");
        let d = c.range(0, "A4:A5");
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a.clone(), t(">1,5")]),
            num(2.0)
        );
        // Under de-DE `1.5` is 1 May (as AOO reads it), so no number is
        // above that date serial; ISO dates and plain decimals stay readable.
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a.clone(), t(">1.5")]),
            num(0.0)
        );
        assert_eq!(
            call(&mut c, "COUNTIF", vec![a.clone(), t(">1,5E0")]),
            num(2.0)
        );
        assert_eq!(call(&mut c, "COUNTIF", vec![a.clone(), t("1,7")]), num(1.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![a, t(">1.000")]), num(0.0));
        assert_eq!(
            call(&mut c, "SUMIF", vec![d.clone(), t("<19.10.2026")]),
            num(46204.0)
        );
        assert_eq!(
            call(&mut c, "SUMIF", vec![d.clone(), t("<2026-10-19")]),
            num(46204.0)
        );
        // Month-day-year is text under de-DE: no date matches it.
        assert_eq!(call(&mut c, "SUMIF", vec![d, t("<10/19/2026")]), num(0.0));
    }

    /// Review 3 M1: unions in the criteria range.
    #[cfg(feature = "ref-union")]
    #[test]
    fn unions() {
        let mut c = grid();
        let u = c.union(0, &["A1:A3", "B1:B2"]);
        assert_eq!(call(&mut c, "COUNTIF", vec![u.clone(), t(">1")]), num(2.0));
        assert_eq!(call(&mut c, "COUNTIF", vec![u.clone(), t("")]), num(0.0));
        assert_eq!(call(&mut c, "SUMIF", vec![u.clone(), t(">0")]), num(32.0));
        assert_eq!(
            call(&mut c, "AVERAGEIF", vec![u.clone(), t(">0")]),
            num(32.0 / 4.0)
        );
        let b = c.range(0, "B1:B6");
        assert_eq!(
            call(&mut c, "SUMIF", vec![u.clone(), t(">0"), b.clone()]),
            err(ErrorKind::ParamList)
        );
        assert_eq!(
            call(&mut c, "SUMIFS", vec![b, u.clone(), t(">0")]),
            err(ErrorKind::Value)
        );
        let blanks = c.union(0, &["A5:A6", "C1:C2"]);
        assert_eq!(call(&mut c, "COUNTBLANK", vec![blanks]), num(3.0));
        assert_eq!(call(&mut c, "COUNTBLANK", vec![u]), num(0.0));
    }

    #[test]
    fn countblank() {
        let mut c = grid();
        c.set(0, "C1", "");
        let a = c.range(0, "A1:A6");
        assert_eq!(call(&mut c, "COUNTBLANK", vec![a]), num(1.0));
        let r = c.range(0, "C1:C3");
        assert_eq!(call(&mut c, "COUNTBLANK", vec![r]), num(3.0));
        assert_eq!(
            call(&mut c, "COUNTBLANK", vec![EvalValue::number(1.0)]),
            err(ErrorKind::Value)
        );
    }

    #[test]
    fn countifs_averageifs() {
        let mut c = grid();
        let a = c.range(0, "A1:A6");
        let b = c.range(0, "B1:B6");
        assert_eq!(
            call(
                &mut c,
                "COUNTIFS",
                vec![a.clone(), t(">0"), b.clone(), t(">=30")]
            ),
            num(2.0)
        );
        assert_eq!(call(&mut c, "COUNTIFS", vec![a.clone(), t("")]), num(1.0));
        assert_eq!(
            call(
                &mut c,
                "COUNTIFS",
                vec![a.clone(), t(""), b.clone(), t(">=50")]
            ),
            num(1.0)
        );
        let short = c.range(0, "A1:A5");
        assert_eq!(
            call(&mut c, "COUNTIFS", vec![a.clone(), t(">0"), short, t(">0")]),
            err(ErrorKind::InvalidArgument)
        );
        assert_eq!(
            call(&mut c, "COUNTIFS", vec![a.clone()]),
            err(ErrorKind::MissingVariable2)
        );
        assert_eq!(
            call(&mut c, "COUNTIFS", vec![a.clone(), t(">0"), b.clone()]),
            err(ErrorKind::MissingVariable2)
        );
        assert_eq!(
            call(&mut c, "AVERAGEIFS", vec![b.clone(), a.clone(), t(">0")]),
            num(100.0 / 3.0)
        );
        assert_eq!(
            call(&mut c, "AVERAGEIFS", vec![b.clone(), a.clone(), t(">100")]),
            err(ErrorKind::Div0)
        );
        assert_eq!(
            call(&mut c, "AVERAGEIFS", vec![b.clone(), a]),
            err(ErrorKind::MissingVariable2)
        );
    }
}
