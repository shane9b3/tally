//! Fuzz harnesses for Tally's untrusted-input parsers (docs/security.md §9).
//!
//! Every function here takes the fuzzer's bytes and runs them through the
//! same entry point the app uses when it opens a file, with the same limits
//! (`IoCodec::read`, i.e. `detect` and then the reader with its
//! `Limits::default()`; `open` goes through `EngineService::open`, the full
//! open including the engine load and the on-open recalc budget). An `Err`
//! is a normal outcome; a panic, abort, out-of-memory or hang is a finding.
//!
//! The functions are shared by the cargo-fuzz targets in `fuzz/` (nightly,
//! libFuzzer) and by this crate's tests (stable): `tests/replay.rs` replays
//! every file in `fuzz/seeds/<target>/` and `fuzz/regressions/<target>/`,
//! so a minimised crash dropped into `fuzz/regressions/<target>/` is
//! guarded by `cargo test` from then on.

use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tally_api::DocumentService;
use tally_engine::workbook::OdfResolver;
use tally_engine::{EngineService, FileCodec, IoCodec, Workbook};
use tally_formula::{Dialect, Resolver};
use tally_types::{CellAddr, DateEpoch, ErrorKind, Value, MAX_COLS, MAX_ROWS};

/// A fuzz target: its name (the cargo-fuzz target and the directory name
/// under `fuzz/seeds/` and `fuzz/regressions/`) and its harness.
#[derive(Clone, Copy, Debug)]
pub struct Target {
    /// Target name.
    pub name: &'static str,
    /// The harness.
    pub run: fn(&[u8]),
}

/// Every target, in the order of `.github/workflows/fuzz.yml`.
pub const TARGETS: &[Target] = &[
    t("ods", ods),
    t("ods_parts", ods_parts),
    t("xlsx", xlsx),
    t("xlsx_parts", xlsx_parts),
    t("xls", xls),
    t("fods", fods),
    t("xmlss", xmlss),
    t("csv", csv),
    t("html", html),
    t("dbf", dbf),
    t("dif", dif),
    t("sylk", sylk),
    t("formula", formula),
    t("numfmt", numfmt),
    t("open", open),
];

const fn t(name: &'static str, run: fn(&[u8])) -> Target {
    Target { name, run }
}

/// The target called `name`.
pub fn target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

/// For the libFuzzer targets that reach the XLS reader (`xls`, `open`): do
/// not treat a panic inside calamine as a finding.
///
/// `tally_io_xls::read_xls` runs calamine under `catch_unwind` on purpose
/// and reports a panic there as "could not be decoded" (docs/security.md),
/// so in the app such a file is an error message, not a crash. libfuzzer-sys
/// installs a panic hook that aborts on every panic, caught or not, which
/// would stop the fuzzer at the first of them. This wraps that hook: a
/// panic whose location is in calamine's sources returns to the reader's
/// `catch_unwind`; any other panic (our own code, also inside that guard)
/// still aborts. A calamine panic that is not caught by the reader unwinds
/// into libfuzzer-sys, which aborts when the test function panics.
///
/// Call it at the start of the fuzz target; the first call installs the
/// hook (libFuzzer's is in place by then).
pub fn tolerate_guarded_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let guarded = info.location().is_some_and(|l| {
                let file = l.file().replace('\\', "/");
                file.contains("/calamine-") || file.contains("/calamine/")
            });
            if !guarded {
                prev(info);
            }
        }));
    });
}

/// What the app does with the bytes of a file with extension `ext`
/// (`tally_engine::service::codec::IoCodec::read`: content sniffing, then
/// the reader with its default limits).
fn codec_read(bytes: &[u8], ext: &str) {
    let _ = IoCodec.read(bytes, ext);
}

/// An `.ods` file as raw bytes.
pub fn ods(data: &[u8]) {
    codec_read(data, "ods");
}

/// An `.xlsx` file as raw bytes.
pub fn xlsx(data: &[u8]) {
    codec_read(data, "xlsx");
}

/// An `.xls` file (OLE2 / BIFF) as raw bytes.
pub fn xls(data: &[u8]) {
    codec_read(data, "xls");
}

/// A flat ODF spreadsheet (`.fods`).
pub fn fods(data: &[u8]) {
    codec_read(data, "fods");
}

/// An Excel 2003 XML spreadsheet (`.xml`).
pub fn xmlss(data: &[u8]) {
    codec_read(data, "xml");
}

/// A `.csv` file (delimiter and encoding detection, then the CSV reader).
pub fn csv(data: &[u8]) {
    codec_read(data, "csv");
}

/// An `.html` file (tables).
pub fn html(data: &[u8]) {
    codec_read(data, "html");
}

/// A dBase `.dbf` file.
pub fn dbf(data: &[u8]) {
    codec_read(data, "dbf");
}

/// A `.dif` file.
pub fn dif(data: &[u8]) {
    codec_read(data, "dif");
}

/// A SYLK `.slk` file.
pub fn sylk(data: &[u8]) {
    codec_read(data, "slk");
}

/// The media type `ods_parts` stores as `mimetype` unless the input has
/// its own `mimetype` part (e.g. the OOo 1.x one of an `.sxc`).
const ODS_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet";

/// An `.ods` package built from fuzzer-chosen parts ([`build_package`]),
/// so mutations reach the XML parsers instead of dying in the zip layer.
pub fn ods_parts(data: &[u8]) {
    if let Some(zip) = build_package(data, Some(ODS_MIMETYPE)) {
        codec_read(&zip, "ods");
    }
}

/// An `.xlsx` package built from fuzzer-chosen parts ([`build_package`]).
pub fn xlsx_parts(data: &[u8]) {
    if let Some(zip) = build_package(data, None) {
        codec_read(&zip, "xlsx");
    }
}

/// Most entries [`build_package`] writes.
const MAX_PARTS: usize = 64;

/// Builds a zip package from `data` = `name \0 content \0 name \0 content …`
/// (NUL never occurs in XML, so the separator is unambiguous; a missing last
/// content is empty). Entries are stored uncompressed, in input order;
/// names that are not UTF-8, empty or repeated are skipped. With
/// `mimetype`, a stored `mimetype` entry is written first unless the input
/// has one. `None` if the zip writer refuses the input.
pub fn build_package(data: &[u8], mimetype: Option<&str>) -> Option<Vec<u8>> {
    let mut chunks = data.split(|&b| b == 0);
    let mut entries: Vec<(&str, &[u8])> = Vec::new();
    let mut seen = HashSet::new();
    while let Some(name) = chunks.next() {
        let content = chunks.next().unwrap_or(&[]);
        let Ok(name) = std::str::from_utf8(name) else {
            continue;
        };
        if name.is_empty() || !seen.insert(name) {
            continue;
        }
        entries.push((name, content));
        if entries.len() == MAX_PARTS {
            break;
        }
    }
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    if let Some(m) = mimetype {
        if !seen.contains("mimetype") {
            w.start_file("mimetype", stored).ok()?;
            w.write_all(m.as_bytes()).ok()?;
        }
    }
    for (name, content) in entries {
        w.start_file(name, stored).ok()?;
        w.write_all(content).ok()?;
    }
    w.finish().ok().map(Cursor::into_inner)
}

/// The inverse of [`build_package`] for a trusted zip (seed generation):
/// every file entry whose content has no NUL byte, in archive order.
pub fn package_to_parts(zip_bytes: &[u8]) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes)).ok()?;
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).ok()?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().ok()?.into_owned();
        let mut content = Vec::new();
        f.read_to_end(&mut content).ok()?;
        if content.contains(&0) || name.as_bytes().contains(&0) {
            continue;
        }
        if !out.is_empty() {
            out.push(0);
        }
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(&content);
    }
    Some(out)
}

/// Formula text: `data[0]` picks the dialect and the origin cell, the rest
/// is the formula (UTF-8; cell text always is).
///
/// - `% 6 == 0`: OpenFormula as stored in ODS, with the resolver the
///   engine uses for file formulas (`OdfResolver`);
/// - `1`: what the user types (Calc A1, en-US; `Workbook::parse_formula`);
/// - `2`: Excel A1 with the engine's resolver;
/// - `3`: an `.xls`/`.xlsx` stored formula through
///   `tally_io_xlsx::excel_formula_to_of`, then the OpenFormula parse;
/// - `4`: an `.xmlss` R1C1 formula through `r1c1_formula_to_of`, then
///   the OpenFormula parse;
/// - `5`: an AOO native formula (SYLK) through `native_formula_to_of`,
///   then the OpenFormula parse.
///
/// Each successful parse is printed back in its dialect and in Calc A1,
/// and the printed text is parsed again.
pub fn formula(data: &[u8]) {
    let Some((&sel, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let mut wb = Workbook::empty();
    let s1 = wb.add_sheet(Some("Sheet1"), None);
    wb.add_sheet(Some("Data 2"), None);
    let (row, col) = match (sel >> 4) % 3 {
        0 => (0, 0),
        1 => (9, 3),
        _ => (MAX_ROWS - 1, MAX_COLS - 1),
    };
    let origin = CellAddr {
        sheet: s1,
        row,
        col,
    };
    let sheet_names = ["Sheet1".to_owned(), "Data 2".to_owned()];
    let odf = OdfResolver(&wb);
    match sel % 6 {
        0 => parse_print(text, origin, Dialect::open_formula(), &odf),
        1 => parse_print(text, origin, Dialect::calc_a1_en_us(), &wb),
        2 => parse_print(text, origin, Dialect::excel_a1(), &wb),
        3 => {
            if let Some(of) = tally_io_xlsx::excel_formula_to_of(text, &sheet_names, 0, row, col) {
                parse_print(&of, origin, Dialect::open_formula(), &odf);
            }
        }
        4 => {
            if let Some(of) = tally_io_xlsx::r1c1_formula_to_of(text, &sheet_names, 0, row, col) {
                parse_print(&of, origin, Dialect::open_formula(), &odf);
            }
        }
        _ => {
            let of = tally_io_ods::native_formula_to_of(text);
            parse_print(&of, origin, Dialect::open_formula(), &odf);
        }
    }
}

fn parse_print(text: &str, origin: CellAddr, dialect: Dialect, resolver: &dyn Resolver) {
    let Ok(f) = tally_formula::parse(text, origin, dialect, resolver) else {
        return;
    };
    for d in [dialect, Dialect::calc_a1_en_us()] {
        let printed = tally_formula::print(&f, origin, d, resolver);
        let _ = tally_formula::parse(&printed, origin, d, resolver);
    }
}

/// Values every parsed number format is applied to.
fn sample_values() -> [Value; 9] {
    [
        Value::Number(0.0),
        Value::Number(-1234.5678),
        Value::Number(45_000.75),
        Value::Number(2_958_465.999_99),
        Value::Number(-1.0),
        Value::Number(1e300),
        Value::text("abc"),
        Value::Bool(true),
        Value::Error(ErrorKind::Div0),
    ]
}

/// Number formats: `data[0]` even = the rest is a format code (as stored in
/// a file or typed in Format Cells), parsed as en-US and as de-DE and
/// applied to sample values; odd = the rest is cell input for the input
/// recognizer (numbers, dates, times, percentages) in four locales.
pub fn numfmt(data: &[u8]) {
    let Some((&sel, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let locales = [
        &tally_numfmt::EN_US,
        &tally_numfmt::DE_DE,
        &tally_numfmt::FR_FR,
        &tally_numfmt::JA_JP,
    ];
    if sel & 1 == 1 {
        let ctx = tally_numfmt::RecognizeContext {
            epoch: DateEpoch::D1899,
            today: (2026, 1, 15),
            year2000: 1930,
        };
        for locale in locales {
            let _ = tally_numfmt::recognize_input_with(text, locale, &ctx);
        }
        return;
    }
    // Readers refuse stored codes over this length (and TEXT() too).
    if text.chars().count() > tally_numfmt::MAX_CODE_LEN {
        return;
    }
    let asts = [
        tally_numfmt::parse_format(text),
        tally_numfmt::parse_format_with_locale(text, &tally_numfmt::DE_DE),
    ];
    for ast in asts.iter().flatten() {
        let _ = tally_numfmt::print_format(ast);
        for v in &sample_values() {
            for (locale, width, epoch) in [
                (locales[0], None, DateEpoch::D1899),
                (locales[1], Some(10), DateEpoch::D1900),
            ] {
                let _ = tally_numfmt::format_value_std(v, ast, locale, width, epoch, None);
            }
        }
    }
}

/// File extensions the `open` target picks from with `data[0]`.
pub const OPEN_EXTS: &[&str] = &[
    "ods", "xlsx", "xls", "fods", "sxc", "xml", "csv", "tsv", "txt", "html", "dbf", "dif", "slk",
    "",
];

static OPEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The whole open, as File ▸ Open does it: `data[0]` picks the extension
/// ([`OPEN_EXTS`]), the rest is written to a temporary file that
/// `EngineService::open` reads (file-size cap, codec, sheet cap, engine
/// load with text recognition, formula parsing and the on-open recalc
/// budget).
pub fn open(data: &[u8]) {
    let Some((&sel, rest)) = data.split_first() else {
        return;
    };
    let ext = OPEN_EXTS[usize::from(sel) % OPEN_EXTS.len()];
    let n = OPEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("tally-fuzz-open-{}-{n}", std::process::id()));
    if !ext.is_empty() {
        path.set_extension(ext);
    }
    if std::fs::write(&path, rest).is_err() {
        return;
    }
    let mut svc = EngineService::with_default_codec();
    let _ = svc.open(&path);
    let _ = std::fs::remove_file(&path);
}
