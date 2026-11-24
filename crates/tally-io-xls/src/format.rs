//! The BIFF8 formatting reader: everything about the look of a workbook
//! that calamine does not expose.
//!
//! It walks the workbook stream the pre-scan extracted and validated
//! ([`crate::prescan`]; the same record framing, substream walk and
//! `CONTINUE` handling) and reads
//!
//! - the workbook globals: `FONT`, `FORMAT`, `XF`, `PALETTE`, `WINDOW1`
//!   (active sheet), the `MSODRAWINGGROUP` picture store and the built-in
//!   `NAME`s `Print_Area` and `Print_Titles`;
//! - per sheet: `COLINFO`, `DEFCOLWIDTH`, `STANDARDWIDTH`, `ROW` (height,
//!   hidden, default format), `DEFAULTROWHEIGHT`, the XF index of every
//!   cell record (`BLANK` and `MULBLANK` included), `WINDOW2`, `PANE`,
//!   `SCL`, `SHEETEXT`, the page setup (`SETUP`, the four margin records,
//!   `HEADER`, `FOOTER`, `HCENTER`, `VCENTER`, `PRINTGRIDLINES`,
//!   `PRINTHEADERS`, `WSBOOL`, the two page break records), the
//!   `MSODRAWING` picture anchors and cell notes (`NOTE` with the text of
//!   its `TXO`).
//!
//! Column widths, row heights and the defaults for both follow the
//! OpenOffice 4.1 import filter (`XclImpColRowSettings`): every row and
//! column of the sheet gets the size the file gives it, so the sheet is
//! laid out as in OpenOffice. The rest maps onto the model the way the
//! XLSX reader maps the equivalent constructs (its palette, border table,
//! number-format translation and page setup conversion are reused).
//!
//! This pass never fails a file: a record that is too short, a count over
//! a limit or an index without a target is skipped, and what was dropped
//! is reported as a warning. Every allocation is bounded by a field of
//! [`Limits`] or by the size of the stream, and the work is linear in the
//! stream size. BIFF5 and older streams (different `XF`/`FONT` layouts)
//! are not read: the workbook then opens with its values only.

use std::collections::HashMap;
use std::sync::Arc;

use tally_model::{
    AlignData, BorderLine, BordersData, CellProtectionData, CellRangeLocal, ChartAnchorData,
    DimRun, FontData, HAlign, ImageData, LinkData, MarginsData, NoteData, OrientationData,
    PageSetupData, PaperData, SplitData, StyleData, StyleRun, UnderlineStyleData, VAlign,
    ViewOptionsData,
};
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::names::NameRecs;
use crate::prescan::{bof_is_pre_biff8, record, u16_at, u32_at, Rec};
use crate::{hlink, Limits};

/// Columns of a BIFF8 sheet.
const BIFF_COLS: usize = 256;
/// Rows of a BIFF8 sheet.
const BIFF_ROWS: u32 = 65_536;
/// Most entries of the picture store looked at.
const MAX_BLIPS: usize = 65_536;
/// OpenOffice's standard column width (`STD_COL_WIDTH`), twips: the width
/// of the columns of a sheet without `DEFCOLWIDTH` and `STANDARDWIDTH`.
const STD_COL_WIDTH_TWIPS: u16 = 1285;
/// OpenOffice's standard row height (`ScGlobal::nStdRowHeight`), twips:
/// the height of the rows of a sheet without `DEFAULTROWHEIGHT`.
const STD_ROW_HEIGHT_TWIPS: u16 = 256;
/// The height of font 0 when the workbook has no `FONT` record, twips
/// (`XclImpFontBuffer::Initialize`: Arial 10).
const APP_FONT_TWIPS: u16 = 200;
/// Largest page margin kept, 1/100 mm (50 in).
const MAX_MARGIN_HMM: f64 = 127_000.0;
/// Most areas of a `Print_Area` or `Print_Titles` name looked at.
const MAX_NAME_AREAS: usize = 8;
/// Most cells one hyperlink may cover (as in the XLSX reader).
const MAX_LINK_CELLS: u64 = 10_000;

/// What a cell format (XF) maps to.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct XfInfo {
    /// Index into [`Formatting::styles`]; `None` for the default look.
    pub style: Option<u32>,
    /// The number format shows a calendar date.
    pub date: bool,
    /// The XF has a number format other than General.
    pub has_format: bool,
    /// Nothing but the workbook's default font: an empty cell with this
    /// format is not worth a cell.
    pub plain: bool,
}

/// The format of one cell record, or of an empty cell of a row with a
/// default format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellFmt {
    pub row: u32,
    pub col: u16,
    pub xf: u16,
}

/// The formatting of one sheet.
#[derive(Debug, Default)]
pub(crate) struct SheetFormat {
    pub col_widths: Vec<DimRun>,
    pub row_heights: Vec<DimRun>,
    pub style_runs: Vec<StyleRun>,
    /// The columns (of the 256) that have a style run: a cell there needs
    /// a style of its own, the default look included, or it would take
    /// the column's.
    pub styled_cols: Vec<bool>,
    /// Cells whose XF is not the default look, cells in a styled column
    /// or in a row with a default format, and the empty cells such a row
    /// formats; sorted row-major, one per position.
    pub cells: Vec<CellFmt>,
    pub page_setup: Option<PageSetupData>,
    /// Manual page breaks before these rows / columns, sorted.
    pub row_breaks: Vec<u32>,
    pub col_breaks: Vec<u32>,
    pub view_options: Option<ViewOptionsData>,
    pub frozen: Option<(u32, u32)>,
    pub split: Option<SplitData>,
    pub zoom: Option<u16>,
    pub tab_color: Option<u32>,
    pub images: Vec<ImageData>,
    /// Cell notes, sorted row-major.
    pub notes: Vec<NoteData>,
    /// Cell hyperlinks, sorted row-major, one per cell.
    pub links: Vec<LinkData>,
}

/// The formatting of a workbook.
#[derive(Debug, Default)]
pub(crate) struct Formatting {
    /// Deduplicated model styles.
    pub styles: Vec<StyleData>,
    /// Per XF index.
    pub xfs: Vec<XfInfo>,
    /// The family of font 0.
    pub default_font: Option<String>,
    /// `WINDOW1`'s selected tab.
    pub active_sheet: Option<u32>,
    /// One entry per `BOUNDSHEET`, in order.
    pub sheets: Vec<SheetFormat>,
    /// The defined names, to be resolved with the sheet names.
    pub names: NameRecs,
    /// What was skipped.
    pub warnings: Vec<String>,
}

impl Formatting {
    /// The XF at `index`.
    pub fn xf(&self, index: u16) -> Option<&XfInfo> {
        self.xfs.get(usize::from(index))
    }
}

// ------------------------------------------------------------ raw records

#[derive(Clone, Debug, Default)]
struct RawFont {
    twips: u16,
    grbit: u16,
    icv: u16,
    bls: u16,
    uls: u8,
    name: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
struct RawXf {
    ifnt: u16,
    ifmt: u16,
    flags: u16,
    align: u8,
    rotation: u8,
    indent: u8,
    lines: u16,
    colors1: u16,
    colors2: u32,
    fill: u16,
}

/// An XF's fields: equal keys are equal looks.
type XfKey = (u16, u16, u16, [u8; 3], u16, u16, u32, u16);

impl RawXf {
    fn key(&self) -> XfKey {
        (
            self.ifnt,
            self.ifmt,
            // Locked and hidden; the rest (style XF, parent) is not read.
            self.flags & 3,
            [self.align, self.rotation, self.indent],
            self.lines,
            self.colors1,
            self.colors2,
            self.fill,
        )
    }
}

/// A built-in `NAME` that sets up printing: `Print_Area` or
/// `Print_Titles` of one sheet.
struct PrintName {
    /// Index of the sheet (the name's `itab` − 1).
    sheet: usize,
    /// `Print_Titles`, else `Print_Area`.
    titles: bool,
    areas: Vec<CellRangeLocal>,
}

impl PrintName {
    /// The rows and the columns to repeat: an area over all 256 columns
    /// names rows, one over all 65,536 rows names columns.
    #[allow(clippy::type_complexity)]
    fn titles(&self) -> (Option<(u32, u32)>, Option<(u32, u32)>) {
        let mut rows = None;
        let mut cols = None;
        for a in &self.areas {
            if a.c0 == 0 && a.c1 == BIFF_COLS as u32 - 1 {
                rows = Some((a.r0, a.r1));
            } else if a.r0 == 0 && a.r1 == BIFF_ROWS - 1 {
                cols = Some((a.c0, a.c1));
            }
        }
        (rows, cols)
    }
}

/// Reads a `NAME` record that is the built-in `Print_Area` (0x06) or
/// `Print_Titles` (0x07) of a sheet. Its formula may only hold areas and
/// cells of a sheet (`ptgArea3d`, `ptgRef3d`), joined by `ptgUnion` under
/// a `ptgMemFunc`; anything else is not a print range Tally can keep, and
/// the name is skipped.
fn print_name(d: &[u8]) -> Option<PrintName> {
    let flags = u16_at(d, 0)?;
    let cch = *d.get(3)?;
    let cce = usize::from(u16_at(d, 4)?);
    let itab = usize::from(u16_at(d, 8)?);
    // Built-in, one character, local to a sheet.
    if flags & 0x20 == 0 || cch != 1 || itab == 0 {
        return None;
    }
    // The name: option flags, then the built-in's id as its character.
    let wide = d.get(14)? & 1 != 0;
    let id = *d.get(15)?;
    let titles = match id {
        0x06 => false,
        0x07 => true,
        _ => return None,
    };
    let at: usize = if wide { 17 } else { 16 };
    let rgce = d.get(at..at.checked_add(cce)?)?;
    let mut areas: Vec<CellRangeLocal> = Vec::new();
    let mut p = 0usize;
    while let Some(&ptg) = rgce.get(p) {
        let (r0, r1, c0, c1, len) = match ptg {
            // ptgUnion, ptgParen.
            0x10 | 0x15 => {
                p += 1;
                continue;
            }
            // ptgMemFunc: the length of the expression that follows.
            0x29 | 0x49 | 0x69 => {
                p += 3;
                continue;
            }
            // ptgRef3d: sheet, row, column.
            0x3A | 0x5A | 0x7A => {
                let (r, c) = (u16_at(rgce, p + 3)?, u16_at(rgce, p + 5)?);
                (r, r, c, c, 7)
            }
            // ptgArea3d: sheet, rows, columns.
            0x3B | 0x5B | 0x7B => (
                u16_at(rgce, p + 3)?,
                u16_at(rgce, p + 5)?,
                u16_at(rgce, p + 7)?,
                u16_at(rgce, p + 9)?,
                11,
            ),
            _ => return None,
        };
        // The column's upper bits say "relative".
        let (c0, c1) = (c0 & 0xFF, c1 & 0xFF);
        if r0 > r1 || c0 > c1 || areas.len() >= MAX_NAME_AREAS {
            return None;
        }
        areas.push(CellRangeLocal {
            r0: u32::from(r0),
            c0: u32::from(c0),
            r1: u32::from(r1),
            c1: u32::from(c1),
        });
        p += len;
    }
    (!areas.is_empty()).then_some(PrintName {
        sheet: itab - 1,
        titles,
        areas,
    })
}

/// The deduplicated model styles. Styles are found through a hash of
/// their debug text (which is not kept) and compared for equality.
#[derive(Default)]
struct StyleTable {
    styles: Vec<StyleData>,
    by_hash: HashMap<u64, Vec<u32>>,
}

impl StyleTable {
    fn hash(style: &StyleData) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{style:?}").hash(&mut h);
        h.finish()
    }

    /// The index of `style`; `None` for the default look and for a style
    /// that is not in the table.
    fn find(&self, style: &StyleData) -> Option<u32> {
        if *style == StyleData::default() {
            return None;
        }
        self.by_hash
            .get(&Self::hash(style))?
            .iter()
            .copied()
            .find(|i| self.styles.get(*i as usize) == Some(style))
    }

    fn push(&mut self, style: StyleData) -> u32 {
        let next = u32::try_from(self.styles.len()).unwrap_or(u32::MAX);
        self.by_hash
            .entry(Self::hash(&style))
            .or_default()
            .push(next);
        self.styles.push(style);
        next
    }
}

/// What the pass dropped, reported once each.
#[derive(Default)]
struct Dropped {
    fonts: bool,
    formats: bool,
    long_format: bool,
    xfs: bool,
    col_infos: bool,
    rows: bool,
    cells: bool,
    style_text: bool,
    page_breaks: bool,
    drawing_bytes: bool,
    images: bool,
    notes: bool,
    links: bool,
    /// Hyperlinks that could not be read or cover too many cells.
    bad_links: usize,
    walk: bool,
    unsupported_pictures: usize,
    other_shapes: usize,
}

/// A picture of the workbook's store.
#[derive(Clone)]
enum Blip {
    Picture(&'static str, Arc<[u8]>),
    /// EMF, WMF, PICT, DIB, TIFF.
    Unsupported,
    /// Absent, truncated or over the size limit.
    Missing,
}

/// Decodes `cch` characters of a BIFF8 string whose option flags are at
/// `d[at]` (bit 0: 16-bit characters, else Latin-1 bytes); a short string
/// is cut at the end of the data.
fn unicode(d: &[u8], at: usize, cch: usize) -> String {
    let Some(&flags) = d.get(at) else {
        return String::new();
    };
    let mut p = at.saturating_add(1);
    if flags & 0x08 != 0 {
        p = p.saturating_add(2);
    }
    if flags & 0x04 != 0 {
        p = p.saturating_add(4);
    }
    let rest = d.get(p..).unwrap_or_default();
    if flags & 1 != 0 {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .take(cch)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        rest.iter().take(cch).map(|b| char::from(*b)).collect()
    }
}

/// The record's own data (without its `CONTINUE` records).
fn body<'a>(s: &'a [u8], r: &Rec) -> &'a [u8] {
    s.get(r.data..r.data.saturating_add(r.len))
        .unwrap_or_default()
}

/// Appends the record's data and its continues to `out`, up to `cap`
/// bytes in all; `false` when the cap cut it.
fn append(s: &[u8], r: &Rec, out: &mut Vec<u8>, cap: usize) -> bool {
    let parts = std::iter::once((r.data, r.len)).chain(r.cont.iter().copied());
    for (at, len) in parts {
        let d = s.get(at..at.saturating_add(len)).unwrap_or_default();
        if out.len().saturating_add(d.len()) > cap {
            return false;
        }
        out.extend_from_slice(d);
    }
    true
}

// ------------------------------------------------------------ colours

/// The colour table: indices 0–7 fixed, 8–63 the (replaceable) palette,
/// 0x40… system colours.
struct Palette([u32; 64]);

impl Palette {
    /// The colour of index `icv` as `0xRRGGBB`; `None` for "automatic"
    /// and unknown indices.
    fn rgb(&self, icv: u16) -> Option<u32> {
        match icv {
            0..=63 => self.0.get(usize::from(icv)).copied(),
            // System window text / background, 3D face, chart and tooltip
            // colours.
            0x40 | 0x4D | 0x4F | 0x51 => Some(0x00_0000),
            0x41 | 0x4E => Some(0xFF_FFFF),
            0x43 => Some(0xC0_C0C0),
            0x50 => Some(0xFF_FFE1),
            _ => None,
        }
    }

    /// A text or line colour: system and automatic indices are `None`.
    fn explicit(&self, icv: u16) -> Option<u32> {
        if icv >= 0x40 {
            return None;
        }
        self.rgb(icv).map(|c| (c << 8) | 0xff)
    }
}

/// OpenOffice's share of the background colour (of 0x80) in each fill
/// pattern (`XclTools::GetPatternColor`): pattern 1 is solid foreground,
/// the others are shown as the mix a viewer sees from a distance.
const PATTERN_BACK_SHARE: [u32; 19] = [
    0x80, 0x00, 0x40, 0x20, 0x60, 0x40, 0x40, 0x40, 0x40, 0x40, 0x20, 0x60, 0x60, 0x60, 0x60, 0x48,
    0x50, 0x70, 0x78,
];

fn mix(fore: u32, back: u32, share: u32) -> u32 {
    let ch = |shift: u32| {
        let f = (fore >> shift) & 0xff;
        let b = (back >> shift) & 0xff;
        // f + (b - f) * share / 0x80, without going negative.
        ((f * (0x80 - share) + b * share) / 0x80).min(0xff)
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

// ------------------------------------------------------------ styles

fn line_name(dg: u32) -> Option<&'static str> {
    Some(match dg {
        1 => "thin",
        2 => "medium",
        3 => "dashed",
        4 => "dotted",
        5 => "thick",
        6 => "double",
        7 => "hair",
        8 => "mediumDashed",
        9 => "dashDot",
        10 => "mediumDashDot",
        11 => "dashDotDot",
        12 => "mediumDashDotDot",
        13 => "slantDashDot",
        _ => return None,
    })
}

struct StyleBuilder<'a> {
    fonts: &'a [RawFont],
    formats: &'a HashMap<u16, String>,
    palette: &'a Palette,
    default_family: Option<String>,
    fmt_cache: HashMap<u16, Option<String>>,
    long_format: bool,
}

/// The code of a built-in number format of a `.xls` where OpenOffice's
/// table (`spBuiltInFormats`, xlstyle.cxx) differs from the one the XLSX
/// reader uses (ECMA-376): 32–35 are times, and the ids past 58 reuse
/// earlier ones.
fn builtin_format_code(ifmt: u16) -> Option<&'static str> {
    let reuse = match ifmt {
        32..=35 | 77 => 21,
        59 => 1,
        60 => 2,
        61 => 3,
        62 => 4,
        67 => 9,
        68 => 10,
        69 => 12,
        70 => 13,
        71 | 72 => 14,
        73 => 15,
        74 => 16,
        75 => 17,
        76 => 20,
        78 => 22,
        79 => 45,
        80 => 46,
        81 => 47,
        other => other,
    };
    tally_io_xlsx::builtin_format_code(u32::from(reuse))
}

impl StyleBuilder<'_> {
    /// BIFF font indices skip 4: the fifth `FONT` record is font 5. Font 4
    /// itself is not in the file: it is font 0 in bold (`true`), as in
    /// OpenOffice (`XclImpFontBuffer::UpdateAppFont`).
    fn font(&self, ifnt: u16) -> (Option<&RawFont>, bool) {
        let i = match ifnt {
            0..=3 => usize::from(ifnt),
            4 => return (self.fonts.first(), true),
            _ => usize::from(ifnt) - 1,
        };
        (self.fonts.get(i).or_else(|| self.fonts.first()), false)
    }

    fn font_data(&self, ifnt: u16) -> FontData {
        let (Some(f), font4) = self.font(ifnt) else {
            return FontData::default();
        };
        let size = f32::from(f.twips.min(409 * 20)) / 20.0;
        FontData {
            // OpenOffice shows weights from 550 up (semibold, bold, …) in
            // bold (`XclFontData::GetScWeight`).
            bold: font4 || f.bls >= 550,
            italic: f.grbit & 0x02 != 0,
            underline: f.uls != 0,
            strike: f.grbit & 0x08 != 0,
            family: f
                .name
                .clone()
                .filter(|n| Some(n) != self.default_family.as_ref()),
            // A size of its own unless it is the size cells without one
            // are drawn at.
            size_pt: (f.twips > 0 && size != tally_model::DEFAULT_FONT_SIZE_PT).then_some(size),
            color: self.palette.explicit(f.icv),
            underline_style: match f.uls {
                2 => UnderlineStyleData::Double,
                0x21 => UnderlineStyleData::SingleAccounting,
                0x22 => UnderlineStyleData::DoubleAccounting,
                _ => UnderlineStyleData::Single,
            },
        }
    }

    fn number_format(&mut self, ifmt: u16) -> Option<String> {
        if let Some(c) = self.fmt_cache.get(&ifmt) {
            return c.clone();
        }
        let code = self
            .formats
            .get(&ifmt)
            .map(String::as_str)
            .or_else(|| builtin_format_code(ifmt));
        let ooo = match code {
            Some(c) if c.chars().count() > tally_model::MAX_NUMBER_FORMAT_LEN => {
                self.long_format = true;
                None
            }
            Some(c) => tally_io_xlsx::excel_to_ooo(c),
            None => None,
        };
        self.fmt_cache.insert(ifmt, ooo.clone());
        ooo
    }

    fn line(&self, dg: u32, icv: u32) -> Option<BorderLine> {
        let (style, width_hmm) = tally_io_xlsx::border_line_style(line_name(dg)?);
        Some(BorderLine {
            style,
            width_hmm,
            color: self.palette.explicit(icv as u16).unwrap_or(0x0000_00ff),
        })
    }

    /// The effective formatting of an XF. A cell XF's own fields are what
    /// Excel and OpenOffice display (the "used attribute" flags only say
    /// which groups differ from the parent style XF, whose values the
    /// writer copies into the cell XF), so neither the flags nor the
    /// parent are consulted, as in the XLSX reader.
    fn style(&mut self, x: &RawXf) -> StyleData {
        let c2 = x.colors2;
        let lines = u32::from(x.lines);
        let c1 = u32::from(x.colors1);
        let mut borders = BordersData {
            left: self.line(lines & 15, c1 & 0x7F),
            right: self.line((lines >> 4) & 15, (c1 >> 7) & 0x7F),
            top: self.line((lines >> 8) & 15, c2 & 0x7F),
            bottom: self.line((lines >> 12) & 15, (c2 >> 7) & 0x7F),
            ..BordersData::default()
        };
        let diagonal = self.line((c2 >> 21) & 15, (c2 >> 14) & 0x7F);
        if c1 & 0x4000 != 0 {
            borders.diagonal_down.clone_from(&diagonal);
        }
        if c1 & 0x8000 != 0 {
            borders.diagonal_up = diagonal;
        }
        let pattern = (c2 >> 26) as usize;
        let fore = self.palette.rgb(x.fill & 0x7F);
        let back = self.palette.rgb((x.fill >> 7) & 0x7F);
        let fill = match pattern {
            0 => None,
            _ => {
                let share = PATTERN_BACK_SHARE.get(pattern).copied().unwrap_or(0);
                let fore = fore.unwrap_or(0x00_0000);
                let back = back.unwrap_or(0xFF_FFFF);
                Some((mix(fore, back, share) << 8) | 0xff)
            }
        };
        let protection = CellProtectionData {
            locked: x.flags & 1 != 0,
            hidden_formula: x.flags & 2 != 0,
            ..CellProtectionData::default()
        };
        let rot = i16::from(x.rotation);
        StyleData {
            font: self.font_data(x.ifnt),
            fill,
            align: AlignData {
                h: match x.align & 7 {
                    // Fill repeats the content; closest is left.
                    1 | 4 => HAlign::Left,
                    // Centre and centre across selection.
                    2 | 6 => HAlign::Center,
                    3 => HAlign::Right,
                    // Justify and distributed.
                    5 | 7 => HAlign::Justify,
                    _ => HAlign::General,
                },
                v: match (x.align >> 4) & 7 {
                    0 => VAlign::Top,
                    1 | 3 | 4 => VAlign::Middle,
                    _ => VAlign::Bottom,
                },
                wrap: x.align & 0x08 != 0,
                indent: x.indent & 15,
                rotation_deg: match rot {
                    0..=90 => rot,
                    91..=180 => 90 - rot,
                    _ => 0,
                },
                stacked: x.rotation == 255,
                shrink_to_fit: x.indent & 0x10 != 0,
            },
            borders,
            number_format: self.number_format(x.ifmt),
            display: None,
            protection: (protection != CellProtectionData::default()).then_some(protection),
            style_name: None,
        }
    }
}

// ------------------------------------------------------------ OfficeArt

struct ArtRec<'a> {
    inst: u16,
    typ: u16,
    /// Empty for a container: its children follow as records of their own.
    body: &'a [u8],
}

/// Calls `f` for every OfficeArt record of `d` in file order, containers
/// flattened (a container's header is reported, then its children). A
/// record that claims more bytes than there are ends the scan. Linear:
/// every step consumes at least a header.
fn art_scan<'a>(d: &'a [u8], mut f: impl FnMut(ArtRec<'a>)) {
    let mut p = 0usize;
    while let (Some(vi), Some(typ), Some(len)) = (u16_at(d, p), u16_at(d, p + 2), u32_at(d, p + 4))
    {
        let start = p + 8;
        let inst = vi >> 4;
        if vi & 0xF == 0xF {
            f(ArtRec {
                inst,
                typ,
                body: &[],
            });
            p = start;
            continue;
        }
        let Some(end) = start.checked_add(len as usize) else {
            break;
        };
        let Some(b) = d.get(start..end) else { break };
        f(ArtRec { inst, typ, body: b });
        p = end;
    }
}

/// The picture of a BLIP store entry (`FBSE` and the BLIP record in it).
fn blip_of(bse: &[u8], max_bytes: u64) -> Blip {
    let Some(&cb_name) = bse.get(33) else {
        return Blip::Missing;
    };
    let at = 36 + usize::from(cb_name);
    let (Some(vi), Some(typ), Some(len)) =
        (u16_at(bse, at), u16_at(bse, at + 2), u32_at(bse, at + 4))
    else {
        return Blip::Missing;
    };
    let Some(data) = (at + 8)
        .checked_add(len as usize)
        .and_then(|end| bse.get(at + 8..end))
    else {
        // The BLIP claims more bytes than its entry holds.
        return Blip::Missing;
    };
    blip_record(vi >> 4, typ, data, max_bytes)
}

/// The picture of a BLIP record (`inst`, type and data), inside a store
/// entry or directly in the store.
fn blip_record(inst: u16, typ: u16, data: &[u8], max_bytes: u64) -> Blip {
    match typ {
        // JPEG, PNG, JPEG (CMYK).
        0xF01D | 0xF01E | 0xF02A => {}
        0xF01A..=0xF01C | 0xF01F | 0xF029 => return Blip::Unsupported,
        _ => return Blip::Missing,
    }
    // One or two 16-byte UIDs (odd instance: two) and a tag byte, then the
    // file. What it is is taken from its signature, as in the XLSX
    // reader: writers are known to store a PNG in a JPEG record.
    let first = if inst & 1 == 1 { 33 } else { 17 };
    for skip in [first, 50 - first] {
        let Some(pic) = data.get(skip..) else {
            continue;
        };
        let mime = if pic.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if pic.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else {
            continue;
        };
        if pic.len() as u64 > max_bytes {
            return Blip::Missing;
        }
        return Blip::Picture(mime, Arc::from(pic));
    }
    Blip::Unsupported
}

/// One shape of a sheet's drawing.
#[derive(Default)]
struct Shape {
    pib: Option<u32>,
    anchor: Option<[u16; 8]>,
}

// ------------------------------------------------------------ sheets

#[derive(Clone, Copy)]
struct ColDef {
    /// 1/256 of a character.
    width: u16,
    hidden: bool,
    /// The style of the column's XF when that is not the plain look: the
    /// column gets a style run.
    style: Option<u32>,
}

#[derive(Clone, Copy)]
struct RowRec {
    row: u32,
    twips: u16,
    flags: u32,
}

impl RowRec {
    /// The row's default format (`fGhostDirty` and the XF in bits 16–27),
    /// which OpenOffice applies to the row's cells without a record
    /// (`ImportExcel::Row34`).
    fn default_xf(&self) -> Option<u16> {
        (self.flags & 0x80 != 0).then_some(((self.flags >> 16) & 0x0FFF) as u16)
    }
}

/// Sizes along one axis for picture anchors: explicit sizes (sorted by
/// index) with running sums, and the default for the others. 1/100 mm.
struct Axis {
    /// (index, size, sum of the sizes before this entry).
    explicit: Vec<(u32, i64, i64)>,
    default: i64,
}

impl Axis {
    fn new(mut sizes: Vec<(u32, i64)>, default: i64) -> Self {
        sizes.sort_by_key(|e| e.0);
        let mut explicit: Vec<(u32, i64, i64)> = Vec::with_capacity(sizes.len());
        let mut sum = 0i64;
        for (i, size) in sizes {
            if let Some(last) = explicit.last_mut() {
                if last.0 == i {
                    // The later record wins.
                    sum += size - last.1;
                    last.1 = size;
                    continue;
                }
            }
            explicit.push((i, size, sum));
            sum += size;
        }
        Self { explicit, default }
    }

    fn size(&self, i: u32) -> i64 {
        match self.explicit.binary_search_by_key(&i, |e| e.0) {
            Ok(k) => self.explicit[k].1,
            Err(_) => self.default,
        }
    }

    /// The summed sizes of the indices below `i`.
    fn before(&self, i: u32) -> i64 {
        let k = self.explicit.partition_point(|e| e.0 < i);
        let explicit_sum = match self.explicit.get(k) {
            Some(e) => e.2,
            None => self.explicit.last().map_or(0, |e| e.1 + e.2),
        };
        explicit_sum + (i64::from(i) - k as i64) * self.default
    }
}

struct Reader<'a> {
    s: &'a [u8],
    limits: &'a Limits,
    xfs: Vec<XfInfo>,
    palette: Palette,
    /// The height of font 0, twips.
    font_twips: i64,
    /// The width of a character of font 0, twips: the unit of column
    /// widths (`XclRoot::SetCharWidth`).
    char_width: i64,
    blips: Vec<Blip>,
    dropped: Dropped,
    /// The rows of the sheet being read that have a default format (one
    /// bit per BIFF row).
    row_xf: Vec<u64>,
    // Workbook-wide counters.
    rows: usize,
    cells: u64,
    drawing_bytes: usize,
    images: usize,
    image_bytes: u64,
    notes: usize,
    /// Cells with a hyperlink, and the bytes of the link targets.
    links: usize,
    link_bytes: u64,
}

/// A `NOTE` record: where the note is and which object holds its text.
struct NoteRec {
    row: u32,
    col: u32,
    shown: bool,
    obj: u16,
    author: Option<String>,
}

/// The text of a `TXO` record: `cch` characters in the `CONTINUE` records
/// that follow it, each starting with its own option flags.
fn txo_text(s: &[u8], r: &Rec, cch: usize) -> String {
    let mut out = String::new();
    let mut left = cch;
    for &(at, len) in &r.cont {
        if left == 0 {
            break;
        }
        let d = s.get(at..at.saturating_add(len)).unwrap_or_default();
        let wide = d.first().is_some_and(|f| f & 1 != 0);
        let have = d.len().saturating_sub(1) / if wide { 2 } else { 1 };
        let n = have.min(left);
        out.push_str(&unicode(d, 0, n));
        left -= n;
    }
    out
}

/// An IEEE double of a record.
fn f64_at(d: &[u8], i: usize) -> Option<f64> {
    let b = d.get(i..i.checked_add(8)?)?;
    Some(f64::from_le_bytes(b.try_into().ok()?))
}

/// A margin in inches → 1/100 mm; `None` for a value that is not a
/// length (NaN, ±∞). Negative margins are 0, huge ones 50 in.
fn inch_to_hmm(v: f64) -> Option<u32> {
    v.is_finite()
        .then(|| (v.max(0.0) * 2540.0).min(MAX_MARGIN_HMM).round() as u32)
}

const fn inch(v: f64) -> u32 {
    (v * 2540.0 + 0.5) as u32
}

/// Paper sizes of the `SETUP` paper codes the model has no name for
/// (code, width, height in 1/100 mm; OpenOffice's `pPaperSizeTable`,
/// xlpage.cxx).
const PAPER_SIZES: [(u16, u32, u32); 58] = [
    (4, inch(17.0), inch(11.0)),
    (6, inch(5.5), inch(8.5)),
    (12, 25_700, 36_400),
    (14, inch(8.5), inch(13.0)),
    (15, 21_500, 27_500),
    (16, inch(10.0), inch(14.0)),
    (17, inch(11.0), inch(17.0)),
    (18, inch(8.5), inch(11.0)),
    (19, inch(3.875), inch(8.875)),
    (20, inch(4.125), inch(9.5)),
    (21, inch(4.5), inch(10.375)),
    (22, inch(4.75), inch(11.0)),
    (23, inch(5.0), inch(11.5)),
    (24, inch(17.0), inch(22.0)),
    (25, inch(22.0), inch(34.0)),
    (26, inch(34.0), inch(44.0)),
    (27, 11_000, 22_000),
    (28, 16_200, 22_900),
    (29, 32_400, 45_800),
    (30, 22_900, 32_400),
    (31, 11_400, 16_200),
    (32, 11_400, 22_900),
    (33, 25_000, 35_300),
    (35, 12_500, 17_600),
    (36, 11_000, 23_000),
    (37, inch(3.875), inch(7.5)),
    (38, inch(3.625), inch(6.5)),
    (39, inch(14.875), inch(11.0)),
    (40, inch(8.5), inch(12.0)),
    (41, inch(8.5), inch(13.0)),
    (42, 25_000, 35_300),
    (43, 10_000, 14_800),
    (44, inch(9.0), inch(11.0)),
    (45, inch(10.0), inch(11.0)),
    (46, inch(15.0), inch(11.0)),
    (47, 22_000, 22_000),
    (50, inch(9.5), inch(12.0)),
    (51, inch(9.5), inch(15.0)),
    (52, inch(11.69), inch(18.0)),
    (53, 23_500, 32_200),
    (54, inch(8.5), inch(11.0)),
    (55, 21_000, 29_700),
    (56, inch(9.5), inch(12.0)),
    (57, 22_700, 35_600),
    (58, 30_500, 48_700),
    (59, inch(8.5), inch(12.69)),
    (60, 21_000, 33_000),
    (61, 14_800, 21_000),
    (62, 18_200, 25_700),
    (63, 32_200, 44_500),
    (64, 17_400, 23_500),
    (65, 20_100, 27_600),
    (66, 42_000, 59_400),
    (67, 29_700, 42_000),
    (68, 32_200, 44_500),
    (69, 20_000, 14_800),
    (70, 10_500, 14_800),
    (88, 12_800, 18_200),
];

/// The paper of a `SETUP` paper code; `None` for "undefined" (the
/// default paper).
fn paper_of(code: u16) -> Option<PaperData> {
    // B5 (JIS), 182 × 257 mm, is not the model's B5 (ISO).
    if code == 13 {
        return Some(PaperData::from_size_hmm(18_200, 25_700));
    }
    if let Some(p) = tally_io_xlsx::paper_from_excel_code(u32::from(code)) {
        return Some(p);
    }
    // The rotated papers (75–83, 89, 90) are their upright size: the
    // orientation flag says how the sheet is printed.
    let code = match code {
        75 => return Some(PaperData::Letter),
        76 => return Some(PaperData::A3),
        77 => return Some(PaperData::A4),
        78 => return Some(PaperData::A5),
        79 => 12,
        80 => return Some(PaperData::from_size_hmm(18_200, 25_700)),
        81 => 43,
        82 => 69,
        83 => 70,
        89 => 88,
        90 => return Some(PaperData::from_size_hmm(inch(11.0), inch(12.0))),
        other => other,
    };
    PAPER_SIZES
        .iter()
        .find(|p| p.0 == code)
        .map(|p| PaperData::from_size_hmm(p.1, p.2))
}

/// The page setup records of a sheet.
#[derive(Default)]
struct PageRecs {
    parts: tally_io_xlsx::PageParts,
    left: Option<u32>,
    right: Option<u32>,
    top: Option<u32>,
    bottom: Option<u32>,
    header: Option<u32>,
    footer: Option<u32>,
}

impl PageRecs {
    /// The model's page setup; the margins a file does not give are
    /// Excel's defaults (0.75 in at the sides, 1 in at top and bottom,
    /// 0.5 in for header and footer; `XclPageData::SetDefaults`).
    fn build(mut self) -> Option<PageSetupData> {
        if !self.parts.any {
            return None;
        }
        self.parts.margins = Some(MarginsData {
            left: self.left.unwrap_or(1900),
            right: self.right.unwrap_or(1900),
            top: self.top.unwrap_or(2500),
            bottom: self.bottom.unwrap_or(2500),
            header: self.header.unwrap_or(1300),
            footer: self.footer.unwrap_or(1300),
        });
        self.parts.build()
    }
}

impl Reader<'_> {
    fn row_has_xf(&self, row: u32) -> bool {
        self.row_xf
            .get((row / 64) as usize)
            .is_some_and(|w| w & (1u64 << (row % 64)) != 0)
    }

    /// Notes the format of a cell record. Kept are the cells whose format
    /// shows, and every cell of a column with a style or of a row with a
    /// default format (`COLINFO` and `ROW` come before the cells they
    /// apply to): such a cell must not take the column's or the row's
    /// format.
    fn cell(
        &mut self,
        out: &mut Vec<CellFmt>,
        cols: &[Option<ColDef>],
        (row, col): (u32, u32),
        xf: u16,
        value: bool,
    ) {
        let Some(def) = cols.get(col as usize) else {
            return;
        };
        let info = self.xfs.get(usize::from(xf));
        // A value cell needs its style; an empty one only if the format
        // shows.
        let shows = info.is_some_and(|x| x.style.is_some() && (value || !x.plain));
        let covered = def.is_some_and(|d| d.style.is_some()) || self.row_has_xf(row);
        if !shows && !covered {
            return;
        }
        if self.cells >= self.limits.max_styled_cells {
            self.dropped.cells = true;
            return;
        }
        self.cells += 1;
        out.push(CellFmt {
            row,
            col: col as u16,
            xf,
        });
    }

    /// Puts a hyperlink on its cells: one entry per cell, sharing the
    /// target. A link over more than [`MAX_LINK_CELLS`] cells is left out;
    /// the cells and the target bytes count against the workbook's limits.
    fn link(&mut self, out: &mut Vec<LinkData>, l: hlink::Hlink) {
        let cells = u64::from(l.r1 - l.r0 + 1) * u64::from(l.c1 - l.c0 + 1);
        if cells > MAX_LINK_CELLS {
            self.dropped.bad_links += 1;
            return;
        }
        let bytes = self.link_bytes.saturating_add(l.url.len() as u64);
        if self.links >= self.limits.max_links || bytes > self.limits.max_link_text_bytes {
            self.dropped.links = true;
            return;
        }
        self.link_bytes = bytes;
        let url: Arc<str> = Arc::from(l.url);
        for row in l.r0..=l.r1 {
            for col in l.c0..=l.c1 {
                if self.links >= self.limits.max_links {
                    self.dropped.links = true;
                    return;
                }
                self.links += 1;
                out.push(LinkData {
                    row,
                    col,
                    url: url.clone(),
                });
            }
        }
    }

    fn sheet(&mut self, start: usize, walked: &mut u64, walk_budget: u64) -> SheetFormat {
        let s = self.s;
        let mut out = SheetFormat::default();
        let mut cols: Vec<Option<ColDef>> = vec![None; BIFF_COLS];
        let mut tail: Option<ColDef> = None;
        let mut col_infos = 0usize;
        let mut def_col_chars: Option<u16> = None;
        let mut standard_width: Option<u16> = None;
        // DEFAULTROWHEIGHT: flags, height.
        let mut def_row: Option<(u16, u16)> = None;
        let mut rows: Vec<RowRec> = Vec::new();
        // The last column with a cell record or a definition.
        let mut last_col: Option<u32> = None;
        let mut window2: Option<u16> = None;
        let mut pane: Option<(u16, u16)> = None;
        let mut page = PageRecs::default();
        let mut drawing: Vec<u8> = Vec::new();
        // Notes: the object records before a TXO name the object the text
        // belongs to; NOTE records point at the object.
        let mut last_obj: Option<(u16, u16)> = None;
        let mut note_texts: HashMap<u16, String> = HashMap::new();
        let mut note_recs: Vec<NoteRec> = Vec::new();
        let mut link_buf: Vec<u8> = Vec::new();
        let mut seen_bof = false;
        let mut depth = 0usize;
        let mut pos = start;
        self.row_xf.iter_mut().for_each(|w| *w = 0);
        while let Some(Ok(r)) = record(s, pos) {
            *walked += (r.next - pos) as u64;
            if *walked > walk_budget {
                self.dropped.walk = true;
                break;
            }
            pos = r.next;
            if depth > 0 {
                // An embedded substream (a chart).
                match r.typ {
                    0x0809 => depth += 1,
                    0x000A => depth -= 1,
                    _ => {}
                }
                continue;
            }
            let d = body(s, &r);
            let w = |i: usize| u16_at(d, i);
            match r.typ {
                0x0809 => {
                    if seen_bof {
                        depth += 1;
                    } else {
                        seen_bof = true;
                    }
                }
                0x000A => break,
                // COLINFO
                0x007D => {
                    let (Some(first), Some(last), Some(width), Some(xf)) = (w(0), w(2), w(4), w(6))
                    else {
                        continue;
                    };
                    let flags = w(8).unwrap_or(0);
                    if first > last || usize::from(first) >= BIFF_COLS {
                        continue;
                    }
                    col_infos += 1;
                    if col_infos > self.limits.max_col_infos {
                        self.dropped.col_infos = true;
                        continue;
                    }
                    let def = ColDef {
                        width,
                        hidden: flags & 1 != 0,
                        style: self
                            .xfs
                            .get(usize::from(xf))
                            .filter(|x| !x.plain)
                            .and_then(|x| x.style),
                    };
                    let end = usize::from(last).min(BIFF_COLS - 1);
                    for slot in cols.iter_mut().take(end + 1).skip(usize::from(first)) {
                        *slot = Some(def);
                    }
                    last_col = last_col.max(Some(end as u32));
                    // A range that reaches the last BIFF column runs to the
                    // end of the sheet.
                    if usize::from(last) >= BIFF_COLS - 1 {
                        tail = Some(def);
                    }
                }
                0x0055 => def_col_chars = w(0).or(def_col_chars),
                0x0099 => standard_width = w(0).or(standard_width),
                0x0225 => {
                    if let (Some(flags), Some(h)) = (w(0), w(2)) {
                        def_row = Some((flags, h));
                    }
                }
                // ROW
                0x0208 => {
                    let (Some(row), Some(h), Some(flags)) = (w(0), w(6), u32_at(d, 12)) else {
                        continue;
                    };
                    if self.rows >= self.limits.max_row_records {
                        self.dropped.rows = true;
                        continue;
                    }
                    self.rows += 1;
                    let rec = RowRec {
                        row: u32::from(row),
                        twips: h,
                        flags,
                    };
                    if let Some(word) = self.row_xf.get_mut(usize::from(row / 64)) {
                        // Of several records for one row the last counts.
                        let bit = 1u64 << (row % 64);
                        if rec.default_xf().is_some() {
                            *word |= bit;
                        } else {
                            *word &= !bit;
                        }
                    }
                    rows.push(rec);
                }
                // NUMBER, RK, LABELSST, LABEL, RSTRING, FORMULA, BOOLERR.
                0x0203 | 0x027E | 0x00FD | 0x0204 | 0x00D6 | 0x0006 | 0x0205 => {
                    if let (Some(row), Some(col), Some(xf)) = (w(0), w(2), w(4)) {
                        let at = (u32::from(row), u32::from(col));
                        last_col = last_col.max(Some(at.1));
                        self.cell(&mut out.cells, &cols, at, xf, true);
                    }
                }
                // BLANK
                0x0201 => {
                    if let (Some(row), Some(col), Some(xf)) = (w(0), w(2), w(4)) {
                        let at = (u32::from(row), u32::from(col));
                        last_col = last_col.max(Some(at.1));
                        self.cell(&mut out.cells, &cols, at, xf, false);
                    }
                }
                // MULRK: row, first column, (XF, RK)…, last column.
                0x00BD => {
                    if let (Some(row), Some(col)) = (w(0), w(2)) {
                        for k in 0..d.len().saturating_sub(6) / 6 {
                            if let Some(xf) = w(4 + 6 * k) {
                                let at = (u32::from(row), u32::from(col) + k as u32);
                                last_col = last_col.max(Some(at.1));
                                self.cell(&mut out.cells, &cols, at, xf, true);
                            }
                        }
                    }
                }
                // MULBLANK: row, first column, XF…, last column.
                0x00BE => {
                    if let (Some(row), Some(col)) = (w(0), w(2)) {
                        for k in 0..d.len().saturating_sub(6) / 2 {
                            if let Some(xf) = w(4 + 2 * k) {
                                let at = (u32::from(row), u32::from(col) + k as u32);
                                last_col = last_col.max(Some(at.1));
                                self.cell(&mut out.cells, &cols, at, xf, false);
                            }
                        }
                    }
                }
                0x023E => window2 = w(0).or(window2),
                0x0041 => {
                    if let (Some(x), Some(y)) = (w(0), w(2)) {
                        pane = Some((x, y));
                    }
                }
                // SCL: the zoom as a fraction.
                0x00A0 => {
                    if let (Some(num), Some(den)) = (w(0), w(2)) {
                        if den > 0 {
                            out.zoom =
                                tally_model::zoom_from_file(i64::from(num) * 100 / i64::from(den));
                        }
                    }
                }
                // SHEETEXT: a 12-byte header, the size, the tab colour: a
                // palette colour, as in OpenOffice
                // (`XclImpTabViewSettings::ReadTabBgColor`).
                0x0862 => {
                    if let Some(icv) = u32_at(d, 16) {
                        let icv = (icv & 0x7F) as u16;
                        if (8..=63).contains(&icv) {
                            out.tab_color = self.palette.rgb(icv).map(|c| (c << 8) | 0xff);
                        }
                    }
                }
                // SETUP: paper, scale, first page number, fit to pages
                // (wide, tall), flags, resolutions, header and footer
                // margins, copies.
                0x00A1 => {
                    let (Some(paper), Some(scale), Some(first), Some(fit_w), Some(fit_h), Some(f)) =
                        (w(0), w(2), w(4), w(6), w(8), w(10))
                    else {
                        continue;
                    };
                    let p = &mut page.parts;
                    p.any = true;
                    // Bit 2: paper, scale and orientation were never set;
                    // bit 6: the orientation was not.
                    let valid = f & 0x04 == 0;
                    p.paper = valid.then(|| paper_of(paper)).flatten();
                    p.scale = (valid && scale > 0).then_some(u32::from(scale));
                    p.orientation = (valid && f & 0x40 == 0).then_some(if f & 0x02 != 0 {
                        OrientationData::Portrait
                    } else {
                        OrientationData::Landscape
                    });
                    p.fit = Some((u32::from(fit_w), u32::from(fit_h)));
                    p.first_page = (f & 0x80 != 0 && (1..=0x7FFF).contains(&first))
                        .then_some(u32::from(first));
                    p.over_then_down = f & 0x01 != 0;
                    page.header = f64_at(d, 16).and_then(inch_to_hmm).or(page.header);
                    page.footer = f64_at(d, 24).and_then(inch_to_hmm).or(page.footer);
                }
                // LEFTMARGIN, RIGHTMARGIN, TOPMARGIN, BOTTOMMARGIN: inches.
                0x0026..=0x0029 => {
                    if let Some(m) = f64_at(d, 0).and_then(inch_to_hmm) {
                        page.parts.any = true;
                        match r.typ {
                            0x0026 => page.left = Some(m),
                            0x0027 => page.right = Some(m),
                            0x0028 => page.top = Some(m),
                            _ => page.bottom = Some(m),
                        }
                    }
                }
                // HEADER, FOOTER: a string with Excel's `&` codes; an empty
                // record is "none".
                0x0014 | 0x0015 => {
                    page.parts.any = true;
                    let text = w(0).map_or_else(String::new, |cch| unicode(d, 2, usize::from(cch)));
                    let hf = tally_io_xlsx::header_footer_from_excel(&text);
                    if r.typ == 0x0014 {
                        page.parts.header = Some(hf);
                    } else {
                        page.parts.footer = Some(hf);
                    }
                }
                // PRINTHEADERS, PRINTGRIDLINES, HCENTER, VCENTER.
                0x002A | 0x002B | 0x0083 | 0x0084 => {
                    if let Some(v) = w(0) {
                        let p = &mut page.parts;
                        p.any = true;
                        match r.typ {
                            0x002A => p.headings = v != 0,
                            0x002B => p.gridlines = v != 0,
                            0x0083 => p.center_h = v != 0,
                            _ => p.center_v = v != 0,
                        }
                    }
                }
                // WSBOOL: bit 8 = fit to pages.
                0x0081 => {
                    if let Some(f) = w(0) {
                        page.parts.fit_to_page = f & 0x0100 != 0;
                        page.parts.any |= page.parts.fit_to_page;
                    }
                }
                // VERTICALPAGEBREAKS, HORIZONTALPAGEBREAKS: a count, then
                // (the column or row after the break, the first and the
                // last row or column it spans) each. A later record
                // replaces an earlier one.
                0x001A | 0x001B => {
                    let (breaks, end) = if r.typ == 0x001B {
                        (&mut out.row_breaks, BIFF_ROWS)
                    } else {
                        (&mut out.col_breaks, BIFF_COLS as u32)
                    };
                    breaks.clear();
                    let n = usize::from(w(0).unwrap_or(0)).min(d.len().saturating_sub(2) / 6);
                    for k in 0..n {
                        let Some(b) = w(2 + 6 * k).map(u32::from) else {
                            break;
                        };
                        if b == 0 || b >= end {
                            continue;
                        }
                        if breaks.len() >= self.limits.max_page_breaks {
                            self.dropped.page_breaks = true;
                            break;
                        }
                        breaks.push(b);
                    }
                }
                // MSODRAWING: the sheet's OfficeArt data, in pieces.
                0x00EC => {
                    let left = self
                        .limits
                        .max_drawing_bytes
                        .saturating_sub(self.drawing_bytes);
                    let before = drawing.len();
                    let cap = before.saturating_add(left);
                    if !append(s, &r, &mut drawing, cap) {
                        self.dropped.drawing_bytes = true;
                    }
                    self.drawing_bytes += drawing.len() - before;
                }
                // OBJ: the common object data (type, id) comes first.
                0x005D => {
                    last_obj = match (w(0), w(4), w(6)) {
                        (Some(0x15), Some(ot), Some(id)) => Some((id, ot)),
                        _ => None,
                    };
                }
                // TXO: the text of the object before it; 0x19 is a note.
                0x01B6 => {
                    if let (Some((id, 0x19)), Some(cch)) = (last_obj.take(), w(10)) {
                        if self.notes < self.limits.max_notes {
                            self.notes += 1;
                            note_texts.insert(id, txo_text(s, &r, usize::from(cch)));
                        } else {
                            self.dropped.notes = true;
                        }
                    }
                }
                // NOTE: row, column, flags (bit 1: always shown), object
                // id, author.
                0x001C => {
                    if let (Some(row), Some(col), Some(flags), Some(obj)) = (w(0), w(2), w(4), w(6))
                    {
                        if note_recs.len() < self.limits.max_notes {
                            let author = w(8)
                                .map(|cch| unicode(d, 10, usize::from(cch)))
                                .map(|a| a.trim_end_matches('\0').to_string())
                                .filter(|a| !a.is_empty());
                            note_recs.push(NoteRec {
                                row: u32::from(row),
                                col: u32::from(col),
                                shown: flags & 2 != 0,
                                obj,
                                author,
                            });
                        } else {
                            self.dropped.notes = true;
                        }
                    }
                }
                // HLINK: the cells and the target.
                0x01B8 => {
                    let link = if r.cont.is_empty() {
                        hlink::parse(d)
                    } else {
                        link_buf.clear();
                        append(s, &r, &mut link_buf, hlink::MAX_RECORD_BYTES)
                            .then(|| hlink::parse(&link_buf))
                            .flatten()
                    };
                    match link {
                        Some(l) => self.link(&mut out.links, l),
                        None => self.dropped.bad_links += 1,
                    }
                }
                _ => {}
            }
        }

        // Hyperlinks: row-major, one per cell (the first record wins, as
        // in the XLSX reader).
        if !out.links.is_sorted_by_key(|l| (l.row, l.col)) {
            out.links.sort_by_key(|l| (l.row, l.col));
        }
        out.links.dedup_by_key(|l| (l.row, l.col));

        // Columns: OpenOffice's conversion (`XclTools::GetScColumnWidth`),
        // a width in 1/256 of the character width of font 0, rounded to
        // twips. Every column gets its width: the ones without a `COLINFO`
        // the sheet's default.
        let char_width = self.char_width as f64;
        let sc_width =
            |w: u16| -> u16 { (f64::from(w) / 256.0 * char_width + 0.5).min(65_535.0) as u16 };
        let default_twips = match (standard_width, def_col_chars) {
            // STANDARDWIDTH, in 1/256 of a character, wins over
            // DEFCOLWIDTH.
            (Some(w), _) if w > 0 => sc_width(w),
            // DEFCOLWIDTH counts whole characters, and Excel adds room
            // that depends on the font size
            // (`XclTools::GetXclDefColWidthCorrection`).
            (_, Some(ch)) if ch > 0 => {
                let correction = 40_960.0 / (self.font_twips - 15).max(60) as f64 + 50.0;
                sc_width((256.0 * f64::from(ch) + correction).min(65_535.0) as u16)
            }
            _ => 0,
        };
        let default_col = twips_to_hmm(if default_twips == 0 {
            STD_COL_WIDTH_TWIPS
        } else {
            default_twips
        });
        // (width, hidden); a width of 0 is a hidden column of the default
        // width.
        let size_of = |def: Option<ColDef>| -> (u32, bool) {
            match def.map(|c| (sc_width(c.width), c.hidden)) {
                Some((0, _)) => (default_col, true),
                Some((twips, hidden)) => (twips_to_hmm(twips), hidden),
                None => (default_col, false),
            }
        };
        out.styled_cols = cols
            .iter()
            .map(|d| d.is_some_and(|d| d.style.is_some()))
            .collect();
        for (c, def) in cols.iter().enumerate() {
            let (size, hidden) = size_of(*def);
            push_run(&mut out.col_widths, c as u32, c as u32, size, hidden);
            if let Some(style) = def.and_then(|d| d.style) {
                out.style_runs.push(StyleRun {
                    col: c as u32,
                    start_row: 0,
                    end_row: MAX_ROWS - 1,
                    style,
                });
            }
        }
        let (size, hidden) = size_of(tail);
        push_run(
            &mut out.col_widths,
            BIFF_COLS as u32,
            MAX_COLS - 1,
            size,
            hidden,
        );

        // Rows, as OpenOffice lays them out
        // (`XclImpColRowSettings::Convert`): every `ROW` record gives its
        // row a height (the "unsynced" flag only says whether the user set
        // it), the other rows have the sheet's default height, and a
        // default that is hidden hides them. The model has no "automatic
        // height of this size", so every row is given its height.
        let (def_flags, def_twips) = match def_row {
            // A default height of 0 is "hidden".
            Some((flags, 0)) => (flags | 0x02, STD_ROW_HEIGHT_TWIPS),
            Some(d) => d,
            None => (0, STD_ROW_HEIGHT_TWIPS),
        };
        let default_row = twips_to_hmm(def_twips);
        let default_hidden = def_flags & 0x02 != 0;
        if !rows.is_sorted_by_key(|r| r.row) {
            rows.sort_by_key(|r| r.row);
        }
        let mut row_sizes: Vec<(u32, i64)> = Vec::with_capacity(rows.len());
        // (row, XF) of the rows with a default format.
        let mut row_xfs: Vec<(u32, u16)> = Vec::new();
        let mut next = 0u32;
        for (i, r) in rows.iter().enumerate() {
            // Of several records for one row the last one counts.
            if rows.get(i + 1).is_some_and(|n| n.row == r.row) {
                continue;
            }
            if let Some(xf) = r.default_xf() {
                row_xfs.push((r.row, xf));
            }
            let twips = r.twips & 0x7FFF;
            // The default height when the record says so.
            let (size, mut hidden) = if r.twips & 0x8000 != 0 {
                (default_row, default_hidden)
            } else if twips == 0 {
                // A height of 0 is a hidden row.
                (default_row, true)
            } else {
                (twips_to_hmm(twips), false)
            };
            hidden |= r.flags & 0x20 != 0;
            if r.row > next {
                push_run(
                    &mut out.row_heights,
                    next,
                    r.row - 1,
                    default_row,
                    default_hidden,
                );
            }
            push_run(&mut out.row_heights, r.row, r.row, size, hidden);
            next = r.row + 1;
            row_sizes.push((r.row, if hidden { 0 } else { i64::from(size) }));
        }
        if next < MAX_ROWS {
            push_run(
                &mut out.row_heights,
                next,
                MAX_ROWS - 1,
                default_row,
                default_hidden,
            );
        }

        // Cells: row-major, one per position (the last record wins).
        if !out.cells.is_sorted_by_key(|c| (c.row, c.col)) {
            out.cells.sort_by_key(|c| (c.row, c.col));
        }
        let mut kept = 0usize;
        for i in 0..out.cells.len() {
            let c = out.cells[i];
            if out
                .cells
                .get(i + 1)
                .is_some_and(|n| (n.row, n.col) == (c.row, c.col))
            {
                continue;
            }
            out.cells[kept] = c;
            kept += 1;
        }
        out.cells.truncate(kept);
        self.row_formats(&mut out, &row_xfs, last_col);

        // Page setup and manual breaks.
        out.page_setup = page.build();
        for breaks in [&mut out.row_breaks, &mut out.col_breaks] {
            breaks.sort_unstable();
            breaks.dedup();
        }

        // View.
        if let Some(flags) = window2 {
            let v = ViewOptionsData {
                show_formulas: flags & 0x01 != 0,
                value_highlighting: false,
                show_zero_values: flags & 0x10 != 0,
                show_gridlines: flags & 0x02 != 0,
                show_headers: flags & 0x04 != 0,
            };
            if v != ViewOptionsData::default() {
                out.view_options = Some(v);
            }
            if let Some((x, y)) = pane {
                if flags & 0x08 != 0 {
                    // Frozen: columns and rows. (OpenOffice freezes at
                    // the first visible cell plus these counts; the
                    // model's frozen panes start at A1, so a sheet frozen
                    // while scrolled keeps its counts.)
                    let cols = u32::from(x).min(MAX_COLS - 1);
                    let rows = u32::from(y).min(MAX_ROWS - 1);
                    if rows > 0 || cols > 0 {
                        out.frozen = Some((rows, cols));
                    }
                } else {
                    // Window ▸ Split: positions in twips.
                    let split = SplitData {
                        x_hmm: twips_to_hmm(x),
                        y_hmm: twips_to_hmm(y),
                    };
                    if split.x_hmm > 0 || split.y_hmm > 0 {
                        out.split = Some(split);
                    }
                }
            }
        }

        // Notes: the ones whose text was found, one per cell.
        note_recs.sort_by_key(|n| (n.row, n.col));
        for n in note_recs {
            let Some(text) = note_texts.remove(&n.obj) else {
                continue;
            };
            if out
                .notes
                .last()
                .is_some_and(|l| (l.row, l.col) == (n.row, n.col))
            {
                continue;
            }
            out.notes.push(NoteData {
                row: n.row,
                col: n.col,
                // Line ends as `\n`; NULs (a length that ran into the
                // formatting runs) are dropped.
                text: text
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\0', ""),
                author: n.author,
                created: None,
                shown: n.shown,
            });
        }
        let note_shapes = out.notes.len();

        // Pictures, placed with the sizes the columns and rows are shown
        // at.
        if !drawing.is_empty() {
            let shown = |def: Option<ColDef>| match size_of(def) {
                (_, true) => 0,
                (size, false) => i64::from(size),
            };
            let col_sizes: Vec<(u32, i64)> = cols
                .iter()
                .enumerate()
                .map(|(c, def)| (c as u32, shown(*def)))
                .collect();
            let col_axis = Axis::new(col_sizes, shown(tail));
            let row_default = if default_hidden {
                0
            } else {
                i64::from(default_row)
            };
            let row_axis = Axis::new(row_sizes, row_default);
            self.pictures(&drawing, &col_axis, &row_axis, note_shapes, &mut out.images);
        }
        out
    }

    /// Adds the empty cells the rows with a default format (`row_xfs`,
    /// sorted) give their format to: the positions without a cell record
    /// from column A to the last column the sheet uses (`last_col`). A
    /// default format that is the plain look only needs cells where a
    /// column style would show instead. Linear in the cells there are
    /// and the cells added, which count against the styled-cell budget.
    fn row_formats(
        &mut self,
        out: &mut SheetFormat,
        row_xfs: &[(u32, u16)],
        last_col: Option<u32>,
    ) {
        let Some(last_col) = last_col.map(|c| c.min(BIFF_COLS as u32 - 1)) else {
            return;
        };
        if row_xfs.is_empty() {
            return;
        }
        let all: Vec<u32> = (0..=last_col).collect();
        let styled: Vec<u32> = all
            .iter()
            .copied()
            .filter(|c| out.styled_cols.get(*c as usize) == Some(&true))
            .collect();
        let mut merged: Vec<CellFmt> = Vec::with_capacity(out.cells.len());
        let mut cells = std::mem::take(&mut out.cells).into_iter().peekable();
        for &(row, xf) in row_xfs {
            merged.extend(std::iter::from_fn(|| cells.next_if(|c| c.row < row)));
            let shows = self.xfs.get(usize::from(xf)).is_some_and(|x| !x.plain);
            for &col in if shows { &all } else { &styled } {
                merged.extend(std::iter::from_fn(|| {
                    cells.next_if(|c| c.row == row && u32::from(c.col) < col)
                }));
                if let Some(c) = cells.next_if(|c| c.row == row && u32::from(c.col) == col) {
                    merged.push(c);
                    continue;
                }
                if self.cells >= self.limits.max_styled_cells {
                    self.dropped.cells = true;
                    break;
                }
                self.cells += 1;
                merged.push(CellFmt {
                    row,
                    col: col as u16,
                    xf,
                });
            }
            if self.dropped.cells {
                break;
            }
        }
        merged.extend(cells);
        out.cells = merged;
    }

    fn pictures(
        &mut self,
        drawing: &[u8],
        cols: &Axis,
        rows: &Axis,
        note_shapes: usize,
        out: &mut Vec<ImageData>,
    ) {
        let mut shapes: Vec<Shape> = Vec::new();
        let mut current: Option<Shape> = None;
        let mut others = 0usize;
        let mut capped = false;
        let cap = self.limits.max_images;
        let mut flush = |shape: Option<Shape>, shapes: &mut Vec<Shape>| {
            let Some(shape) = shape else { return };
            match (shape.pib, shape.anchor) {
                (Some(_), Some(_)) if shapes.len() < cap => shapes.push(shape),
                (Some(_), Some(_)) => capped = true,
                // An anchored shape without a picture: a text box, a
                // line, a control… A picture without a sheet anchor is
                // part of a group shape (its place is relative to the
                // group), which is not imported.
                (None, Some(_)) | (Some(_), None) => others += 1,
                // The group's own shape and other bookkeeping.
                _ => {}
            }
        };
        art_scan(drawing, |r| match r.typ {
            0xF004 => {
                flush(current.take(), &mut shapes);
                current = Some(Shape::default());
            }
            // OPT: property id (14 bits, bit 14 = "is a BLIP id"), value.
            0xF00B => {
                if let Some(shape) = current.as_mut() {
                    let n = usize::from(r.inst).min(r.body.len() / 6);
                    for k in 0..n {
                        let id = u16_at(r.body, 6 * k).unwrap_or(0);
                        if id & 0x3FFF == 0x0104 && id & 0x4000 != 0 {
                            shape.pib = u32_at(r.body, 6 * k + 2);
                        }
                    }
                }
            }
            // ClientAnchor: flags, then column / offset / row / offset of
            // the two corners.
            0xF010 => {
                if let Some(shape) = current.as_mut() {
                    let mut a = [0u16; 8];
                    let mut ok = true;
                    for (k, slot) in a.iter_mut().enumerate() {
                        match u16_at(r.body, 2 + 2 * k) {
                            Some(v) => *slot = v,
                            None => ok = false,
                        }
                    }
                    if ok {
                        shape.anchor = Some(a);
                    }
                }
            }
            _ => {}
        });
        flush(current.take(), &mut shapes);
        self.dropped.images |= capped;
        // The boxes of the notes that were read are not "left out".
        self.dropped.other_shapes += others.saturating_sub(note_shapes);
        for shape in shapes {
            let (Some(pib), Some(a)) = (shape.pib, shape.anchor) else {
                continue;
            };
            let blip = (pib as usize)
                .checked_sub(1)
                .and_then(|i| self.blips.get(i));
            let (mime, bytes) = match blip {
                Some(Blip::Picture(mime, bytes)) => (*mime, bytes.clone()),
                Some(Blip::Unsupported) => {
                    self.dropped.unsupported_pictures += 1;
                    continue;
                }
                Some(Blip::Missing) | None => {
                    self.dropped.other_shapes += 1;
                    continue;
                }
            };
            // Every reference counts against the document total.
            if self.images >= self.limits.max_images
                || self.image_bytes + bytes.len() as u64 > self.limits.max_total_image_bytes
            {
                self.dropped.images = true;
                continue;
            }
            self.images += 1;
            self.image_bytes += bytes.len() as u64;
            out.push(ImageData {
                name: format!("Image {}", out.len() + 1),
                anchor: anchor(a, cols, rows),
                mime: mime.to_string(),
                bytes,
            });
        }
    }
}

/// A client anchor (column, 1/1024 of its width, row, 1/256 of its height,
/// for the top-left and the bottom-right corner) → the model's anchor.
fn anchor(a: [u16; 8], cols: &Axis, rows: &Axis) -> ChartAnchorData {
    let [c1, dx1, r1, dy1, c2, dx2, r2, dy2] = a.map(u32::from);
    let (c1, c2) = (c1.min(BIFF_COLS as u32), c2.min(BIFF_COLS as u32));
    let part = |size: i64, frac: u32, of: i64| size * i64::from(frac).min(of) / of;
    let x1 = cols.before(c1) + part(cols.size(c1), dx1, 1024);
    let x2 = cols.before(c2) + part(cols.size(c2), dx2, 1024);
    let y1 = rows.before(r1) + part(rows.size(r1), dy1, 256);
    let y2 = rows.before(r2) + part(rows.size(r2), dy2, 256);
    let clamp = |v: i64| v.clamp(0, i64::from(u32::MAX)) as u32;
    ChartAnchorData {
        row: r1,
        col: c1.min(MAX_COLS - 1),
        dx_hmm: clamp(x1 - cols.before(c1)),
        dy_hmm: clamp(y1 - rows.before(r1)),
        width_hmm: clamp(x2 - x1).clamp(100, 1_000_000),
        height_hmm: clamp(y2 - y1).clamp(100, 1_000_000),
    }
}

fn twips_to_hmm(twips: u16) -> u32 {
    tally_io_xlsx::pt_to_hmm(f64::from(twips) / 20.0)
}

/// Appends a run, merging with the previous one when contiguous and equal.
fn push_run(runs: &mut Vec<DimRun>, start: u32, end: u32, size_hmm: u32, hidden: bool) {
    if let Some(last) = runs.last_mut() {
        if last.end.checked_add(1) == Some(start)
            && last.size_hmm == size_hmm
            && last.hidden == hidden
        {
            last.end = end;
            return;
        }
    }
    runs.push(DimRun {
        start,
        end,
        size_hmm,
        hidden,
    });
}

/// The warning of a workbook whose formatting was not read.
fn values_only(why: &str) -> Formatting {
    Formatting {
        warnings: vec![format!(
            "{why}: the workbook was opened with its values only (no column widths, row heights, \
             fonts, number formats or pictures)."
        )],
        ..Formatting::default()
    }
}

/// What [`read`] is replaced by when it did not finish (a panic caught
/// by the caller's guard: a bug, never expected).
pub(crate) fn failed() -> Formatting {
    values_only("The formatting of this file could not be read")
}

/// The formatting of a workbook whose sheets do not pair up with
/// calamine's.
pub(crate) fn unattributable() -> Formatting {
    values_only("The formatting of this file does not match its sheets")
}

/// Reads the formatting of the workbook stream `s`. Never fails: see the
/// module docs.
pub(crate) fn read(s: &[u8], limits: &Limits) -> Formatting {
    let mut fonts: Vec<RawFont> = Vec::new();
    let mut formats: HashMap<u16, String> = HashMap::new();
    let mut raw_xfs: Vec<RawXf> = Vec::new();
    let mut palette = Palette(tally_io_xlsx::INDEXED_PALETTE);
    let mut sheet_pos: Vec<usize> = Vec::new();
    let mut group: Vec<u8> = Vec::new();
    let mut active: Option<u16> = None;
    let mut prints: Vec<PrintName> = Vec::new();
    let mut names = NameRecs::default();
    let mut extern_buf: Vec<u8> = Vec::new();
    let mut format_bytes = 0u64;
    let mut dropped = Dropped::default();
    let group_cap = usize::try_from(limits.max_total_image_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(limits.max_drawing_bytes);

    let mut pos = 0usize;
    while let Some(Ok(r)) = record(s, pos) {
        pos = r.next;
        let d = body(s, &r);
        let w = |i: usize| u16_at(d, i);
        match r.typ {
            0x0809 => {
                let v = w(0).unwrap_or(0);
                let dt = if d.len() >= 4 { w(2).unwrap_or(0) } else { 0 };
                if bof_is_pre_biff8(v, dt) {
                    return values_only(
                        "This is an Excel 5.0/95 (or older) workbook, whose formatting Tally \
                         does not read",
                    );
                }
            }
            0x000A => break,
            // FONT: a truncated record still takes its index.
            0x0031 => {
                if fonts.len() >= limits.max_fonts {
                    dropped.fonts = true;
                    continue;
                }
                let name = d.get(14).and_then(|cch| {
                    tally_model::clean_font_name(&unicode(d, 15, usize::from(*cch)))
                });
                fonts.push(RawFont {
                    twips: w(0).unwrap_or(0),
                    grbit: w(2).unwrap_or(0),
                    icv: w(4).unwrap_or(0x7FFF),
                    bls: w(6).unwrap_or(400),
                    uls: d.get(10).copied().unwrap_or(0),
                    name,
                });
            }
            // FORMAT: id, then the code.
            0x041E => {
                let (Some(id), Some(cch)) = (w(0), w(2)) else {
                    continue;
                };
                if formats.len() >= limits.max_number_formats && !formats.contains_key(&id) {
                    dropped.formats = true;
                    continue;
                }
                let code = unicode(d, 4, usize::from(cch));
                // The codes share the text budget of the styles they end
                // up in.
                format_bytes = format_bytes.saturating_add(code.len() as u64);
                if format_bytes > limits.max_style_text_bytes {
                    dropped.style_text = true;
                    continue;
                }
                formats.insert(id, code);
            }
            // XF: a truncated record still takes its index (default look).
            0x00E0 => {
                if raw_xfs.len() >= limits.max_xfs {
                    dropped.xfs = true;
                    continue;
                }
                let x = if d.len() >= 20 {
                    RawXf {
                        ifnt: w(0).unwrap_or(0),
                        ifmt: w(2).unwrap_or(0),
                        flags: w(4).unwrap_or(1),
                        align: d.get(6).copied().unwrap_or(0x20),
                        rotation: d.get(7).copied().unwrap_or(0),
                        indent: d.get(8).copied().unwrap_or(0),
                        lines: w(10).unwrap_or(0),
                        colors1: w(12).unwrap_or(0),
                        colors2: u32_at(d, 14).unwrap_or(0),
                        fill: w(18).unwrap_or(0),
                    }
                } else {
                    RawXf {
                        flags: 1,
                        align: 0x20,
                        ..RawXf::default()
                    }
                };
                raw_xfs.push(x);
            }
            // PALETTE: colours 8….
            0x0092 => {
                let n = usize::from(w(0).unwrap_or(0)).min(56);
                for i in 0..n {
                    if let Some(c) = d.get(2 + 4 * i..5 + 4 * i) {
                        palette.0[8 + i] =
                            (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]);
                    }
                }
            }
            0x003D => active = w(10).or(active),
            // NAME: the print area and the print titles of a sheet; the
            // other names are defined names.
            0x0018 => {
                if let Some(p) = print_name(d) {
                    // One of each per sheet; they are applied in file
                    // order, so of several the last one counts.
                    if prints.len() < limits.max_sheets.saturating_mul(2) {
                        prints.push(p);
                    }
                }
                names.name(d, limits);
            }
            // SUPBOOK and EXTERNSHEET: what the sheet indices of 3-D
            // references in names stand for.
            0x01AE => names.supbook(d),
            // EXTERNNAME: the add-in functions formulas call by name.
            0x0023 => names.externname(d, limits),
            0x0017 => {
                extern_buf.clear();
                // 65,536 entries of 6 bytes at most.
                append(s, &r, &mut extern_buf, 2 + 6 * 65_536);
                names.externsheet(&extern_buf);
            }
            0x0085 => {
                if sheet_pos.len() < limits.max_sheets {
                    sheet_pos.push(u32_at(d, 0).unwrap_or(u32::MAX) as usize);
                }
            }
            // MSODRAWINGGROUP: the picture store, over several records.
            0x00EB => {
                if !append(s, &r, &mut group, group_cap) {
                    dropped.drawing_bytes = true;
                }
            }
            _ => {}
        }
    }

    // Styles.
    let default_family = fonts.first().and_then(|f| f.name.clone());
    let font_twips = i64::from(fonts.first().map_or(APP_FONT_TWIPS, |f| f.twips));
    let mut builder = StyleBuilder {
        fonts: &fonts,
        formats: &formats,
        palette: &palette,
        default_family: default_family.clone(),
        fmt_cache: HashMap::new(),
        long_format: false,
    };
    // The look of a cell that only has the default font.
    let base = StyleData {
        font: builder.font_data(0),
        ..StyleData::default()
    };
    // One model style per distinct look. XFs that are equal field by
    // field are resolved once.
    let mut table = StyleTable::default();
    let mut by_xf: HashMap<XfKey, XfInfo> = HashMap::new();
    let mut style_text = 0u64;
    let mut xfs: Vec<XfInfo> = Vec::with_capacity(raw_xfs.len());
    for x in &raw_xfs {
        if let Some(info) = by_xf.get(&x.key()) {
            xfs.push(*info);
            continue;
        }
        let mut style = builder.style(x);
        let mut idx = table.find(&style);
        if idx.is_none() && style != StyleData::default() {
            // Every style owns its number format and font name: their
            // total is bounded. Past the budget a new style keeps its look
            // without them (and may then be a look there already).
            let text = style.number_format.as_ref().map_or(0, String::len)
                + style.font.family.as_ref().map_or(0, String::len);
            if style_text.saturating_add(text as u64) > limits.max_style_text_bytes {
                dropped.style_text = true;
                style.number_format = None;
                style.font.family = None;
                idx = table.find(&style);
            } else {
                style_text += text as u64;
            }
            if idx.is_none() && style != StyleData::default() {
                idx = Some(table.push(style.clone()));
            }
        }
        let info = XfInfo {
            style: idx,
            date: style
                .number_format
                .as_deref()
                .is_some_and(tally_io_xlsx::is_date_format),
            has_format: style.number_format.is_some(),
            plain: idx.is_none() || style == base,
        };
        by_xf.insert(x.key(), info);
        xfs.push(info);
    }
    drop(by_xf);
    let styles = table.styles;
    dropped.long_format = builder.long_format;

    // The picture store.
    let mut blips: Vec<Blip> = Vec::new();
    art_scan(&group, |r| {
        if blips.len() >= MAX_BLIPS {
            return;
        }
        match r.typ {
            0xF007 => blips.push(blip_of(r.body, limits.max_image_bytes)),
            // A BLIP directly in the store (MS-ODRAW allows it instead of
            // a store entry) takes an index too.
            0xF018..=0xF117 => {
                blips.push(blip_record(r.inst, r.typ, r.body, limits.max_image_bytes));
            }
            _ => {}
        }
    });
    drop(group);

    let mut reader = Reader {
        s,
        limits,
        xfs,
        palette,
        font_twips,
        // The fallback of `XclRoot::SetCharWidth` (OpenOffice measures '0'
        // on its printer; the reader measures nothing): 11/20 of the font
        // height.
        char_width: (11 * font_twips / 20).max(1),
        blips,
        dropped,
        row_xf: vec![0u64; (BIFF_ROWS / 64) as usize],
        rows: 0,
        cells: 0,
        drawing_bytes: 0,
        images: 0,
        image_bytes: 0,
        notes: 0,
        links: 0,
        link_bytes: 0,
    };
    // Sheets, walked from each BOUNDSHEET offset like the pre-scan (the
    // same budget: sheets may share or overlap their substreams).
    let walk_budget = 2 * s.len() as u64 + (1 << 20);
    let mut walked = 0u64;
    let mut sheets: Vec<SheetFormat> = Vec::with_capacity(sheet_pos.len());
    for &start in &sheet_pos {
        if start > s.len() || reader.dropped.walk {
            sheets.push(SheetFormat::default());
            continue;
        }
        sheets.push(reader.sheet(start, &mut walked, walk_budget));
    }
    // Print areas and titles, as the XLSX reader places `_xlnm.Print_Area`
    // and `_xlnm.Print_Titles`.
    for p in prints {
        let Some(sheet) = sheets.get_mut(p.sheet) else {
            continue;
        };
        if p.titles {
            let (rows, cols) = p.titles();
            if rows.is_some() || cols.is_some() {
                let setup = sheet.page_setup.get_or_insert_with(PageSetupData::default);
                setup.repeat_rows = rows.or(setup.repeat_rows);
                setup.repeat_cols = cols.or(setup.repeat_cols);
            }
        } else if let [area] = p.areas.as_slice() {
            sheet
                .page_setup
                .get_or_insert_with(PageSetupData::default)
                .print_range = Some(*area);
        }
    }

    let d = &reader.dropped;
    let mut warnings: Vec<String> = Vec::new();
    if d.fonts || d.formats || d.xfs {
        warnings.push(format!(
            "Only the first {} fonts, {} number formats and {} cell formats were read.",
            limits.max_fonts, limits.max_number_formats, limits.max_xfs
        ));
    }
    if d.long_format {
        warnings.push(tally_model::long_number_format_warning());
    }
    if d.style_text {
        warnings.push(format!(
            "The number formats and font names of this file take more than {} MiB: some cell \
             formats were read without them.",
            limits.max_style_text_bytes / (1024 * 1024)
        ));
    }
    if d.page_breaks {
        warnings.push(format!(
            "Only the first {} manual page breaks of a sheet were read.",
            limits.max_page_breaks
        ));
    }
    if d.col_infos {
        warnings.push(format!(
            "Only the first {} column definitions of a sheet are read.",
            limits.max_col_infos
        ));
    }
    if d.rows {
        warnings.push(format!(
            "Only the first {} row definitions were read; later rows have the default height.",
            limits.max_row_records
        ));
    }
    if d.cells || d.walk {
        warnings.push(format!(
            "Only the formatting of the first {} cells was read.",
            limits.max_styled_cells
        ));
    }
    if d.images || d.drawing_bytes {
        warnings.push(format!(
            "Only the first {} pictures ({} MiB in all) were read.",
            limits.max_images,
            limits.max_total_image_bytes / (1024 * 1024)
        ));
    }
    if d.notes {
        warnings.push(format!(
            "Only the first {} cell notes were read.",
            limits.max_notes
        ));
    }
    if d.links {
        warnings.push(format!(
            "Only the first {} cells with a hyperlink were read.",
            limits.max_links
        ));
    }
    if d.bad_links > 0 {
        warnings.push(format!(
            "{} hyperlink(s) with a target Tally does not read, or over more than 10,000 cells, \
             were left out.",
            d.bad_links
        ));
    }
    if names.dropped {
        warnings.push(format!(
            "Only the first {} defined names ({} MiB) were read.",
            limits.max_names,
            limits.max_name_bytes / (1024 * 1024)
        ));
    }
    if d.unsupported_pictures > 0 {
        warnings.push(format!(
            "{} picture(s) in a format Tally does not show (EMF, WMF, PICT, BMP or TIFF) were \
             left out.",
            d.unsupported_pictures
        ));
    }
    if d.other_shapes > 0 {
        warnings.push(format!(
            "{} drawing object(s) (text boxes, shapes, controls or charts) were left out.",
            d.other_shapes
        ));
    }
    Formatting {
        styles,
        xfs: reader.xfs,
        default_font: default_family,
        active_sheet: active.map(u32::from),
        sheets,
        names,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built-in NAME record's data: `id` for sheet `itab` with `rgce`.
    fn name(id: u8, itab: u16, rgce: &[u8]) -> Vec<u8> {
        let mut d = vec![0x20, 0, 0, 1];
        d.extend_from_slice(&(rgce.len() as u16).to_le_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&itab.to_le_bytes());
        d.extend_from_slice(&[0, 0, 0, 0, 0, id]);
        d.extend_from_slice(rgce);
        d
    }

    fn area(r0: u16, r1: u16, c0: u16, c1: u16) -> Vec<u8> {
        let mut d = vec![0x3B, 0, 0];
        for w in [r0, r1, c0, c1] {
            d.extend_from_slice(&w.to_le_bytes());
        }
        d
    }

    #[test]
    fn print_names_cut_or_garbled_are_skipped() {
        let full = name(0x06, 1, &area(1, 4, 2, 3));
        let p = print_name(&full).unwrap();
        assert_eq!((p.sheet, p.titles, p.areas.len()), (0, false, 1));
        // Cut anywhere: no panic, and never a name with half an area.
        for cut in 0..full.len() {
            assert!(print_name(&full[..cut]).is_none(), "cut at {cut}");
        }
        // A formula length beyond the record, a token cut by the formula
        // length, a token that is no area, more areas than are looked at.
        let mut long = full.clone();
        long[4] = 200;
        assert!(print_name(&long).is_none());
        let mut short = full.clone();
        short[4] = 6;
        assert!(print_name(&short).is_none());
        assert!(print_name(&name(0x06, 1, &[0x1E, 1, 0])).is_none());
        let many: Vec<u8> = (0..=MAX_NAME_AREAS)
            .flat_map(|_| area(0, 0, 0, 0))
            .collect();
        assert!(print_name(&name(0x07, 1, &many)).is_none());
        // Every single byte as a formula, and as the token after an area.
        for b in 0..=255u8 {
            let _ = print_name(&name(0x06, 1, &[b]));
            let mut rgce = area(0, 1, 0, 1);
            rgce.push(b);
            let _ = print_name(&name(0x07, 1, &rgce));
        }
        // A 16-bit name string.
        let mut wide = full.clone();
        wide[14] = 1;
        wide.insert(16, 0);
        assert_eq!(print_name(&wide).map(|p| p.areas.len()), Some(1));
    }

    #[test]
    fn margins_and_papers() {
        assert_eq!(inch_to_hmm(0.5), Some(1270));
        assert_eq!(inch_to_hmm(-1.0), Some(0));
        assert_eq!(inch_to_hmm(1e9), Some(127_000));
        assert_eq!(inch_to_hmm(f64::NAN), None);
        assert_eq!(inch_to_hmm(f64::INFINITY), None);
        assert_eq!(f64_at(&[0; 7], 0), None);
        assert_eq!(f64_at(&[0; 8], usize::MAX), None);
        // Every paper code maps to a size or to none, upright.
        for code in 0..=u16::MAX {
            if let Some(p) = paper_of(code) {
                let (w, h) = p.size_hmm();
                assert!(w > 0 && w <= h, "code {code}");
            }
        }
        assert_eq!(paper_of(0), None);
        assert_eq!(paper_of(9), Some(PaperData::A4));
    }
}
