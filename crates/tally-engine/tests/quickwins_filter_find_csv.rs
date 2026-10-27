//! AutoFilter "Top 10" (and top/bottom N and N %), Find & Replace in notes
//! and in the current selection, and the CSV Text Export options.
#![allow(clippy::unwrap_used)]

mod common;

use common::{range, Fixture};
use tally_api::{
    ColumnFilter, DocumentService, FilterCondition, FilterOp, FindMode, FindQuery, NoteDto, Op,
};

fn hidden_rows(f: &Fixture) -> Vec<u32> {
    let l = f.svc.sheet_layout(f.doc, f.sheet).unwrap();
    l.row_runs
        .iter()
        .filter(|r| r.hidden)
        .flat_map(|r| r.start..=r.end)
        .collect()
}

fn top(f: &mut Fixture, op: FilterOp, n: &str) {
    let res = f.apply(vec![Op::SetColumnFilter {
        sheet: f.sheet,
        col: 0,
        filter: Some(ColumnFilter {
            values: None,
            condition: Some(FilterCondition {
                op,
                value: n.into(),
            }),
        }),
    }]);
    assert!(res.errors.is_empty(), "{:?}", res.errors);
}

/// AOO's AutoFilter "Top 10" is `SC_TOPVAL` with 10: the threshold is the
/// 10th largest number and ties at it pass (`ScTable::TopTenQuery`), so 11
/// rows stay when the 10th and 11th largest are equal. Text and empty
/// cells never pass.
#[test]
fn autofilter_top_10_keeps_ties() {
    let mut f = Fixture::new();
    f.set("A1", "Amount");
    // 1..=12, with 3 twice (rows 2..=14), and a text cell (row 15).
    let vals = [1, 2, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    for (i, v) in vals.iter().enumerate() {
        f.set(&format!("A{}", i + 2), &v.to_string());
    }
    f.set("A15", "n/a");
    f.apply(vec![Op::SetAutoFilter {
        sheet: f.sheet,
        range: Some(range("A1:A15")),
    }]);
    top(&mut f, FilterOp::TopValues, "10");
    // Kept: 3, 3, 4..=12 (11 rows); hidden: 1, 2 (rows 2-3, 0-based 1-2)
    // and the text row (0-based 14).
    assert_eq!(hidden_rows(&f), vec![1, 2, 14]);
    top(&mut f, FilterOp::BottomValues, "3");
    // 1, 2, 3, 3 (the tie at the 3rd smallest).
    assert_eq!(hidden_rows(&f), (5..=14).collect::<Vec<u32>>());
    top(&mut f, FilterOp::TopPercent, "25");
    // 13 numbers: offset 13 - 13*25/100 = 10 → the threshold is 10.
    assert_eq!(hidden_rows(&f), (1..=10).chain([14]).collect::<Vec<u32>>());
    top(&mut f, FilterOp::BottomPercent, "10");
    // offset 13*10/100 = 1 → threshold 2: 1 and 2.
    assert_eq!(hidden_rows(&f), (3..=14).collect::<Vec<u32>>());
    // The criteria round-trip through the API.
    let info = f.svc.filter_info(f.doc, f.sheet).unwrap();
    let active = info.columns[0].active.as_ref().unwrap();
    assert_eq!(
        active.condition.as_ref().map(|c| c.op),
        Some(FilterOp::BottomPercent)
    );
    // Undo goes back to the previous Top/Bottom N.
    f.svc.undo(f.doc).unwrap();
    assert_eq!(hidden_rows(&f), (1..=10).chain([14]).collect::<Vec<u32>>());
}

fn q(text: &str) -> FindQuery {
    FindQuery {
        text: text.into(),
        sheet: None,
        in_formulas: false,
        match_case: false,
        whole_cell: false,
        mode: FindMode::Literal,
        from: None,
        in_notes: None,
        selection: None,
    }
}

fn note(f: &mut Fixture, r: u32, c: u32, text: &str) {
    f.apply(vec![Op::SetNote {
        sheet: f.sheet,
        r,
        c,
        note: NoteDto {
            r,
            c,
            text: text.into(),
            author: None,
            created: None,
            shown: false,
        },
    }]);
}

fn note_text(f: &Fixture, r: u32, c: u32) -> String {
    f.svc
        .note(f.doc, f.sheet, r, c)
        .unwrap()
        .map(|n| n.text)
        .unwrap_or_default()
}

/// AOO's Search in "Notes": hits are the cells whose note matches; Replace
/// All edits the note text and one undo restores it.
#[test]
fn find_and_replace_in_notes() {
    let mut f = Fixture::new();
    f.set("A1", "apple");
    f.set("B2", "pear");
    note(&mut f, 1, 1, "Check the apple price");
    note(&mut f, 3, 0, "apple apple");
    note(&mut f, 4, 2, "nothing here");
    let hits = |f: &Fixture, query: &FindQuery| {
        f.svc
            .find(f.doc, query)
            .unwrap()
            .hits
            .iter()
            .map(|h| (h.r, h.c))
            .collect::<Vec<_>>()
    };
    assert_eq!(hits(&f, &q("apple")), vec![(0, 0)], "cells only by default");
    let in_notes = FindQuery {
        in_notes: Some(true),
        ..q("apple")
    };
    assert_eq!(hits(&f, &in_notes), vec![(1, 1), (3, 0)]);
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: Some(f.sheet),
        query: in_notes.clone(),
        replace: "plum".into(),
    }]);
    assert_eq!(res.replaced, Some(2));
    assert_eq!(note_text(&f, 1, 1), "Check the plum price");
    assert_eq!(note_text(&f, 3, 0), "plum plum");
    assert_eq!(f.text("A1"), "apple", "cells are untouched");
    f.svc.undo(f.doc).unwrap();
    assert_eq!(note_text(&f, 1, 1), "Check the apple price");
    assert_eq!(note_text(&f, 3, 0), "apple apple");
    f.svc.redo(f.doc).unwrap();
    assert_eq!(note_text(&f, 3, 0), "plum plum");
}

/// "Current selection only": find and replace stay inside the ranges.
#[test]
fn find_and_replace_in_the_selection() {
    let mut f = Fixture::new();
    for a in ["A1", "A2", "B1", "C3", "D4"] {
        f.set(a, "x");
    }
    let sel = FindQuery {
        selection: Some(vec![range("A1:A2"), range("C3:D4"), range("A2:B2")]),
        ..q("x")
    };
    let found = f.svc.find(f.doc, &sel).unwrap();
    let cells: Vec<(u32, u32)> = found.hits.iter().map(|h| (h.r, h.c)).collect();
    assert_eq!(cells, vec![(0, 0), (1, 0), (2, 2), (3, 3)]);
    // A single-cell selection is Replace (the current hit) in the dialog.
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: Some(f.sheet),
        query: FindQuery {
            selection: Some(vec![range("C3")]),
            ..q("x")
        },
        replace: "y".into(),
    }]);
    assert_eq!(res.replaced, Some(1));
    assert_eq!(f.text("C3"), "y");
    assert_eq!(f.text("D4"), "x");
    let res = f.apply(vec![Op::ReplaceAll {
        sheet: Some(f.sheet),
        query: sel,
        replace: "z".into(),
    }]);
    assert_eq!(res.replaced, Some(3));
    assert_eq!(f.text("B1"), "x", "outside the selection");
    // A bad range is refused.
    let bad = FindQuery {
        selection: Some(vec![tally_api::RangeDto {
            r0: 3,
            c0: 0,
            r1: 1,
            c1: 0,
        }]),
        ..q("x")
    };
    assert!(f.svc.find(f.doc, &bad).is_err());
}

#[cfg(feature = "io")]
mod csv {
    use super::*;
    use tally_api::{CsvCharset, CsvExportOptions, FileFormat};
    use tally_engine::EngineService;

    /// An AutoFilter area that does not reach the sheet's end is kept as
    /// written: `A1:D1000` with data to row 500 round-trips through XLSX and
    /// ODS as `A1:D1000`, and a value typed at row 800 is filtered.
    #[test]
    fn partial_autofilter_area_is_kept_as_written() {
        let dir = std::env::temp_dir().join(format!("tally-af-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc = EngineService::with_default_codec();
        let info = svc.new_workbook(None).unwrap();
        let (doc, s) = (info.doc, info.sheets[0].id);
        let mut ops = vec![Op::SetCellInput {
            sheet: s,
            r: 0,
            c: 0,
            input: "Amount".into(),
        }];
        for r in 1..500 {
            ops.push(Op::SetCellInput {
                sheet: s,
                r,
                c: 0,
                input: r.to_string(),
            });
        }
        ops.push(Op::SetAutoFilter {
            sheet: s,
            range: Some(range("A1:D1000")),
        });
        ops.push(Op::SetColumnFilter {
            sheet: s,
            col: 0,
            filter: Some(ColumnFilter {
                values: None,
                condition: Some(FilterCondition {
                    op: FilterOp::Ge,
                    value: "1000".into(),
                }),
            }),
        });
        svc.apply(doc, common::tx(ops)).unwrap();
        for (name, fmt) in [
            ("keep.xlsx", FileFormat::Xlsx),
            ("keep.ods", FileFormat::Ods),
        ] {
            let p = dir.join(name);
            svc.save(doc, Some(&p), fmt).unwrap();
            let back = svc.open(&p).unwrap();
            let bs = back.sheets[0].id;
            let fi = svc.filter_info(back.doc, bs).unwrap();
            assert_eq!(fi.range, Some(range("A1:D1000")), "{name}");
            let again = dir.join(format!("again-{name}"));
            svc.save(back.doc, Some(&again), fmt).unwrap();
            let third = svc.open(&again).unwrap();
            assert_eq!(
                svc.filter_info(third.doc, third.sheets[0].id)
                    .unwrap()
                    .range,
                Some(range("A1:D1000")),
                "{name} saved back unchanged"
            );
            // Rows 800 and 801, typed after opening, are inside the filter.
            svc.apply(
                back.doc,
                common::tx(vec![
                    Op::SetCellInput {
                        sheet: bs,
                        r: 799,
                        c: 0,
                        input: "2000".into(),
                    },
                    Op::SetCellInput {
                        sheet: bs,
                        r: 800,
                        c: 0,
                        input: "5".into(),
                    },
                    Op::RefreshFilter { sheet: bs },
                ]),
            )
            .unwrap();
            let l = svc.sheet_layout(back.doc, bs).unwrap();
            let hidden = |r: u32| {
                l.row_runs
                    .iter()
                    .any(|x| x.hidden && x.start <= r && r <= x.end)
            };
            assert!(!hidden(799), "{name}: 2000 passes >= 1000");
            assert!(hidden(800), "{name}: 5 is filtered out");
            assert!(hidden(1) && hidden(999) && !hidden(1000), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn saved(svc: &mut EngineService, doc: tally_api::DocId, name: &str) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!("tally-csv-opts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        svc.save(doc, Some(&p), FileFormat::Csv).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        let _ = std::fs::remove_file(&p);
        bytes
    }

    /// AOO's Text Export options: delimiters, "Quote all text cells",
    /// "Save cell content as shown" and the character set; the document
    /// keeps them for the next save.
    #[test]
    fn csv_export_options() {
        let mut svc = EngineService::with_default_codec();
        let info = svc.new_workbook(None).unwrap();
        let (doc, s) = (info.doc, info.sheets[0].id);
        let set = |svc: &mut EngineService, r, c, t: &str| {
            svc.apply(
                doc,
                common::tx(vec![Op::SetCellInput {
                    sheet: s,
                    r,
                    c,
                    input: t.into(),
                }]),
            )
            .unwrap();
        };
        set(&mut svc, 0, 0, "Café");
        set(&mut svc, 0, 1, "1234.5678");
        set(&mut svc, 0, 2, "a;b");
        // Two decimals with a thousands separator.
        svc.apply(
            doc,
            common::tx(vec![Op::ApplyStyle {
                sheet: s,
                ranges: vec![range("B1")],
                patch: tally_api::StylePatch {
                    number_format: Some("#,##0.00".into()),
                    ..tally_api::StylePatch::default()
                },
            }]),
        )
        .unwrap();
        // Defaults: comma, quotes where needed, as shown, UTF-8.
        assert_eq!(
            saved(&mut svc, doc, "a.csv"),
            "Café,\"1,234.57\",a;b\r\n".as_bytes()
        );
        let opts = CsvExportOptions {
            field_delimiter: ";".into(),
            text_delimiter: "'".into(),
            quote_all_text: true,
            as_shown: false,
            charset: CsvCharset::Windows1252,
        };
        svc.set_csv_options(doc, &opts).unwrap();
        assert_eq!(
            saved(&mut svc, doc, "b.csv"),
            b"'Caf\xe9';1234.5678;'a;b'\r\n".to_vec()
        );
        // No text delimiter, tab, UTF-16.
        let opts = CsvExportOptions {
            field_delimiter: "\t".into(),
            text_delimiter: String::new(),
            quote_all_text: true,
            as_shown: true,
            charset: CsvCharset::Utf16,
        };
        svc.set_csv_options(doc, &opts).unwrap();
        let bytes = saved(&mut svc, doc, "c.csv");
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&units).unwrap(),
            "Café\t1,234.57\ta;b\r\n"
        );
        // Bad delimiters are refused and the old options stay.
        for (fd, td) in [
            ("", "\""),
            (";;", "\""),
            ("\"", "\""),
            (",", "ab"),
            ("\n", ""),
        ] {
            let bad = CsvExportOptions {
                field_delimiter: fd.into(),
                text_delimiter: td.into(),
                ..CsvExportOptions::default()
            };
            assert!(svc.set_csv_options(doc, &bad).is_err(), "{fd:?} {td:?}");
        }
        assert_eq!(svc.document(doc).unwrap().csv_options, opts);
    }

    /// Saving an opened CSV keeps its delimiter and character set (AOO
    /// reuses the import's filter options): a `;` Windows-1252 file and a
    /// `.tsv` come back as they were, not as comma-separated UTF-8.
    #[test]
    fn csv_save_reuses_the_import_options() {
        let dir = std::env::temp_dir().join(format!("tally-csv-reuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut svc = EngineService::with_default_codec();
        for (name, bytes) in [
            ("semi.csv", b"Name;Ort\r\nJos\xe9;K\xf6ln\r\n".to_vec()),
            ("tabs.tsv", b"a\tb\r\nc,d\te\r\n".to_vec()),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, &bytes).unwrap();
            let info = svc.open(&p).unwrap();
            svc.save(info.doc, None, FileFormat::Csv).unwrap();
            assert_eq!(std::fs::read(&p).unwrap(), bytes, "{name}");
        }
        let again = svc.open(&dir.join("semi.csv")).unwrap().doc;
        let o = svc.document(again).unwrap().csv_options.clone();
        assert_eq!(o.field_delimiter, ";");
        assert_eq!(o.charset, CsvCharset::Windows1252);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No text delimiter with a delimiter inside a cell, and characters the
    /// character set lacks, are reported on save.
    #[test]
    fn csv_save_warns_about_lossy_options() {
        let mut svc = EngineService::with_default_codec();
        let info = svc.new_workbook(None).unwrap();
        let (doc, s) = (info.doc, info.sheets[0].id);
        svc.apply(
            doc,
            common::tx(vec![Op::SetCellInput {
                sheet: s,
                r: 0,
                c: 0,
                input: "a,b ✓".into(),
            }]),
        )
        .unwrap();
        let opts = CsvExportOptions {
            text_delimiter: String::new(),
            charset: CsvCharset::Iso88591,
            ..CsvExportOptions::default()
        };
        svc.set_csv_options(doc, &opts).unwrap();
        let p = std::env::temp_dir().join(format!("tally-csv-warn-{}.csv", std::process::id()));
        let info = svc.save(doc, Some(&p), FileFormat::Csv).unwrap();
        let _ = std::fs::remove_file(&p);
        assert!(
            info.warnings
                .iter()
                .any(|w| w.contains("no text delimiter")),
            "{:?}",
            info.warnings
        );
        assert!(
            info.warnings.iter().any(|w| w.starts_with("1 characters")),
            "{:?}",
            info.warnings
        );
    }
}

// ----- review: a file's AutoFilter over the whole sheet -----------------------------

fn wide_filter(
    op: tally_model::FilterOpData,
    value: &str,
    far_cell: bool,
) -> tally_model::WorkbookData {
    use tally_model::*;
    use tally_types::{Value, MAX_COLS, MAX_ROWS};
    let mut sheet = SheetData {
        name: "S".into(),
        ..SheetData::default()
    };
    let cell = |row, col, v: Value| CellData {
        row,
        col,
        content: CellContentData::Value(v),
        style: None,
    };
    sheet.cells.push(cell(0, 0, Value::text("H")));
    sheet.cells.push(cell(1, 0, Value::Number(1.0)));
    sheet.cells.push(cell(2, 1, Value::text("x")));
    if far_cell {
        sheet
            .cells
            .push(cell(MAX_ROWS - 1, MAX_COLS - 1, Value::Number(2.0)));
    }
    sheet.auto_filter = Some(AutoFilterData {
        range: CellRangeLocal {
            r0: 0,
            c0: 0,
            r1: MAX_ROWS - 1,
            c1: MAX_COLS - 1,
        },
        columns: (0..MAX_COLS)
            .map(|col| FilterColumnData {
                col,
                values: None,
                condition: Some(FilterConditionData {
                    op,
                    value: value.into(),
                }),
            })
            .collect(),
        filtered_rows: Vec::new(),
    });
    WorkbookData {
        sheets: vec![sheet],
        ..WorkbookData::default()
    }
}

/// 16,384 filtered columns over 1,048,576 rows (a crafted 48 KB XLSX):
/// a refresh reads only the filtered columns' cells, so it takes
/// milliseconds instead of minutes.
#[test]
fn whole_sheet_autofilter_refreshes_quickly() {
    use std::time::{Duration, Instant};
    use tally_model::FilterOpData;
    use tally_types::MAX_ROWS;
    for (op, value, hidden) in [
        // Every cell passes `<> zz`, empty ones too.
        (FilterOpData::Ne, "zz", vec![]),
        // Top 10 needs a number in all 16,384 columns: every row is hidden.
        (FilterOpData::TopValues, "10", vec![(1, MAX_ROWS - 1)]),
    ] {
        let mut wb =
            tally_engine::Workbook::from_data_owned(wide_filter(op, value, true), false, false);
        let sheet = wb.sheets[0].id;
        assert_eq!(
            wb.sheet(sheet)
                .unwrap()
                .meta
                .auto_filter
                .as_ref()
                .unwrap()
                .range
                .r1,
            MAX_ROWS - 1,
            "the far cell keeps the whole area"
        );
        let t = Instant::now();
        wb.reapply_filter(sheet);
        let took = t.elapsed();
        assert!(took < Duration::from_secs(1), "{op:?}: {took:?}");
        assert_eq!(
            wb.sheet(sheet).unwrap().filtered_rows.spans(),
            &hidden[..],
            "{op:?}"
        );
        // The popover's value lists read the cells only, too.
        let t = Instant::now();
        let info = tally_engine::filter::filter_info(&wb, sheet).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(info.columns[0].unique, vec!["1", ""]);
        assert_eq!(info.columns[0].counts, vec![1, MAX_ROWS - 2]);
    }
    // Without the far cell the area is clamped to the data at load.
    let wb = tally_engine::Workbook::from_data_owned(
        wide_filter(FilterOpData::Ne, "zz", false),
        false,
        false,
    );
    let af = wb.sheets[0].meta.auto_filter.as_ref().unwrap();
    assert_eq!((af.range.r1, af.range.c1), (2, 1));
    assert_eq!(af.columns.len(), 2);
}

/// 10,000 overlapping whole-sheet selection ranges read each cell once.
#[test]
fn overlapping_selection_ranges_are_merged() {
    let mut f = Fixture::new();
    for a in ["A1", "B2", "XFD1048576"] {
        f.set(a, "x");
    }
    let whole = tally_api::RangeDto {
        r0: 0,
        c0: 0,
        r1: tally_types::MAX_ROWS - 1,
        c1: tally_types::MAX_COLS - 1,
    };
    let q = FindQuery {
        selection: Some(vec![whole; 10_000]),
        ..q("x")
    };
    let t = std::time::Instant::now();
    let found = f.svc.find(f.doc, &q).unwrap();
    assert_eq!(found.hits.len(), 3);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
}
