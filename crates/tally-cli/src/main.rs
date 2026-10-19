//! `tally-cli`: command-line front end for the Tally engine.
//!
//! ```text
//! tally-cli eval [--cells 'A1=2,B1=3'] [--at C1] '<formula>'
//! tally-cli recalc <file.ods|file.csv>
//! tally-cli functions
//! ```
//!
//! Gate G1: `tally-cli eval --cells 'A1=2,B1=3' '=SUM(A1:B1)*2'` prints `10`.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use tally_api::DocumentService;
use tally_engine::recalc::recalc;
use tally_engine::{EngineService, Workbook};
use tally_numfmt::{general_string, EN_US};
use tally_types::{CellAddr, Value};

const USAGE: &str = "\
usage:
  tally-cli eval [--cells 'A1=2,B1=3'] [--at CELL] '<formula>'
  tally-cli recalc <file.ods|file.csv>
  tally-cli functions";

#[derive(Debug, PartialEq)]
enum Cmd {
    Eval {
        cells: Vec<(String, String)>,
        at: Option<String>,
        formula: String,
    },
    Recalc {
        path: String,
    },
    Functions,
    Help,
}

/// Splits `A1=2,B1=3,C1==SUM(A1,B1)` into `(cell, input)` pairs. A comma
/// only separates entries when the text after it starts with `CELL=`.
fn parse_cells(spec: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for piece in spec.split(',') {
        let starts_entry = piece.split_once('=').is_some_and(|(k, _)| {
            let k = k.trim();
            let k = k.rsplit_once(['.', '!']).map_or(k, |(_, a)| a);
            tally_types::parse_a1(k).is_some()
        });
        match (starts_entry, out.last_mut()) {
            (true, _) => {
                let (k, v) = piece.split_once('=').ok_or("bad --cells entry")?;
                out.push((k.trim().to_owned(), v.to_owned()));
            }
            (false, Some(last)) => {
                last.1.push(',');
                last.1.push_str(piece);
            }
            (false, None) => {
                return Err(format!("bad --cells entry {piece:?} (expected CELL=VALUE)"))
            }
        }
    }
    Ok(out)
}

fn parse_args(args: &[String]) -> Result<Cmd, String> {
    let Some(cmd) = args.first() else {
        return Ok(Cmd::Help);
    };
    match cmd.as_str() {
        "eval" => {
            let mut cells = Vec::new();
            let mut at = None;
            let mut formula = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--cells" => {
                        i += 1;
                        let spec = args.get(i).ok_or("--cells needs a value")?;
                        cells.extend(parse_cells(spec)?);
                    }
                    "--at" => {
                        i += 1;
                        at = Some(args.get(i).ok_or("--at needs a cell")?.clone());
                    }
                    other if formula.is_none() => formula = Some(other.to_owned()),
                    other => return Err(format!("unexpected argument {other:?}")),
                }
                i += 1;
            }
            Ok(Cmd::Eval {
                cells,
                at,
                formula: formula.ok_or("eval needs a formula")?,
            })
        }
        "recalc" => Ok(Cmd::Recalc {
            path: args.get(1).ok_or("recalc needs a file")?.clone(),
        }),
        "functions" => Ok(Cmd::Functions),
        "help" | "-h" | "--help" => Ok(Cmd::Help),
        other => Err(format!("unknown command {other:?}")),
    }
}

/// Renders a value for the terminal.
fn render(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => general_string(*n, None, &EN_US),
        Value::Text(t) => t.to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Value::Error(e) => e.display().into_owned(),
        Value::Array(m) => m.get(0, 0).map(render).unwrap_or_default(),
    }
}

fn addr_of(wb: &mut Workbook, key: &str) -> Result<CellAddr, String> {
    let (sheet, a1) = match key.rsplit_once(['.', '!']) {
        Some((s, a)) => {
            let s = s.trim_matches('\'');
            let id = match wb.sheet_by_name_ci(s) {
                Some(id) => id,
                None => wb.add_sheet(Some(s), None),
            };
            (id, a)
        }
        None => (wb.sheets.first().map_or(1, |s| s.id), key),
    };
    CellAddr::from_a1(sheet, a1).ok_or_else(|| format!("bad cell {key:?}"))
}

/// Evaluates `formula` after entering `cells`; returns the rendered result.
fn eval(cells: &[(String, String)], at: Option<&str>, formula: &str) -> Result<String, String> {
    let mut wb = Workbook::new();
    for (k, v) in cells {
        let a = addr_of(&mut wb, k)?;
        wb.set_input(a, v);
    }
    let origin = match at {
        Some(a) => addr_of(&mut wb, a)?,
        None => {
            let s = wb.sheets.first().map_or(1, |s| s.id);
            let col = wb
                .sheet(s)
                .and_then(|sh| sh.used_range())
                .map_or(0, |u| u.c1 + 1);
            CellAddr::new(s, 0, col.min(tally_types::MAX_COLS - 1))
        }
    };
    let text = if formula.starts_with('=') {
        formula.to_owned()
    } else {
        format!("={formula}")
    };
    wb.set_input(origin, &text);
    recalc(&mut wb);
    Ok(render(&wb.value(origin)))
}

fn recalc_file(path: &str) -> Result<String, String> {
    let mut svc = EngineService::with_default_codec();
    let t0 = Instant::now();
    let info = svc.open(Path::new(path)).map_err(|e| e.to_string())?;
    let elapsed = t0.elapsed();
    let doc = svc.document(info.doc).map_err(|e| e.to_string())?;
    Ok(format!(
        "{}: {} sheet(s), {} cells, {} formulas; loaded and recalculated in {:.1} ms",
        info.title,
        info.sheets.len(),
        doc.wb.cell_count(),
        doc.wb.formulas.len(),
        elapsed.as_secs_f64() * 1000.0
    ))
}

fn functions() -> String {
    let svc = EngineService::new();
    let mut out = String::new();
    for f in svc.list_functions() {
        out.push_str(&format!("{}\t{}\t{}\n", f.name, f.category, f.signature));
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("tally-cli: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = match cmd {
        Cmd::Help => Ok(USAGE.to_owned()),
        Cmd::Eval { cells, at, formula } => eval(&cells, at.as_deref(), &formula),
        Cmd::Recalc { path } => recalc_file(&path),
        Cmd::Functions => Ok(functions().trim_end().to_owned()),
    };
    match result {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tally-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn gate_g1() {
        let cells = parse_cells("A1=2,B1=3").unwrap();
        assert_eq!(eval(&cells, None, "=SUM(A1:B1)*2").unwrap(), "10");
    }

    #[test]
    fn cells_spec_with_commas_in_formulas() {
        let cells = parse_cells("A1=2,B1==SUM(A1,3),C1=x").unwrap();
        assert_eq!(
            cells,
            vec![
                ("A1".to_owned(), "2".to_owned()),
                ("B1".to_owned(), "=SUM(A1,3)".to_owned()),
                ("C1".to_owned(), "x".to_owned())
            ]
        );
        assert_eq!(eval(&cells, None, "=B1&C1").unwrap(), "5x");
        assert!(parse_cells("oops").is_err());
    }

    #[test]
    fn eval_values_and_errors() {
        assert_eq!(eval(&[], None, "=1/0").unwrap(), "#DIV/0!");
        assert_eq!(eval(&[], None, "1+1").unwrap(), "2");
        assert_eq!(eval(&[], None, "=\"a\"&\"b\"").unwrap(), "ab");
        assert_eq!(eval(&[], None, "=SUM(1;2").unwrap(), "Err:508");
        let cells = parse_cells("Sheet2.A1=4").unwrap();
        assert_eq!(eval(&cells, None, "=Sheet2.A1*2").unwrap(), "8");
        let cells = parse_cells("A1=1,A2=2,A3=3").unwrap();
        assert_eq!(eval(&cells, Some("C2"), "=A1:A3").unwrap(), "2");
    }

    #[test]
    fn args() {
        assert_eq!(parse_args(&s(&[])), Ok(Cmd::Help));
        assert_eq!(parse_args(&s(&["functions"])), Ok(Cmd::Functions));
        assert!(matches!(
            parse_args(&s(&["eval", "--cells", "A1=1", "=A1"])),
            Ok(Cmd::Eval { .. })
        ));
        assert!(parse_args(&s(&["eval"])).is_err());
        assert!(parse_args(&s(&["recalc"])).is_err());
        assert!(parse_args(&s(&["nope"])).is_err());
        assert!(functions().contains("SUM\t"));
    }
}
