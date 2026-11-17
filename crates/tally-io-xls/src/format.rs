//! The BIFF8 formatting reader: everything about the look of a workbook
//! that calamine does not expose.
//!
//! It walks the workbook stream the pre-scan extracted and validated
//! ([`crate::prescan`]; the same record framing, substream walk and
//! `CONTINUE` handling) and reads
//!
//! - the workbook globals: `FONT`, `FORMAT`, `XF`, `PALETTE`, `WINDOW1`
//!   (active sheet) and the `MSODRAWINGGROUP` picture store;
//! - per sheet: `COLINFO`, `DEFCOLWIDTH`, `STANDARDWIDTH`, `ROW`,
//!   `DEFAULTROWHEIGHT`, the XF index of every cell record (`BLANK` and
//!   `MULBLANK` included), `WINDOW2`, `PANE`, `SCL`, `SHEETEXT`, the
//!   `MSODRAWING` picture anchors and cell notes (`NOTE` with the text of
//!   its `TXO`).
//!
//! The result maps onto the model the way the XLSX reader maps the
//! equivalent constructs (its unit conversions, palette, border table and
//! number-format translation are reused).
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
    AlignData, BorderLine, BordersData, CellProtectionData, ChartAnchorData, DimRun, FontData,
    HAlign, ImageData, NoteData, SplitData, StyleData, StyleRun, UnderlineStyleData, VAlign,
    ViewOptionsData,
};
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::prescan::{bof_is_pre_biff8, record, u16_at, u32_at, Rec};
use crate::Limits;

/// Columns of a BIFF8 sheet.
const BIFF_COLS: usize = 256;
/// Most entries of the picture store looked at.
const MAX_BLIPS: usize = 65_536;

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

/// The format of one cell record.
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
    /// Cells whose XF is not the default look, sorted row-major, one per
    /// position.
    pub cells: Vec<CellFmt>,
    pub view_options: Option<ViewOptionsData>,
    pub frozen: Option<(u32, u32)>,
    pub split: Option<SplitData>,
    pub zoom: Option<u16>,
    pub tab_color: Option<u32>,
    pub images: Vec<ImageData>,
    /// Cell notes, sorted row-major.
    pub notes: Vec<NoteData>,
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
    drawing_bytes: bool,
    images: bool,
    notes: bool,
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

impl StyleBuilder<'_> {
    /// BIFF font indices skip 4: the fifth `FONT` record is font 5.
    fn font(&self, ifnt: u16) -> Option<&RawFont> {
        let i = match ifnt {
            0..=3 => usize::from(ifnt),
            4 => return self.fonts.first(),
            _ => usize::from(ifnt) - 1,
        };
        self.fonts.get(i).or_else(|| self.fonts.first())
    }

    fn font_data(&self, ifnt: u16) -> FontData {
        let Some(f) = self.font(ifnt) else {
            return FontData::default();
        };
        let size = f32::from(f.twips.min(409 * 20)) / 20.0;
        FontData {
            bold: f.bls >= 600,
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
            .or_else(|| tally_io_xlsx::builtin_format_code(u32::from(ifmt)));
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
    match typ {
        // JPEG, PNG, JPEG (CMYK).
        0xF01D | 0xF01E | 0xF02A => {}
        0xF01A..=0xF01C | 0xF01F | 0xF029 => return Blip::Unsupported,
        _ => return Blip::Missing,
    }
    // One or two 16-byte UIDs (odd instance: two) and a tag byte, then the
    // file. What it is is taken from its signature, as in the XLSX
    // reader: writers are known to store a PNG in a JPEG record.
    let first = if (vi >> 4) & 1 == 1 { 33 } else { 17 };
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
    xf: u16,
}

#[derive(Clone, Copy)]
struct RowRec {
    row: u32,
    twips: u16,
    flags: u32,
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
    /// Maximum digit width of the default font, px.
    mdw: f64,
    blips: Vec<Blip>,
    dropped: Dropped,
    // Workbook-wide counters.
    rows: usize,
    cells: u64,
    drawing_bytes: usize,
    images: usize,
    image_bytes: u64,
    notes: usize,
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

impl Reader<'_> {
    fn cell(&mut self, out: &mut Vec<CellFmt>, row: u32, col: u32, xf: u16, value: bool) {
        let Some(info) = self.xfs.get(usize::from(xf)) else {
            return;
        };
        // A value cell needs its style; an empty one only if the format
        // shows.
        if info.style.is_none() || (!value && info.plain) || col >= BIFF_COLS as u32 {
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

    fn sheet(&mut self, start: usize, walked: &mut u64, walk_budget: u64) -> SheetFormat {
        let s = self.s;
        let mut out = SheetFormat::default();
        let mut cols: Vec<Option<ColDef>> = vec![None; BIFF_COLS];
        let mut tail: Option<ColDef> = None;
        let mut col_infos = 0usize;
        let mut def_col_chars: Option<u16> = None;
        let mut standard_width: Option<u16> = None;
        let mut def_row_twips: Option<u16> = None;
        let mut rows: Vec<RowRec> = Vec::new();
        let mut window2: Option<u16> = None;
        let mut pane: Option<(u16, u16)> = None;
        let mut drawing: Vec<u8> = Vec::new();
        // Notes: the object records before a TXO name the object the text
        // belongs to; NOTE records point at the object.
        let mut last_obj: Option<(u16, u16)> = None;
        let mut note_texts: HashMap<u16, String> = HashMap::new();
        let mut note_recs: Vec<NoteRec> = Vec::new();
        let mut seen_bof = false;
        let mut depth = 0usize;
        let mut pos = start;
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
                        hidden: flags & 1 != 0 || width == 0,
                        xf,
                    };
                    let end = usize::from(last).min(BIFF_COLS - 1);
                    for slot in cols.iter_mut().take(end + 1).skip(usize::from(first)) {
                        *slot = Some(def);
                    }
                    // A range that reaches the last BIFF column runs to the
                    // end of the sheet.
                    if usize::from(last) >= BIFF_COLS - 1 {
                        tail = Some(def);
                    }
                }
                0x0055 => def_col_chars = w(0).or(def_col_chars),
                0x0099 => standard_width = w(0).or(standard_width),
                0x0225 => def_row_twips = w(2).or(def_row_twips),
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
                    rows.push(RowRec {
                        row: u32::from(row),
                        twips: h,
                        flags,
                    });
                }
                // NUMBER, RK, LABELSST, LABEL, RSTRING, FORMULA, BOOLERR.
                0x0203 | 0x027E | 0x00FD | 0x0204 | 0x00D6 | 0x0006 | 0x0205 => {
                    if let (Some(row), Some(col), Some(xf)) = (w(0), w(2), w(4)) {
                        self.cell(&mut out.cells, u32::from(row), u32::from(col), xf, true);
                    }
                }
                // BLANK
                0x0201 => {
                    if let (Some(row), Some(col), Some(xf)) = (w(0), w(2), w(4)) {
                        self.cell(&mut out.cells, u32::from(row), u32::from(col), xf, false);
                    }
                }
                // MULRK: row, first column, (XF, RK)…, last column.
                0x00BD => {
                    if let (Some(row), Some(col)) = (w(0), w(2)) {
                        for k in 0..d.len().saturating_sub(6) / 6 {
                            if let Some(xf) = w(4 + 6 * k) {
                                let c = u32::from(col) + k as u32;
                                self.cell(&mut out.cells, u32::from(row), c, xf, true);
                            }
                        }
                    }
                }
                // MULBLANK: row, first column, XF…, last column.
                0x00BE => {
                    if let (Some(row), Some(col)) = (w(0), w(2)) {
                        for k in 0..d.len().saturating_sub(6) / 2 {
                            if let Some(xf) = w(4 + 2 * k) {
                                let c = u32::from(col) + k as u32;
                                self.cell(&mut out.cells, u32::from(row), c, xf, false);
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
                // SHEETEXT: a 12-byte header, the size, the tab colour.
                0x0862 => {
                    if let Some(icv) = u32_at(d, 16) {
                        let icv = (icv & 0x7F) as u16;
                        if icv != 0x7F {
                            out.tab_color = self.palette.rgb(icv).map(|c| (c << 8) | 0xff);
                        }
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
                _ => {}
            }
        }

        // Columns.
        let mdw = self.mdw;
        let to_hmm = |w: u16| tally_io_xlsx::column_width_to_hmm(f64::from(w) / 256.0, mdw);
        let default_col: Option<u32> = match (standard_width, def_col_chars) {
            (Some(w), _) if w > 0 => Some(to_hmm(w)),
            // Excel pads the characters and rounds up to 8 px.
            (None, Some(ch)) if ch > 0 => Some(tally_io_xlsx::px_to_hmm(
                ((f64::from(ch.min(255)) * mdw + 5.0) / 8.0).ceil() * 8.0,
            )),
            _ => None,
        }
        .filter(|w| *w != tally_io_xlsx::DEFAULT_COL_WIDTH_HMM);
        let size_of = |def: Option<ColDef>| -> Option<(u32, bool)> {
            match def {
                Some(c) if c.width == 0 => Some((
                    default_col.unwrap_or(tally_io_xlsx::DEFAULT_COL_WIDTH_HMM),
                    true,
                )),
                Some(c) => Some((to_hmm(c.width), c.hidden)),
                None => default_col.map(|w| (w, false)),
            }
        };
        for (c, def) in cols.iter().enumerate() {
            if let Some((size, hidden)) = size_of(*def) {
                push_run(&mut out.col_widths, c as u32, c as u32, size, hidden);
            }
            let style = def
                .and_then(|d| self.xfs.get(usize::from(d.xf)))
                .filter(|x| !x.plain)
                .and_then(|x| x.style);
            if let Some(style) = style {
                out.style_runs.push(StyleRun {
                    col: c as u32,
                    start_row: 0,
                    end_row: MAX_ROWS - 1,
                    style,
                });
            }
        }
        if let Some((size, hidden)) = size_of(tail) {
            push_run(
                &mut out.col_widths,
                BIFF_COLS as u32,
                MAX_COLS - 1,
                size,
                hidden,
            );
        }

        // Rows: a height the user (or the writer) set, or a hidden row. A
        // height without the "unsynced" flag is an auto-fit result, which
        // the application re-fits (as for XLSX rows without customHeight).
        let default_row = def_row_twips
            .map(|t| t & 0x7FFF)
            .filter(|t| *t > 0)
            .map_or(tally_io_xlsx::DEFAULT_ROW_HEIGHT_HMM, twips_to_hmm);
        if !rows.is_sorted_by_key(|r| r.row) {
            rows.sort_by_key(|r| r.row);
        }
        let mut row_sizes: Vec<(u32, i64)> = Vec::with_capacity(rows.len());
        for (i, r) in rows.iter().enumerate() {
            // Of several records for one row the last one counts.
            if rows.get(i + 1).is_some_and(|n| n.row == r.row) {
                continue;
            }
            let hidden = r.flags & 0x20 != 0;
            let unsynced = r.flags & 0x40 != 0;
            let twips = r.twips & 0x7FFF;
            let own = (twips > 0 && r.twips & 0x8000 == 0).then(|| twips_to_hmm(twips));
            if hidden || (unsynced && own.is_some()) {
                push_run(
                    &mut out.row_heights,
                    r.row,
                    r.row,
                    own.unwrap_or(default_row),
                    hidden,
                );
            }
            let shown = if hidden {
                0
            } else {
                own.unwrap_or(default_row)
            };
            row_sizes.push((r.row, i64::from(shown)));
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
                    // Frozen: columns and rows.
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

        // Pictures.
        if !drawing.is_empty() {
            let col_default =
                i64::from(default_col.unwrap_or(tally_io_xlsx::DEFAULT_COL_WIDTH_HMM));
            let shown = |def: Option<ColDef>| match size_of(def) {
                Some((_, true)) => 0,
                Some((size, false)) => i64::from(size),
                None => col_default,
            };
            let col_sizes: Vec<(u32, i64)> = cols
                .iter()
                .enumerate()
                .map(|(c, def)| (c as u32, shown(*def)))
                .collect();
            let col_axis = Axis::new(col_sizes, shown(tail));
            let row_axis = Axis::new(row_sizes, i64::from(default_row));
            self.pictures(&drawing, &col_axis, &row_axis, note_shapes, &mut out.images);
        }
        out
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
                // line, a control…
                (None, Some(_)) => others += 1,
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
                formats.insert(id, unicode(d, 4, usize::from(cch)));
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
    let default_size = fonts
        .first()
        .filter(|f| f.twips > 0)
        .map(|f| f32::from(f.twips.min(409 * 20)) / 20.0);
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
    let mut styles: Vec<StyleData> = Vec::new();
    let mut dedup: HashMap<String, u32> = HashMap::new();
    let mut xfs: Vec<XfInfo> = Vec::with_capacity(raw_xfs.len());
    for x in &raw_xfs {
        let style = builder.style(x);
        let mut info = XfInfo {
            style: None,
            date: style
                .number_format
                .as_deref()
                .is_some_and(tally_io_xlsx::is_date_format),
            has_format: style.number_format.is_some(),
            plain: style == base,
        };
        if style != StyleData::default() {
            let key = format!("{style:?}");
            let next = u32::try_from(styles.len()).unwrap_or(u32::MAX);
            let idx = *dedup.entry(key).or_insert(next);
            if idx == next {
                styles.push(style);
            }
            info.style = Some(idx);
        } else {
            info.plain = true;
        }
        xfs.push(info);
    }
    dropped.long_format = builder.long_format;

    // The picture store.
    let mut blips: Vec<Blip> = Vec::new();
    art_scan(&group, |r| {
        if r.typ == 0xF007 && blips.len() < MAX_BLIPS {
            blips.push(blip_of(r.body, limits.max_image_bytes));
        }
    });
    drop(group);

    let mut reader = Reader {
        s,
        limits,
        xfs,
        palette,
        mdw: tally_io_xlsx::max_digit_width(default_family.as_deref(), default_size),
        blips,
        dropped,
        rows: 0,
        cells: 0,
        drawing_bytes: 0,
        images: 0,
        image_bytes: 0,
        notes: 0,
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
        warnings,
    }
}
