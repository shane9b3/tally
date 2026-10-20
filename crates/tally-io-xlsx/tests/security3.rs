//! Regression tests for the third security review
//! (`docs/reviews/security-review-3-2026-10-20.md`), XLSX reader side:
//! HIGH-3 (hyperlink expansion), MEDIUM-5 (`spinCount`, hash material),
//! MEDIUM-6 (relationship lookup), MEDIUM-8 (comment text) and the
//! conditional-format caps. Inputs are generated here.

#![allow(clippy::unwrap_used)] // tests may unwrap (docs/contributing.md)

use std::fmt::Write as _;
use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tally_io_xlsx::{read_xlsx, Limits};
use tally_model::WorkbookData;

const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const RELS_NS: &str = r#"xmlns="http://schemas.openxmlformats.org/package/2006/relationships""#;

/// A one-sheet package with the sheet's own relationships and an optional
/// comments part.
fn package(sheet_xml: &str, sheet_rels: Option<&str>, comments: Option<&str>) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut entries = vec![
        (
            "xl/workbook.xml".to_string(),
            format!(
                r#"<workbook {NS}><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".to_string(),
            format!(
                r#"<Relationships {RELS_NS}><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml".to_string(),
            sheet_xml.to_string(),
        ),
    ];
    if let Some(r) = sheet_rels {
        entries.push((
            "xl/worksheets/_rels/sheet1.xml.rels".to_string(),
            format!("<Relationships {RELS_NS}>{r}</Relationships>"),
        ));
    }
    if let Some(c) = comments {
        entries.push(("xl/comments1.xml".to_string(), c.to_string()));
    }
    for (name, data) in entries {
        z.start_file(name, opts).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn ws(rows: &str, post: &str) -> String {
    format!("<worksheet {NS}><sheetData>{rows}</sheetData>{post}</worksheet>")
}

fn hyperlink_rel(id: &str, target: &str) -> String {
    format!(
        r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{target}" TargetMode="External"/>"#
    )
}

fn read(bytes: Vec<u8>) -> (WorkbookData, Duration) {
    let t = Instant::now();
    let r = read_xlsx(Cursor::new(bytes), &Limits::default()).unwrap();
    (r, t.elapsed())
}

/// HIGH-3 (`bigtarget`): one 1 MB external target on A1:J1000 took 2.9 s
/// and 2.5 GB (8 MB: killed at 6.5 GB). Targets over 8 KiB are dropped
/// when the relationships are parsed, with a warning.
#[test]
fn over_long_hyperlink_targets_are_dropped_at_parse_time() {
    let target = format!("https://example.com/{}", "x".repeat(1 << 20));
    let bytes = package(
        &ws(
            "",
            r#"<hyperlinks><hyperlink ref="A1:J1000" r:id="rId9"/></hyperlinks>"#,
        ),
        Some(&hyperlink_rel("rId9", &target)),
        None,
    );
    let (wb, took) = read(bytes);
    assert!(took < Duration::from_secs(5), "{took:?}");
    assert!(wb.sheets[0].links.is_empty());
    assert!(
        wb.warnings.iter().any(|w| w.contains("8192 bytes")),
        "{:?}",
        wb.warnings
    );
    // An 8 KiB target is kept and shared by every cell of the ref.
    let ok = format!("https://example.com/{}", "y".repeat(8000));
    let bytes = package(
        &ws(
            "",
            r#"<hyperlinks><hyperlink ref="A1:J100" r:id="rId9"/></hyperlinks>"#,
        ),
        Some(&hyperlink_rel("rId9", &ok)),
        None,
    );
    let (wb, _) = read(bytes);
    let links = &wb.sheets[0].links;
    assert_eq!(links.len(), 1000);
    assert!(links.iter().all(|l| Arc::ptr_eq(&l.url, &links[0].url)));
    assert_eq!(&*links[0].url, ok);
}

/// HIGH-3 (`links`): 100,000 refs × 10,000 cells (location only) ran for
/// more than a minute and 1.8 GB. Expansion stops at 100,000 link cells
/// per sheet before anything is cloned; the cells count against the
/// cell budget.
#[test]
fn hyperlink_expansion_is_capped_per_sheet() {
    let mut links = String::new();
    for i in 0..1000u32 {
        let _ = write!(
            links,
            r#"<hyperlink ref="A{}:J{}" location="S!A1"/>"#,
            i * 1000 + 1,
            i * 1000 + 1000
        );
    }
    let bytes = package(
        &ws("", &format!("<hyperlinks>{links}</hyperlinks>")),
        None,
        None,
    );
    let (wb, took) = read(bytes);
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(wb.sheets[0].links.len(), 100_000);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.contains("first 100000 hyperlinks")),
        "{:?}",
        wb.warnings
    );
    assert!(wb.sheets[0].links.iter().all(|l| &*l.url == "#S.A1"));
    // A small cell budget stops the expansion instead of failing the file.
    let limits = Limits {
        max_cells: 25_000,
        ..Limits::default()
    };
    let bytes = package(
        &ws(
            "",
            r#"<hyperlinks><hyperlink ref="A1:J1000" location="S!A1"/><hyperlink ref="A1001:J2000" location="S!A1"/><hyperlink ref="A2001:J3000" location="S!A1"/></hyperlinks>"#,
        ),
        None,
        None,
    );
    let wb = read_xlsx(Cursor::new(bytes), &limits).unwrap();
    assert_eq!(wb.sheets[0].links.len(), 20_000);
}

/// MEDIUM-6 (`rels`): 100,000 links each with its own relationship took
/// 8.8 s (a linear scan per link). The lookup is a hash map.
#[test]
fn relationship_lookup_is_a_hash_map() {
    let n = 30_000u32;
    let mut links = String::new();
    let mut rels = String::new();
    for i in 0..n {
        let _ = write!(links, r#"<hyperlink ref="A{}" r:id="rId{i}"/>"#, i + 1);
        rels.push_str(&hyperlink_rel(
            &format!("rId{i}"),
            &format!("https://example.com/{i}"),
        ));
    }
    let bytes = package(
        &ws("", &format!("<hyperlinks>{links}</hyperlinks>")),
        Some(&rels),
        None,
    );
    let (wb, took) = read(bytes);
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(wb.sheets[0].links.len(), n as usize);
    assert_eq!(&*wb.sheets[0].links[7].url, "https://example.com/7");
}

/// MEDIUM-5 (`spin`, `salt`): `spinCount="4294967295"` cost 3 s per
/// unlock attempt; a 16 MB salt was cloned with every meta op. The spin
/// count is clamped to 100,000 and over-long hash material is dropped
/// (the sheet stays protected; any password unlocks it).
#[test]
fn spin_count_is_clamped_and_long_hash_material_dropped() {
    let prot = |extra: &str| {
        ws(
            "",
            &format!(
                r#"<sheetProtection sheet="1" algorithmName="SHA-512" hashValue="AAAA" {extra}/>"#
            ),
        )
    };
    let (wb, _) = read(package(
        &prot(r#"saltValue="c2FsdA==" spinCount="4294967295""#),
        None,
        None,
    ));
    let p = wb.sheets[0].protection.as_ref().unwrap();
    assert_eq!(p.spin_count, Some(100_000));
    assert_eq!(p.password_hash.as_deref(), Some("AAAA"));
    // Pseudo-random base64 (a repeated letter would trip the zip-bomb
    // ratio guard first), 2 MB like the review's random salt.
    let mut x = 0x9E37_79B9u32;
    let big: String = (0..(2 << 20))
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[(x % 64) as usize]
                as char
        })
        .collect();
    let (wb, took) = read(package(
        &prot(&format!(r#"saltValue="{big}" spinCount="100""#)),
        None,
        None,
    ));
    assert!(took < Duration::from_secs(10), "{took:?}");
    let p = wb.sheets[0].protection.as_ref().unwrap();
    assert!(p.password_hash.is_none() && p.salt.is_none() && p.spin_count.is_none());
}

/// MEDIUM-8: a 60 KiB comment is truncated to the engine's 32 KiB note
/// cap with a warning.
#[test]
fn long_comments_are_truncated() {
    let comments = format!(
        r#"<comments {NS}><authors><author>bob</author></authors><commentList><comment ref="A1" authorId="0"><text><t>{}</t></text></comment><comment ref="A2" authorId="0"><text><t>short</t></text></comment></commentList></comments>"#,
        "é".repeat(30_000)
    );
    let rel = r#"<Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/>"#;
    let (wb, _) = read(package(&ws("", ""), Some(rel), Some(&comments)));
    let notes = &wb.sheets[0].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].text.len(), 32 * 1024);
    assert!(notes[0].text.chars().all(|c| c == 'é'));
    assert_eq!(notes[1].text, "short");
    assert!(
        wb.warnings.iter().any(|w| w.contains("32 KiB")),
        "{:?}",
        wb.warnings
    );
}

/// CRITICAL-1 (`cfsame`): 10,000 `cfRule`s on one range. Only the first
/// 2,048 formats of a sheet are kept (warning) and each keeps at most 64
/// ranges.
#[test]
fn conditional_formats_are_capped() {
    let mut cfs = String::new();
    for i in 0..3000 {
        let _ = write!(
            cfs,
            r#"<conditionalFormatting sqref="A1:T1000"><cfRule type="expression" priority="{}"><formula>A1&gt;0</formula></cfRule></conditionalFormatting>"#,
            i + 1
        );
    }
    let sqref: String = (0..100).map(|i| format!("A{} ", i + 1)).collect();
    let _ = write!(
        cfs,
        r#"<conditionalFormatting sqref="{sqref}"><cfRule type="expression" priority="5000"><formula>A1&gt;0</formula></cfRule></conditionalFormatting>"#
    );
    let (wb, took) = read(package(&ws("", &cfs), None, None));
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(wb.sheets[0].conditional_formats.len(), 2048);
    assert!(
        wb.warnings
            .iter()
            .any(|w| w.contains("2048 conditional formats")),
        "{:?}",
        wb.warnings
    );
    let (wb, _) = read(package(
        &ws(
            "",
            &format!(
                r#"<conditionalFormatting sqref="{sqref}"><cfRule type="expression" priority="1"><formula>A1&gt;0</formula></cfRule></conditionalFormatting>"#
            ),
        ),
        None,
        None,
    ));
    assert_eq!(wb.sheets[0].conditional_formats[0].ranges.len(), 64);
}
