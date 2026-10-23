//! Large but legitimate packages: `content.xml` is streamed, and memory is
//! bounded by the per-event, cell, text and attribute budgets rather than by
//! its byte cap (`Limits::max_content_xml_bytes`). A 50 MB ODS has
//! a `content.xml` of about 600 MB (XML compresses about 12:1), which the old
//! 512 MiB cap refused (docs/perf-baseline.md). Zip bombs are still stopped
//! by the ratio check (`malicious::zip_bomb_is_rejected_within_one_second`)
//! and entry floods by the EOCD precheck (`malicious::too_many_entries`).

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::io::{BufReader, BufWriter, Cursor, Write};
use std::time::{Duration, Instant};

use tally_io_ods::{read_ods, Limits, OdsError};
use tally_model::WorkbookData;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="S">"#;
const TAIL: &str = "</table:table></office:spreadsheet></office:body></office:document-content>";

const MIB: u64 = 1024 * 1024;

/// Writes an ODS package whose `content.xml` is `HEAD`, `rows` and `TAIL`.
fn write_ods<W: Write + std::io::Seek>(out: W, rows: impl FnOnce(&mut dyn Write) -> u64) -> u64 {
    let mut zip = ZipWriter::new(out);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", deflated).unwrap();
    zip.write_all(HEAD.as_bytes()).unwrap();
    let body = rows(&mut zip);
    zip.write_all(TAIL.as_bytes()).unwrap();
    zip.finish().unwrap();
    HEAD.len() as u64 + body + TAIL.len() as u64
}

/// One row of `cols` float cells.
fn row(cols: u32, seed: &mut u64) -> String {
    let mut s = String::from("<table:table-row>");
    for _ in 0..cols {
        // A small LCG: varied numbers compress like real data (~10:1 overall).
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let v = (*seed >> 40) as f64 / 100.0;
        s.push_str(&format!(
            r#"<table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell>"#
        ));
    }
    s.push_str("</table:table-row>");
    s
}

fn small_ods(rows: u32) -> (Vec<u8>, u64) {
    let mut cur = Cursor::new(Vec::new());
    let mut seed = 1;
    let len = write_ods(&mut cur, |w| {
        let mut n = 0;
        for _ in 0..rows {
            let r = row(4, &mut seed);
            w.write_all(r.as_bytes()).unwrap();
            n += r.len() as u64;
        }
        n
    });
    (cur.into_inner(), len)
}

fn limit_what(r: &Result<WorkbookData, OdsError>) -> &'static str {
    match r {
        Err(OdsError::LimitExceeded { what, .. }) => what,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

/// The defaults admit a ~100 MB ODS (content.xml ≈ 1.2 GB) with room to
/// spare, and the total budget covers content.xml plus every other part.
#[test]
fn defaults_admit_a_100_mb_ods() {
    let l = Limits::default();
    assert!(l.max_content_xml_bytes > 512 * MIB, "the old cap");
    assert!(l.max_content_xml_bytes >= 1_200_000_000);
    assert!(l.max_total_bytes >= l.max_content_xml_bytes + l.max_entry_bytes * 4);
    // Ratio and memory budgets are unchanged.
    assert_eq!(l.max_ratio, 200);
    assert_eq!(l.max_cells, 10_000_000);
}

/// The content.xml cap is exact and separate from `max_entry_bytes`: an
/// entry of exactly the cap opens, one byte more is refused, and the cap for
/// other parts does not apply to content.xml.
#[test]
fn content_xml_cap_is_exact_and_separate() {
    let (bytes, len) = small_ods(2_000);
    assert!(len > 256 * 1024, "{len}");
    let at = |cap: u64| Limits {
        max_content_xml_bytes: cap,
        max_entry_bytes: 64 * 1024,
        ..Limits::default()
    };
    let wb = read_ods(Cursor::new(&bytes), &at(len)).unwrap();
    assert_eq!(wb.sheets[0].cells.len(), 8_000);
    let r = read_ods(Cursor::new(&bytes), &at(len - 1));
    assert_eq!(limit_what(&r), "zip entry uncompressed bytes");
}

/// The limit error reads as a sentence with a human-sized limit.
#[test]
fn limit_message_is_readable() {
    let (bytes, _) = small_ods(10);
    let limits = Limits {
        max_content_xml_bytes: 2 * 1024,
        ..Limits::default()
    };
    let msg = read_ods(Cursor::new(&bytes), &limits)
        .unwrap_err()
        .to_string();
    assert_eq!(
        msg,
        "limit exceeded: the file is too large or complex to open safely \
         (zip entry uncompressed bytes over 2 KiB)"
    );
    let e = OdsError::LimitExceeded {
        what: "materialized cells",
        limit: 10_000_000,
    };
    assert!(e
        .to_string()
        .ends_with("(materialized cells over 10000000)"));
}

/// A ≥ 50 MB ODS whose content.xml is over the old 512 MiB cap opens with
/// the default limits. Slow in a debug build (it writes and parses ~600 MB of
/// XML), so run it with
/// `cargo test --release -p tally-io-ods --test large_entries -- --ignored`.
#[test]
#[ignore = "writes a ~54 MB file and parses ~700 MB of XML; run in release"]
fn fifty_mb_ods_with_700_mb_content_xml_opens() {
    let path = std::env::temp_dir().join(format!("tally-large-{}.ods", std::process::id()));
    let cols = 12;
    let rows = 520_000;
    let t = Instant::now();
    let len = write_ods(BufWriter::new(std::fs::File::create(&path).unwrap()), |w| {
        let mut seed = 7;
        let mut n = 0;
        for _ in 0..rows {
            let r = row(cols, &mut seed);
            w.write_all(r.as_bytes()).unwrap();
            n += r.len() as u64;
        }
        n
    });
    let size = std::fs::metadata(&path).unwrap().len();
    eprintln!(
        "wrote {size} bytes (content.xml {len} bytes, ratio {:.1}) in {:?}",
        len as f64 / size as f64,
        t.elapsed()
    );
    let t = Instant::now();
    let r = read_ods(
        BufReader::new(std::fs::File::open(&path).unwrap()),
        &Limits::default(),
    );
    let took = t.elapsed();
    std::fs::remove_file(&path).unwrap();
    let wb = r.unwrap();
    eprintln!("read in {took:?}");
    assert!(size >= 50_000_000, "file is {size} bytes");
    assert!(len > 512 * MIB, "content.xml is {len} bytes");
    assert_eq!(wb.sheets[0].cells.len(), (rows * cols) as usize);
    assert!(took < Duration::from_secs(60), "took {took:?}");
}

/// With content.xml allowed to reach 2 GiB, one element could otherwise make
/// quick-xml buffer gigabytes (the old `bigattr.ods` peaked at ~800 MB RSS
/// with a 400 MiB attribute). The per-event cap refuses it after 64 MiB.
/// The entry is stored (1:1), so the ratio check does not fire first.
#[test]
fn huge_attribute_is_refused_by_the_event_cap() {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(tally_io_ods::MIMETYPE.as_bytes()).unwrap();
    zip.start_file("content.xml", stored).unwrap();
    zip.write_all(HEAD.as_bytes()).unwrap();
    zip.write_all(
        br#"<table:table-row><table:table-cell office:value-type="string" office:string-value=""#,
    )
    .unwrap();
    let chunk = "abcdefgh".repeat(128 * 1024); // 1 MiB
    for _ in 0..80 {
        zip.write_all(chunk.as_bytes()).unwrap();
    }
    zip.write_all(br#""/></table:table-row>"#).unwrap();
    zip.write_all(TAIL.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let t = Instant::now();
    let r = read_ods(Cursor::new(&bytes), &Limits::default());
    assert_eq!(limit_what(&r), "XML event bytes");
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "took {:?}",
        t.elapsed()
    );
}
