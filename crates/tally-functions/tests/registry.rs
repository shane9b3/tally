//! Registry lookups, metadata consistency and arity errors.

#![allow(clippy::unwrap_used)]

mod common;
use common::*;

use std::collections::HashMap;

use serde::Deserialize;
use tally_functions::{
    build_registry, check_arity, registry, run, ArgList, Args, Category, EvalContext, FnFlags,
    FuncId, FunctionDef, Param, ParamKind, Registry,
};

#[derive(Deserialize)]
struct MasterList {
    function: Vec<MasterEntry>,
}

#[derive(Deserialize)]
struct MasterEntry {
    name: String,
    category: String,
    odf_name: String,
    min_args: u8,
    max_args: u8,
    batch: Option<u8>,
    extension: bool,
}

fn master() -> HashMap<String, MasterEntry> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/functions/master-list.toml"
    );
    let text = std::fs::read_to_string(path).unwrap();
    let list: MasterList = toml::from_str(&text).unwrap();
    list.function
        .into_iter()
        .map(|f| (f.name.clone(), f))
        .collect()
}

#[test]
fn core_functions_registered_without_conflicts() {
    let r = registry();
    assert!(r.conflicts().is_empty(), "{:?}", r.conflicts());
    for name in ["SUM", "ROUND", "AVERAGE", "COUNT", "IF", "CONCATENATE"] {
        assert!(r.lookup(name).is_some(), "{name}");
    }
    assert_eq!(r.len(), r.iter().count());
    for (id, def) in r.iter() {
        assert_eq!(r.get(id).map(|d| d.name), Some(def.name));
        assert_eq!(r.lookup(def.name), Some(id));
        assert_eq!(def.name, def.name.to_ascii_uppercase());
        assert!(!def.help.is_empty(), "{} has no help", def.name);
    }
    assert!(r.get(FuncId(u32::MAX)).is_none());
}

#[test]
fn registry_matches_master_list() {
    let m = master();
    assert_eq!(m.len(), 407);
    assert_eq!(m.values().filter(|f| f.batch == Some(0)).count(), 68);
    assert!(m["IFERROR"].extension && m["IFNA"].extension);
    for (_, def) in registry().iter() {
        let e = m
            .get(def.name)
            .unwrap_or_else(|| panic!("{} not in master list", def.name));
        assert_eq!(def.category.aoo_name(), e.category, "{}", def.name);
        assert_eq!(def.odf_name, e.odf_name, "{}", def.name);
        assert_eq!(def.min_args, e.min_args, "{}", def.name);
        assert_eq!(def.max_args, e.max_args, "{}", def.name);
        assert_eq!(def.extension, e.extension, "{}", def.name);
    }
}

#[test]
fn lookup_is_case_insensitive() {
    let r = registry();
    assert_eq!(r.lookup("sum"), r.lookup("SUM"));
    assert_eq!(r.lookup("Concatenate"), r.lookup("CONCATENATE"));
    assert_eq!(r.lookup("NOPE"), None);
}

fn dummy(ctx: &mut dyn EvalContext, args: &mut dyn ArgList) -> EvalValue {
    let a = Args::new(ctx, args);
    run(|| Ok(Value::Number(a.count() as f64)))
}

const ONE: &[Param] = &[Param::required("x", ParamKind::Number)];
const IFS_PARAMS: &[Param] = &[
    Param::repeat("test", ParamKind::Bool),
    Param::repeat("value", ParamKind::Lazy),
];

fn sample() -> (Registry, FuncId, FuncId, FuncId) {
    let mut r = Registry::new();
    let ifs = r.register(
        FunctionDef::new("IFS", Category::Logical, IFS_PARAMS, dummy)
            .odf("COM.MICROSOFT.IFS")
            .ooxml("_xlfn.IFS")
            .extension()
            .help("x"),
    );
    let weeks = r.register(
        FunctionDef::new("WEEKS", Category::AddIn, ONE, dummy).odf("ORG.OPENOFFICE.WEEKS"),
    );
    let iferror = r.register(
        FunctionDef::new("IFERROR", Category::Logical, ONE, dummy).odf("COM.MICROSOFT.IFERROR"),
    );
    (r, ifs, weeks, iferror)
}

#[test]
fn lookup_by_all_three_name_kinds() {
    let (r, ifs, weeks, iferror) = sample();
    assert_eq!(r.lookup("IFS"), Some(ifs));
    assert_eq!(r.lookup_odf("COM.MICROSOFT.IFS"), Some(ifs));
    assert_eq!(r.lookup_odf("com.microsoft.ifs"), Some(ifs));
    assert_eq!(r.lookup_ooxml("_xlfn.IFS"), Some(ifs));
    assert_eq!(r.lookup_ooxml("_XLFN.ifs"), Some(ifs));
    assert_eq!(r.lookup_odf("ORG.OPENOFFICE.WEEKS"), Some(weeks));
    assert_eq!(r.lookup_odf("WEEKS"), None);
    assert_eq!(r.lookup_odf("COM.MICROSOFT.IFERROR"), Some(iferror));
    // OOXML falls back to canonical names, with or without the prefix.
    assert_eq!(r.lookup_ooxml("IFERROR"), Some(iferror));
    assert_eq!(r.lookup_ooxml("_xlfn.WEEKS"), Some(weeks));
    assert_eq!(r.lookup_any("ORG.OPENOFFICE.WEEKS"), Some(weeks));
    assert_eq!(r.lookup_any("_xlfn.IFS"), Some(ifs));
    assert_eq!(r.lookup_any("weeks"), Some(weeks));
    let ids: Vec<u32> = r.iter().map(|(id, _)| id.0).collect();
    assert_eq!(ids, vec![0, 1, 2], "ids are dense, in registration order");
    let global = registry();
    assert_eq!(global.lookup_odf("SUM"), global.lookup("SUM"));
    assert_eq!(global.lookup_ooxml("SUM"), global.lookup("SUM"));
}

#[test]
fn duplicate_names_are_recorded_not_panicking() {
    let (mut r, ifs, _, _) = sample();
    let dup = r.register(FunctionDef::new("ifs", Category::Logical, ONE, dummy).odf("X.IFS"));
    assert_eq!(r.lookup("IFS"), Some(ifs), "first registration wins");
    assert_eq!(r.lookup_odf("X.IFS"), Some(dup));
    assert_eq!(r.conflicts().len(), 1);
    assert_eq!(r.conflicts()[0].table, "name");
    assert_eq!(r.conflicts()[0].rejected, dup);
}

#[test]
fn derived_arity_and_params() {
    let (r, ifs, _, _) = sample();
    let def = r.get(ifs).unwrap();
    assert_eq!((def.min_args, def.max_args), (2, 255));
    assert!(def.is_variadic());
    assert_eq!(def.param(0).unwrap().name, "test");
    assert_eq!(def.param(3).unwrap().name, "value");
    assert_eq!(def.param(4).unwrap().name, "test", "pair repeats");
    assert_eq!(def.param_kind(5), ParamKind::Lazy);
    let round = registry().def_by_name("ROUND").unwrap();
    assert_eq!((round.min_args, round.max_args), (1, 2));
    assert!(round.param(2).is_none());
    assert_eq!(round.param_kind(2), ParamKind::Any);
    let iff = registry().def_by_name("IF").unwrap();
    assert!(iff.flags.contains(FnFlags::LAZY_ARGS));
    assert!(!iff.flags.contains(FnFlags::VOLATILE));
    assert!((FnFlags::VOLATILE | FnFlags::FORCE_ARRAY).contains(FnFlags::FORCE_ARRAY));
    assert!(!iff.param_forces_array(0));
}

#[test]
fn signatures() {
    let r = registry();
    let sig = |n: &str| r.def_by_name(n).unwrap().signature();
    assert_eq!(sig("SUM"), "SUM(number; ...)");
    assert_eq!(sig("ROUND"), "ROUND(number; [count])");
    assert_eq!(sig("IF"), "IF(test; [then_value]; [otherwise_value])");
    assert_eq!(sig("CONCATENATE"), "CONCATENATE(text; ...)");
    const SUMIF: &[Param] = &[
        Param::required("range", ParamKind::Reference),
        Param::required("criteria", ParamKind::Criteria),
        Param::optional("sum_range", ParamKind::Reference),
    ];
    let def = FunctionDef::new("SUMIF", Category::Mathematical, SUMIF, dummy);
    assert_eq!(def.signature(), "SUMIF(range; criteria; [sum_range])");
    assert_eq!((def.min_args, def.max_args), (2, 3));
}

#[test]
fn categories() {
    assert_eq!(
        Category::from_aoo_name("Date&Time"),
        Some(Category::DateTime)
    );
    assert_eq!(Category::from_aoo_name("Add-in"), Some(Category::AddIn));
    assert_eq!(Category::from_aoo_name("Math"), None);
    for c in Category::ALL {
        assert_eq!(Category::from_aoo_name(c.aoo_name()), Some(c));
        assert_eq!(c.to_string(), c.aoo_name());
    }
    assert_eq!(Category::DateTime.dir_name(), "datetime");
}

#[test]
fn arity_errors() {
    let r = registry();
    let round = r.def_by_name("ROUND").unwrap();
    assert_eq!(check_arity(round, 0), Err(ErrorKind::MissingVariable2));
    assert_eq!(check_arity(round, 1), Ok(()));
    assert_eq!(check_arity(round, 2), Ok(()));
    assert_eq!(check_arity(round, 3), Err(ErrorKind::ParamList));
    let sum = r.def_by_name("SUM").unwrap();
    assert_eq!(check_arity(sum, 0), Err(ErrorKind::MissingVariable2));
    assert_eq!(check_arity(sum, 254), Ok(()));
    assert_eq!(check_arity(sum, 1000), Ok(()));

    let mut c = MockEvalContext::new();
    assert_eq!(
        call(&mut c, "ROUND", vec![]),
        err(ErrorKind::MissingVariable2)
    );
    assert_eq!(
        call(&mut c, "ROUND", vec![n(1.0), n(2.0), n(3.0)]),
        err(ErrorKind::ParamList)
    );
    assert_eq!(call(&mut c, "IF", vec![]), err(ErrorKind::MissingVariable2));
    assert_eq!(
        call(&mut c, "IF", vec![b(true), n(1.0), n(2.0), n(3.0)]),
        err(ErrorKind::ParamList)
    );
    assert_eq!(
        call(&mut c, "SUM", vec![]),
        err(ErrorKind::MissingVariable2)
    );
    assert_eq!(
        call(&mut c, "CONCATENATE", vec![]),
        err(ErrorKind::MissingVariable2)
    );
    // Arity errors are reported before any argument is evaluated.
    let mut args = MockArgs::new(vec![n(1.0), n(2.0), n(3.0)]);
    call_with(&mut c, "ROUND", &mut args);
    assert!(args.evaluated.is_empty());
}

#[test]
fn unknown_id_is_name_error() {
    let mut c = MockEvalContext::new();
    let mut args = MockArgs::new(vec![]);
    let v = registry().eval_call(&mut c, FuncId(9_999), &mut args);
    assert_eq!(v, e(ErrorKind::Name));
    assert!(build_registry().len() == registry().len());
}
