//! Depreciation: SLN, SYD, DB, DDB, VDB.
//!
//! Ported from Apache OpenOffice `sc/source/core/tool/interpr2.cxx`
//! (`ScLIA`, `ScDIA`, `ScGDA2`, `ScGDA`/`ScGetGDA`, `ScVDB`/`ScInterVDB`),
//! Apache License 2.0.

use tally_types::{approx_equal, ErrorKind, Value};

use super::opt_num;
use crate::coerce::approx_floor;
use crate::registry::{Category, FunctionDef, Param, ParamKind, Registry};
use crate::{run, ArgList, Args, EvalContext, EvalValue};

fn approx_ceil(x: f64) -> f64 {
    -approx_floor(-x)
}

const SLN_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("salvage", ParamKind::Number),
    Param::required("life", ParamKind::Number),
];

/// `SLN(cost; salvage; life)`.
pub const SLN: FunctionDef = FunctionDef::new("SLN", Category::Financial, SLN_PARAMS, sln)
    .help("Calculates the linear depreciation per period.");

fn sln(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let cost = a.num(0)?;
        let salvage = a.num(1)?;
        let life = a.num(2)?;
        Ok(Value::Number((cost - salvage) / life))
    })
}

const SYD_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("salvage", ParamKind::Number),
    Param::required("life", ParamKind::Number),
    Param::required("period", ParamKind::Number),
];

/// `SYD(cost; salvage; life; period)`.
pub const SYD: FunctionDef = FunctionDef::new("SYD", Category::Financial, SYD_PARAMS, syd)
    .help("Calculates the arithmetically declining value of an asset (depreciation) for a specified period.");

fn syd(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let cost = a.num(0)?;
        let salvage = a.num(1)?;
        let life = a.num(2)?;
        let per = a.num(3)?;
        Ok(Value::Number(
            ((cost - salvage) * (life - per + 1.0)) / ((life * (life + 1.0)) / 2.0),
        ))
    })
}

const DB_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("salvage", ParamKind::Number),
    Param::required("life", ParamKind::Number),
    Param::required("period", ParamKind::Number),
    Param::optional("month", ParamKind::Integer),
];

/// `DB(cost; salvage; life; period; [month])`.
pub const DB: FunctionDef = FunctionDef::new("DB", Category::Financial, DB_PARAMS, db)
    .help("Returns the real depreciation of an asset for a specified period using the fixed-declining balance method.");

fn db(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let cost = a.num(0)?;
        let salvage = a.num(1)?;
        let life = a.num(2)?;
        let per = a.num(3)?;
        let months = if a.count() > 4 {
            approx_floor(a.num(4)?)
        } else {
            12.0
        };
        if !(1.0..=12.0).contains(&months)
            || life > 1200.0
            || salvage < 0.0
            || per > life + 1.0
            || salvage > cost
            || cost < 0.0
        {
            return Err(ErrorKind::InvalidArgument);
        }
        let rate = 1.0 - (salvage / cost).powf(1.0 / life);
        let rate = approx_floor((rate * 1000.0) + 0.5) / 1000.0;
        let first = cost * rate * months / 12.0;
        let mut out = 0.0;
        if approx_floor(per) == 1.0 {
            out = first;
        } else {
            let mut sum = first;
            let min = if life > per { per } else { life };
            let imax = approx_floor(min) as u32;
            for _ in 2..=imax {
                out = (cost - sum) * rate;
                sum += out;
            }
            if per > life {
                out = ((cost - sum) * rate * (12.0 - months)) / 12.0;
            }
        }
        Ok(Value::Number(out))
    })
}

/// `ScGetGDA`: double-declining balance for one period.
fn gda(cost: f64, salvage: f64, life: f64, per: f64, factor: f64) -> f64 {
    let mut rate = factor / life;
    let old;
    if rate >= 1.0 {
        rate = 1.0;
        old = if per == 1.0 { cost } else { 0.0 };
    } else {
        old = cost * (1.0 - rate).powf(per - 1.0);
    }
    let new = cost * (1.0 - rate).powf(per);
    let d = if new < salvage {
        old - salvage
    } else {
        old - new
    };
    if d < 0.0 {
        0.0
    } else {
        d
    }
}

const DDB_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("salvage", ParamKind::Number),
    Param::required("life", ParamKind::Number),
    Param::required("period", ParamKind::Number),
    Param::optional("factor", ParamKind::Number),
];

/// `DDB(cost; salvage; life; period; [factor])`.
pub const DDB: FunctionDef = FunctionDef::new("DDB", Category::Financial, DDB_PARAMS, ddb)
    .help("Calculates the depreciation of an asset for a specific period using the double-declining balance method or declining balance factor.");

fn ddb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let cost = a.num(0)?;
        let salvage = a.num(1)?;
        let life = a.num(2)?;
        let per = a.num(3)?;
        let factor = opt_num(&mut a, 4, 2.0)?;
        if cost < 0.0 || salvage < 0.0 || factor <= 0.0 || salvage > cost || per < 1.0 || per > life
        {
            return Err(ErrorKind::InvalidArgument);
        }
        Ok(Value::Number(gda(cost, salvage, life, per, factor)))
    })
}

/// `ScInterVDB`.
fn inter_vdb(cost: f64, salvage: f64, life: f64, life1: f64, period: f64, factor: f64) -> f64 {
    let int_end = approx_ceil(period);
    let loop_end = int_end as u64;
    let mut vdb = 0.0;
    let mut rest = cost - salvage;
    let mut now_sln = false;
    let mut sln = 0.0;
    for i in 1..=loop_end {
        let mut term;
        if !now_sln {
            let g = gda(cost, salvage, life, i as f64, factor);
            sln = rest / (life1 - (i - 1) as f64);
            if sln > g {
                term = sln;
                now_sln = true;
            } else {
                term = g;
                rest -= g;
            }
        } else {
            term = sln;
        }
        if i == loop_end {
            term *= period + 1.0 - int_end;
        }
        vdb += term;
    }
    vdb
}

const VDB_PARAMS: &[Param] = &[
    Param::required("cost", ParamKind::Number),
    Param::required("salvage", ParamKind::Number),
    Param::required("life", ParamKind::Number),
    Param::required("start", ParamKind::Number),
    Param::required("end", ParamKind::Number),
    Param::optional("factor", ParamKind::Number),
    Param::optional("no_switch", ParamKind::Bool),
];

/// `VDB(cost; salvage; life; start; end; [factor]; [no_switch])`.
pub const VDB: FunctionDef = FunctionDef::new("VDB", Category::Financial, VDB_PARAMS, vdb)
    .help("Variable declining balance. Returns the declining balance depreciation for a particular period.");

/// Upper bound on VDB loop iterations (periods); AOO has none, but a
/// period count beyond this is not a meaningful depreciation schedule.
const VDB_MAX_PERIODS: f64 = 1.0e7;

fn vdb(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let mut a = Args::new(ctx, args);
    run(|| {
        let mut cost = a.num(0)?;
        let salvage = a.num(1)?;
        let life = a.num(2)?;
        let mut start = a.num(3)?;
        let mut end = a.num(4)?;
        let factor = opt_num(&mut a, 5, 2.0)?;
        // AOO GetBool: a number, nonzero is TRUE.
        let no_switch = opt_num(&mut a, 6, 0.0)? != 0.0;
        if start < 0.0 || end < start || end > life || cost < 0.0 || salvage > cost || factor <= 0.0
        {
            return Err(ErrorKind::InvalidArgument);
        }
        if end > VDB_MAX_PERIODS {
            return Err(ErrorKind::InvalidArgument);
        }
        let int_start = approx_floor(start);
        let int_end = approx_ceil(end);
        let loop_start = int_start as u64;
        let loop_end = int_end as u64;
        let mut out = 0.0;
        if no_switch {
            for i in (loop_start + 1)..=loop_end {
                let mut term = gda(cost, salvage, life, i as f64, factor);
                if i == loop_start + 1 {
                    term *= end.min(int_start + 1.0) - start;
                } else if i == loop_end {
                    term *= end + 1.0 - int_end;
                }
                out += term;
            }
        } else {
            let mut life1 = life;
            if !approx_equal(start, approx_floor(start))
                && factor > 1.0
                && (start > life / 2.0 || approx_equal(start, life / 2.0))
            {
                let part = start - life / 2.0;
                start = life / 2.0;
                end -= part;
                life1 += 1.0;
            }
            cost -= inter_vdb(cost, salvage, life, life1, start, factor);
            out = inter_vdb(cost, salvage, life, life - start, end - start, factor);
        }
        Ok(Value::Number(out))
    })
}

pub(super) fn register(r: &mut Registry) {
    r.register(SLN);
    r.register(SYD);
    r.register(DB);
    r.register(DDB);
    r.register(VDB);
}

#[cfg(test)]
mod tests {
    use crate::mock::{call, MockEvalContext};
    use crate::EvalValue;
    use tally_types::{ErrorKind, Value};

    fn num(c: &mut MockEvalContext, f: &str, args: &[f64]) -> f64 {
        match call(c, f, args.iter().map(|&x| EvalValue::number(x)).collect()) {
            Value::Number(v) => v,
            other => panic!("{f}: {other:?}"),
        }
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn straight_and_syd() {
        let mut c = MockEvalContext::new();
        close(num(&mut c, "SLN", &[30000.0, 7500.0, 10.0]), 2250.0, 0.0);
        close(
            num(&mut c, "SYD", &[30000.0, 7500.0, 10.0, 1.0]),
            4090.909090909,
            1e-8,
        );
        close(
            num(&mut c, "SYD", &[30000.0, 7500.0, 10.0, 10.0]),
            409.090909091,
            1e-8,
        );
    }

    #[test]
    fn declining_balance() {
        let mut c = MockEvalContext::new();
        close(
            num(&mut c, "DB", &[1000000.0, 100000.0, 6.0, 1.0, 7.0]),
            186083.333333,
            1e-5,
        );
        close(
            num(&mut c, "DB", &[1000000.0, 100000.0, 6.0, 7.0, 7.0]),
            15845.098473,
            1e-5,
        );
        close(
            num(&mut c, "DDB", &[2400.0, 300.0, 3650.0, 1.0]),
            1.315068493,
            1e-8,
        );
        close(
            num(&mut c, "DDB", &[2400.0, 300.0, 10.0, 1.0, 2.0]),
            480.0,
            1e-9,
        );
        close(
            num(&mut c, "DDB", &[2400.0, 300.0, 10.0, 10.0]),
            22.1225472,
            1e-6,
        );
        assert_eq!(
            call(
                &mut c,
                "DDB",
                [2400.0, 300.0, 10.0, 11.0].map(EvalValue::number).to_vec()
            ),
            Value::Error(ErrorKind::InvalidArgument)
        );
    }

    #[test]
    fn vdb() {
        let mut c = MockEvalContext::new();
        close(
            num(&mut c, "VDB", &[2400.0, 300.0, 3650.0, 0.0, 1.0]),
            1.315068493,
            1e-8,
        );
        close(
            num(&mut c, "VDB", &[2400.0, 300.0, 120.0, 0.0, 1.0]),
            40.0,
            1e-9,
        );
        close(
            num(&mut c, "VDB", &[2400.0, 300.0, 10.0, 0.0, 0.875, 1.5]),
            315.0,
            1e-9,
        );
        close(
            num(&mut c, "VDB", &[2400.0, 300.0, 120.0, 6.0, 18.0]),
            396.306053,
            1e-5,
        );
        close(
            num(&mut c, "VDB", &[2400.0, 300.0, 120.0, 6.0, 18.0, 1.5]),
            311.808937,
            1e-5,
        );
    }
}
