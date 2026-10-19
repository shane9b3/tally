//! Plain interchange model for Tally workbooks.
//!
//! [`WorkbookData`] is what file readers produce and writers consume
//! (`tally-io-ods`, `tally-io-csv`), and what the engine converts to and from
//! with `Workbook::from_data` / `to_data`. It has no engine dependency and no
//! behaviour: just data with serde derives.
//!
//! This is a frozen WP0 contract; WP5 may add fields through a lead-reviewed
//! PR (see `docs/contributing.md`).
//!
//! Conventions:
//! - rows and columns are 0-based;
//! - colours are `0xRRGGBBAA` packed into a `u32`;
//! - lengths are in 1/100 mm (`_hmm`), like ODF.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use tally_types::{CalcSettings, Value};

/// A whole workbook.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkbookData {
    /// Sheets in tab order.
    pub sheets: Vec<SheetData>,
    /// Style table; [`CellData::style`] indexes into it.
    pub styles: Vec<StyleData>,
    /// Named expressions.
    pub names: Vec<NameData>,
    /// Document settings.
    pub settings: DocSettings,
    /// Document metadata.
    pub meta: DocMeta,
}

/// One sheet.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct SheetData {
    /// Sheet (tab) name.
    pub name: String,
    /// Non-empty or styled cells, sorted row-major (by row, then column).
    pub cells: Vec<CellData>,
    /// Column widths that differ from the default.
    pub col_widths: Vec<DimRun>,
    /// Row heights that differ from the default.
    pub row_heights: Vec<DimRun>,
    /// Merged areas.
    pub merges: Vec<CellRangeLocal>,
    /// Frozen panes as `(rows, cols)` frozen at the top/left.
    pub frozen: Option<(u32, u32)>,
    /// Tab colour (`0xRRGGBBAA`).
    pub tab_color: Option<u32>,
    /// Whether the sheet is hidden.
    pub hidden: bool,
}

/// One cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellData {
    /// 0-based row.
    pub row: u32,
    /// 0-based column.
    pub col: u32,
    /// Value or formula.
    pub content: CellContentData,
    /// Index into [`WorkbookData::styles`], or `None` for the default style.
    pub style: Option<u32>,
}

/// The content of a cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CellContentData {
    /// A constant value ([`Value::Empty`] for a styled but empty cell).
    Value(Value),
    /// A formula.
    Formula {
        /// Formula text in the OpenFormula dialect, e.g. `of:=SUM([.A1:.A3])`.
        text: String,
        /// The cached result stored in the file, if any.
        cached: Option<Value>,
    },
}

impl Default for CellContentData {
    fn default() -> Self {
        CellContentData::Value(Value::Empty)
    }
}

/// A cell style.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct StyleData {
    /// Font attributes.
    pub font: FontData,
    /// Background fill (`0xRRGGBBAA`), or `None` for no fill.
    pub fill: Option<u32>,
    /// Alignment.
    pub align: AlignData,
    /// Cell borders.
    pub borders: BordersData,
    /// OOo number format code, or `None` for General.
    pub number_format: Option<String>,
    /// Special display treatment (chips).
    pub display: Option<DisplayData>,
}

/// Font attributes.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct FontData {
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Single underline.
    pub underline: bool,
    /// Strikethrough.
    pub strike: bool,
    /// Font family, or `None` for the default.
    pub family: Option<String>,
    /// Size in points, or `None` for the default.
    pub size_pt: Option<f32>,
    /// Text colour (`0xRRGGBBAA`), or `None` for automatic.
    pub color: Option<u32>,
}

/// Horizontal alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HAlign {
    /// Numbers right, text left, booleans and errors centred.
    #[default]
    General,
    /// Left.
    Left,
    /// Centre.
    Center,
    /// Right.
    Right,
    /// Justified.
    Justify,
}

/// Vertical alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum VAlign {
    /// Bottom (the spreadsheet default).
    #[default]
    Bottom,
    /// Middle.
    Middle,
    /// Top.
    Top,
}

/// Alignment attributes.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct AlignData {
    /// Horizontal alignment.
    pub h: HAlign,
    /// Vertical alignment.
    pub v: VAlign,
    /// Wrap text.
    pub wrap: bool,
    /// Indent level.
    pub indent: u8,
}

/// Line style of a border.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum BorderStyle {
    /// Solid line.
    #[default]
    Solid,
    /// Dashed line.
    Dashed,
    /// Dotted line.
    Dotted,
    /// Double line.
    Double,
}

/// One border line.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BorderLine {
    /// Line style.
    pub style: BorderStyle,
    /// Width in 1/100 mm.
    pub width_hmm: u32,
    /// Colour (`0xRRGGBBAA`).
    pub color: u32,
}

/// The four borders of a cell; `None` means no border.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BordersData {
    /// Top border.
    pub top: Option<BorderLine>,
    /// Bottom border.
    pub bottom: Option<BorderLine>,
    /// Left border.
    pub left: Option<BorderLine>,
    /// Right border.
    pub right: Option<BorderLine>,
}

/// Special display styles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DisplayData {
    /// Render the value as a rounded "pill" with these colours (`0xRRGGBBAA`).
    /// Persisted to ODS as an extension attribute; other apps see a fill.
    Chip {
        /// Background colour.
        bg: u32,
        /// Text colour.
        fg: u32,
    },
}

/// A named expression.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct NameData {
    /// The name.
    pub name: String,
    /// Index of the sheet the name is local to, or `None` for workbook scope.
    pub sheet: Option<u32>,
    /// Expression in the OpenFormula dialect, e.g. `of:=[$Sheet1.$A$1:.$A$10]`.
    pub formula: String,
}

/// Document-level settings.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DocSettings {
    /// Index of the active sheet in [`WorkbookData::sheets`].
    pub active_sheet: u32,
    /// Calculation settings.
    pub calc_settings: CalcSettings,
}

/// Document metadata. Timestamps are ISO 8601 strings as stored in the file.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DocMeta {
    /// Document title.
    pub title: Option<String>,
    /// Application that wrote the file.
    pub generator: Option<String>,
    /// Creation time.
    pub created: Option<String>,
    /// Last modification time.
    pub modified: Option<String>,
}

/// A run of rows or columns `start..=end` sharing one size.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DimRun {
    /// First index (inclusive).
    pub start: u32,
    /// Last index (inclusive).
    pub end: u32,
    /// Size in 1/100 mm.
    pub size_hmm: u32,
    /// Hidden (also used for filtered rows).
    pub hidden: bool,
}

/// An inclusive range on an implied sheet (the sheet that contains it).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct CellRangeLocal {
    /// First row.
    pub r0: u32,
    /// First column.
    pub c0: u32,
    /// Last row.
    pub r1: u32,
    /// Last column.
    pub c1: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tally_types::{ErrorKind, IterSettings, Matrix};

    fn sample() -> WorkbookData {
        let bold = StyleData {
            font: FontData {
                bold: true,
                family: Some("Inter".into()),
                size_pt: Some(11.0),
                color: Some(0x1122_33ff),
                ..FontData::default()
            },
            fill: Some(0xffee_ddff),
            align: AlignData {
                h: HAlign::Right,
                v: VAlign::Middle,
                wrap: true,
                indent: 1,
            },
            borders: BordersData {
                bottom: Some(BorderLine {
                    style: BorderStyle::Double,
                    width_hmm: 26,
                    color: 0x0000_00ff,
                }),
                ..BordersData::default()
            },
            number_format: Some("#,##0.00".into()),
            display: Some(DisplayData::Chip {
                bg: 0xd1fa_e5ff,
                fg: 0x0657_4fff,
            }),
        };
        let sheet1 =
            SheetData {
                name: "Orders".into(),
                cells: vec![
                    CellData {
                        row: 0,
                        col: 0,
                        content: CellContentData::Value(Value::Number(2.0)),
                        style: Some(0),
                    },
                    CellData {
                        row: 0,
                        col: 1,
                        content: CellContentData::Value(Value::text("Paid")),
                        style: None,
                    },
                    CellData {
                        row: 1,
                        col: 0,
                        content: CellContentData::Formula {
                            text: "of:=SUM([.A1:.A1])".into(),
                            cached: Some(Value::Number(2.0)),
                        },
                        style: None,
                    },
                    CellData {
                        row: 1,
                        col: 1,
                        content: CellContentData::Formula {
                            text: "of:=1/0".into(),
                            cached: Some(Value::Error(ErrorKind::Div0)),
                        },
                        style: None,
                    },
                    CellData {
                        row: 2,
                        col: 0,
                        content: CellContentData::Value(Value::Array(Arc::new(
                            Matrix::new_filled(1, 2, Value::Bool(true)),
                        ))),
                        style: None,
                    },
                ],
                col_widths: vec![DimRun {
                    start: 0,
                    end: 2,
                    size_hmm: 2258,
                    hidden: false,
                }],
                row_heights: vec![DimRun {
                    start: 5,
                    end: 5,
                    size_hmm: 452,
                    hidden: true,
                }],
                merges: vec![CellRangeLocal {
                    r0: 3,
                    c0: 0,
                    r1: 3,
                    c1: 2,
                }],
                frozen: Some((1, 0)),
                tab_color: Some(0x3366_ccff),
                hidden: false,
            };
        WorkbookData {
            sheets: vec![
                sheet1,
                SheetData {
                    name: "Empty".into(),
                    hidden: true,
                    ..SheetData::default()
                },
            ],
            styles: vec![bold],
            names: vec![NameData {
                name: "Total".into(),
                sheet: None,
                formula: "of:=[$Orders.$A$1:.$A$2]".into(),
            }],
            settings: DocSettings {
                active_sheet: 1,
                calc_settings: CalcSettings {
                    iterations: Some(IterSettings::default()),
                    ..CalcSettings::default()
                },
            },
            meta: DocMeta {
                title: Some("Sample".into()),
                generator: Some("Tally/0.1.0".into()),
                created: Some("2026-10-19T08:00:00".into()),
                modified: None,
            },
        }
    }

    #[test]
    fn workbook_json_round_trip() {
        let wb = sample();
        let json = serde_json::to_string_pretty(&wb).unwrap();
        let back: WorkbookData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wb);
    }

    #[test]
    fn defaults() {
        let wb = WorkbookData::default();
        assert!(wb.sheets.is_empty());
        assert_eq!(wb.settings.active_sheet, 0);
        assert_eq!(
            CellContentData::default(),
            CellContentData::Value(Value::Empty)
        );
        let s = StyleData::default();
        assert_eq!(s.align.h, HAlign::General);
        assert_eq!(s.align.v, VAlign::Bottom);
        assert!(s.number_format.is_none());
    }
}
