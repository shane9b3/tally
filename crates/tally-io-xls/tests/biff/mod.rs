//! A small test-only BIFF8 writer: records → workbook stream → compound
//! file, so `.xls` fixtures are code, not blobs.
#![allow(dead_code, clippy::unwrap_used)]

use tally_io_xls::prescan;

/// One record: type, length, data.
pub fn rec(t: u16, d: &[u8]) -> Vec<u8> {
    let mut v = t.to_le_bytes().to_vec();
    v.extend_from_slice(&(d.len() as u16).to_le_bytes());
    v.extend_from_slice(d);
    v
}

fn words(ws: &[u16]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// `BOF` of a BIFF8 substream (`kind` 5 = globals, 0x10 = worksheet).
pub fn bof(kind: u16) -> Vec<u8> {
    let mut d = words(&[0x0600, kind, 0x0DBB, 0x07CC]);
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&6u32.to_le_bytes());
    rec(0x0809, &d)
}

/// `BOF` of a BIFF5 substream.
pub fn bof5(kind: u16) -> Vec<u8> {
    rec(0x0809, &words(&[0x0500, kind, 0x0DBB, 0x07CC]))
}

/// A 16-bit `XLUnicodeString` body (flags = 1) without the length.
fn utf16(s: &str) -> Vec<u8> {
    let mut d = vec![1u8];
    for u in s.encode_utf16() {
        d.extend_from_slice(&u.to_le_bytes());
    }
    d
}

/// `FONT`.
pub struct Font<'a> {
    pub name: &'a str,
    pub twips: u16,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub underline: u8,
    pub color: u16,
}

impl Default for Font<'_> {
    fn default() -> Self {
        Self {
            name: "Arial",
            twips: 200,
            bold: false,
            italic: false,
            strike: false,
            underline: 0,
            color: 0x7FFF,
        }
    }
}

pub fn font(f: &Font) -> Vec<u8> {
    let grbit = u16::from(f.italic) << 1 | u16::from(f.strike) << 3;
    let mut d = words(&[f.twips, grbit, f.color, if f.bold { 700 } else { 400 }, 0]);
    d.extend_from_slice(&[f.underline, 0, 0, 0]);
    d.push(f.name.encode_utf16().count() as u8);
    d.extend(utf16(f.name));
    rec(0x0031, &d)
}

/// `FORMAT` (number format `id` with an Excel code).
pub fn format(id: u16, code: &str) -> Vec<u8> {
    let mut d = words(&[id, code.encode_utf16().count() as u16]);
    d.extend(utf16(code));
    rec(0x041E, &d)
}

/// `XF` fields; the defaults are a plain cell XF.
#[derive(Clone)]
pub struct Xf {
    pub font: u16,
    pub fmt: u16,
    pub locked: bool,
    pub hidden: bool,
    pub style: bool,
    pub halign: u8,
    pub wrap: bool,
    pub valign: u8,
    pub rotation: u8,
    pub indent: u8,
    pub shrink: bool,
    /// Line styles: left, right, top, bottom, diagonal.
    pub lines: [u8; 5],
    /// Colour indices: left, right, top, bottom, diagonal.
    pub line_colors: [u8; 5],
    pub diag_down: bool,
    pub diag_up: bool,
    pub pattern: u8,
    pub fore: u8,
    pub back: u8,
}

impl Default for Xf {
    fn default() -> Self {
        Self {
            font: 0,
            fmt: 0,
            locked: true,
            hidden: false,
            style: false,
            halign: 0,
            wrap: false,
            valign: 2,
            rotation: 0,
            indent: 0,
            shrink: false,
            lines: [0; 5],
            line_colors: [0x40; 5],
            diag_down: false,
            diag_up: false,
            pattern: 0,
            fore: 0x40,
            back: 0x41,
        }
    }
}

pub fn xf(x: &Xf) -> Vec<u8> {
    let w4 = u16::from(x.locked)
        | u16::from(x.hidden) << 1
        | u16::from(x.style) << 2
        | if x.style { 0xFFF0 } else { 0 };
    let b6 = x.halign & 7 | u8::from(x.wrap) << 3 | (x.valign & 7) << 4;
    let b8 = x.indent & 15 | u8::from(x.shrink) << 4;
    let lines = u16::from(x.lines[0] & 15)
        | u16::from(x.lines[1] & 15) << 4
        | u16::from(x.lines[2] & 15) << 8
        | u16::from(x.lines[3] & 15) << 12;
    let c1 = u16::from(x.line_colors[0] & 0x7F)
        | u16::from(x.line_colors[1] & 0x7F) << 7
        | u16::from(x.diag_down) << 14
        | u16::from(x.diag_up) << 15;
    let c2 = u32::from(x.line_colors[2] & 0x7F)
        | u32::from(x.line_colors[3] & 0x7F) << 7
        | u32::from(x.line_colors[4] & 0x7F) << 14
        | u32::from(x.lines[4] & 15) << 21
        | u32::from(x.pattern & 0x3F) << 26;
    let fill = u16::from(x.fore & 0x7F) | u16::from(x.back & 0x7F) << 7;
    let mut d = words(&[x.font, x.fmt, w4]);
    d.extend_from_slice(&[b6, x.rotation, b8, 0]);
    d.extend(words(&[lines, c1]));
    d.extend_from_slice(&c2.to_le_bytes());
    d.extend(words(&[fill]));
    rec(0x00E0, &d)
}

/// `PALETTE` with `colors` as `0xRRGGBB` (indices 8…).
pub fn palette(colors: &[u32]) -> Vec<u8> {
    let mut d = words(&[colors.len() as u16]);
    for c in colors {
        d.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, *c as u8, 0]);
    }
    rec(0x0092, &d)
}

/// `SST` with unformatted strings.
pub fn sst(strings: &[&str]) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    d.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    for s in strings {
        d.extend(words(&[s.encode_utf16().count() as u16]));
        d.extend(utf16(s));
    }
    rec(0x00FC, &d)
}

pub fn number(r: u16, c: u16, x: u16, v: f64) -> Vec<u8> {
    let mut d = words(&[r, c, x]);
    d.extend_from_slice(&v.to_le_bytes());
    rec(0x0203, &d)
}

/// `RK` holding the integer `v`.
pub fn rk(r: u16, c: u16, x: u16, v: i32) -> Vec<u8> {
    let mut d = words(&[r, c, x]);
    d.extend_from_slice(&((v << 2) | 2).to_le_bytes());
    rec(0x027E, &d)
}

/// `MULRK`: integers from column `c` on, each with its XF.
pub fn mulrk(r: u16, c: u16, cells: &[(u16, i32)]) -> Vec<u8> {
    let mut d = words(&[r, c]);
    for (x, v) in cells {
        d.extend(words(&[*x]));
        d.extend_from_slice(&((v << 2) | 2).to_le_bytes());
    }
    d.extend(words(&[c + cells.len() as u16 - 1]));
    rec(0x00BD, &d)
}

pub fn label(r: u16, c: u16, x: u16, s: &str) -> Vec<u8> {
    let mut d = words(&[r, c, x, s.encode_utf16().count() as u16]);
    d.extend(utf16(s));
    rec(0x0204, &d)
}

pub fn labelsst(r: u16, c: u16, x: u16, i: u32) -> Vec<u8> {
    let mut d = words(&[r, c, x]);
    d.extend_from_slice(&i.to_le_bytes());
    rec(0x00FD, &d)
}

pub fn blank(r: u16, c: u16, x: u16) -> Vec<u8> {
    rec(0x0201, &words(&[r, c, x]))
}

pub fn mulblank(r: u16, c: u16, xfs: &[u16]) -> Vec<u8> {
    let mut d = words(&[r, c]);
    d.extend(words(xfs));
    d.extend(words(&[c + xfs.len() as u16 - 1]));
    rec(0x00BE, &d)
}

pub fn boolerr(r: u16, c: u16, x: u16, v: u8, is_err: bool) -> Vec<u8> {
    let mut d = words(&[r, c, x]);
    d.extend_from_slice(&[v, u8::from(is_err)]);
    rec(0x0205, &d)
}

/// `FORMULA` with a cached number and the parsed expression `rgce`.
pub fn formula(r: u16, c: u16, x: u16, cached: f64, rgce: &[u8]) -> Vec<u8> {
    let mut d = words(&[r, c, x]);
    d.extend_from_slice(&cached.to_le_bytes());
    d.extend(words(&[0]));
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend(words(&[rgce.len() as u16]));
    d.extend_from_slice(rgce);
    rec(0x0006, &d)
}

/// `PtgRef` to a relative cell.
pub fn ptg_ref(r: u16, c: u16) -> Vec<u8> {
    let mut d = vec![0x44];
    d.extend(words(&[r, c | 0xC000]));
    d
}

/// `PtgInt`.
pub fn ptg_int(v: u16) -> Vec<u8> {
    let mut d = vec![0x1E];
    d.extend(words(&[v]));
    d
}

/// `MERGEDCELLS`: `(r0, r1, c0, c1)` each.
pub fn merged(ranges: &[(u16, u16, u16, u16)]) -> Vec<u8> {
    let mut d = words(&[ranges.len() as u16]);
    for (r0, r1, c0, c1) in ranges {
        d.extend(words(&[*r0, *r1, *c0, *c1]));
    }
    rec(0x00E5, &d)
}

/// `COLINFO`: width in 1/256 character.
pub fn colinfo(first: u16, last: u16, width: u16, x: u16, flags: u16) -> Vec<u8> {
    rec(0x007D, &words(&[first, last, width, x, flags, 0]))
}

/// `ROW`: height in twips and the flag dword (the XF goes in bits 16–27).
pub fn row(r: u16, twips: u16, flags: u32) -> Vec<u8> {
    let mut d = words(&[r, 0, 0, twips, 0, 0]);
    d.extend_from_slice(&flags.to_le_bytes());
    rec(0x0208, &d)
}

pub fn defcolwidth(chars: u16) -> Vec<u8> {
    rec(0x0055, &words(&[chars]))
}

pub fn standardwidth(w: u16) -> Vec<u8> {
    rec(0x0099, &words(&[w]))
}

pub fn defaultrowheight(flags: u16, twips: u16) -> Vec<u8> {
    rec(0x0225, &words(&[flags, twips]))
}

/// `WINDOW2` with the option flags.
pub fn window2(flags: u16) -> Vec<u8> {
    let mut d = words(&[flags, 0, 0]);
    d.extend_from_slice(&0x40u32.to_le_bytes());
    d.extend(words(&[0, 0]));
    d.extend_from_slice(&0u32.to_le_bytes());
    rec(0x023E, &d)
}

pub fn pane(x: u16, y: u16) -> Vec<u8> {
    let mut d = words(&[x, y, y, x]);
    d.push(0);
    rec(0x0041, &d)
}

/// `WINDOW1` with the active tab.
pub fn window1(active: u16) -> Vec<u8> {
    rec(
        0x003D,
        &words(&[0, 0, 0x4000, 0x2000, 0x38, active, 0, 1, 600]),
    )
}

/// `SCL` (zoom as a fraction).
pub fn scl(num: u16, den: u16) -> Vec<u8> {
    rec(0x00A0, &words(&[num, den]))
}

/// `SHEETEXT` with a tab colour index.
pub fn sheetext(icv: u32) -> Vec<u8> {
    let mut d = words(&[0x0862, 0, 0, 0, 0, 0]);
    d.extend_from_slice(&0x14u32.to_le_bytes());
    d.extend_from_slice(&icv.to_le_bytes());
    rec(0x0862, &d)
}

// ------------------------------------------------------------- drawings

/// One OfficeArt record.
pub fn art(ver: u8, inst: u16, typ: u16, body: &[u8]) -> Vec<u8> {
    let mut d = words(&[u16::from(ver) | inst << 4, typ]);
    d.extend_from_slice(&(body.len() as u32).to_le_bytes());
    d.extend_from_slice(body);
    d
}

/// A BSE (`0xF007`) holding one blip of record type `blip_type` (`0xF01E`
/// PNG, `0xF01D` JPEG, `0xF01A` EMF…).
pub fn bse(blip_type: u16, inst: u16, data: &[u8]) -> Vec<u8> {
    let mut blip = vec![0u8; 16];
    blip.push(0xFF);
    blip.extend_from_slice(data);
    let blip = art(0, inst, blip_type, &blip);
    let mut b = vec![6u8, 6];
    b.extend_from_slice(&[0u8; 16]);
    b.extend(words(&[0xFF]));
    b.extend_from_slice(&(blip.len() as u32).to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend(blip);
    art(2, 6, 0xF007, &b)
}

/// The OfficeArt body of `MSODRAWINGGROUP`: a `DggContainer` with a BLIP
/// store of `bses`.
pub fn dgg(bses: &[Vec<u8>]) -> Vec<u8> {
    let store = art(0xF, bses.len() as u16, 0xF001, &bses.concat());
    let mut body = art(0, 0, 0xF006, &[0u8; 16]);
    body.extend(store);
    art(0xF, 0, 0xF000, &body)
}

/// `MSODRAWINGGROUP` records for `body`, split with `CONTINUE` records
/// after `chunk` bytes.
pub fn msodrawinggroup(body: &[u8], chunk: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, part) in body.chunks(chunk.max(1)).enumerate() {
        out.extend(rec(if i == 0 { 0x00EB } else { 0x003C }, part));
    }
    out
}

/// A shape: its type (75 = picture frame, 202 = text box), the blip it
/// shows (1-based) and its client anchor `(col1, dx1, row1, dy1, col2,
/// dx2, row2, dy2)`.
pub fn shape(kind: u16, pib: Option<u32>, anchor: [u16; 8]) -> Vec<u8> {
    let mut body = art(2, kind, 0xF00A, &[0u8; 8]);
    if let Some(p) = pib {
        let mut opt = words(&[0x4104]);
        opt.extend_from_slice(&p.to_le_bytes());
        body.extend(art(3, 1, 0xF00B, &opt));
    }
    let mut a = words(&[0]);
    a.extend(words(&anchor));
    body.extend(art(0, 0, 0xF010, &a));
    body.extend(art(0, 0, 0xF011, &[]));
    art(0xF, 0, 0xF004, &body)
}

/// A sheet's drawing: `DgContainer` ▸ `SpgrContainer` ▸ shapes.
pub fn dg(shapes: &[Vec<u8>]) -> Vec<u8> {
    let spgr = art(0xF, 0, 0xF003, &shapes.concat());
    let mut body = art(0, 1, 0xF008, &[0u8; 8]);
    body.extend(spgr);
    art(0xF, 0, 0xF002, &body)
}

pub fn msodrawing(body: &[u8]) -> Vec<u8> {
    rec(0x00EC, body)
}

/// A minimal `OBJ` record (picture).
pub fn obj() -> Vec<u8> {
    let mut d = words(&[0x15, 0x12, 8, 1, 0x6011]);
    d.extend_from_slice(&[0u8; 12]);
    d.extend(words(&[0, 0]));
    rec(0x005D, &d)
}

// ------------------------------------------------------------- workbook

/// The smallest valid PNG signature plus a few bytes (not a real image;
/// readers store pictures without decoding them).
pub fn fake_png(n: usize) -> Vec<u8> {
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend((0..n).map(|i| i as u8));
    v
}

/// A workbook stream: the globals (BOF, `globals`, one BOUNDSHEET per
/// sheet, EOF) and one worksheet substream per `(name, records)`.
pub fn stream(globals: &[u8], sheets: &[(&str, Vec<u8>)]) -> Vec<u8> {
    stream_with(bof(5), bof(0x10), globals, sheets)
}

/// [`stream`] with BIFF5 `BOF` records.
pub fn stream5(globals: &[u8], sheets: &[(&str, Vec<u8>)]) -> Vec<u8> {
    stream_with(bof5(5), bof5(0x10), globals, sheets)
}

fn stream_with(
    gbof: Vec<u8>,
    sbof: Vec<u8>,
    globals: &[u8],
    sheets: &[(&str, Vec<u8>)],
) -> Vec<u8> {
    let biff5 = gbof.len() < 20;
    let mut g = gbof;
    g.extend(rec(0x0042, &0x04B0u16.to_le_bytes()));
    g.extend_from_slice(globals);
    let mut pos = Vec::new();
    for (name, _) in sheets {
        pos.push(g.len() + 4);
        let mut d = vec![0u8; 6];
        if biff5 {
            d.push(name.len() as u8);
            d.extend_from_slice(name.as_bytes());
        } else {
            d.push(name.encode_utf16().count() as u8);
            d.extend(utf16(name));
        }
        g.extend(rec(0x0085, &d));
    }
    g.extend(rec(0x000A, &[]));
    for (p, (_, body)) in pos.iter().zip(sheets) {
        let at = g.len() as u32;
        g[*p..*p + 4].copy_from_slice(&at.to_le_bytes());
        g.extend_from_slice(&sbof);
        g.extend_from_slice(body);
        g.extend(rec(0x000A, &[]));
    }
    g
}

/// The usual globals: `fonts` (index 4 is skipped by BIFF, so the fifth
/// entry is font 5), `formats`, 16 default XFs (0–14 style XFs, 15 the
/// default cell XF) and then `xfs` (XF 16…).
pub fn globals(fonts: &[Font], formats: &[(u16, &str)], xfs: &[Xf]) -> Vec<u8> {
    let mut g = Vec::new();
    for f in fonts {
        g.extend(font(f));
    }
    for (id, code) in formats {
        g.extend(format(*id, code));
    }
    for i in 0..16 {
        g.extend(xf(&Xf {
            style: i < 15,
            ..Xf::default()
        }));
    }
    for x in xfs {
        g.extend(xf(x));
    }
    g
}

/// Wraps a workbook stream in a compound file.
pub fn file(stream: Vec<u8>) -> Vec<u8> {
    prescan::build_container(stream).0
}

/// `RSTRING`: a label with (ignored) formatting runs.
pub fn rstring(r: u16, c: u16, x: u16, s: &str) -> Vec<u8> {
    let mut d = words(&[r, c, x, s.encode_utf16().count() as u16]);
    d.extend(utf16(s));
    d.extend(words(&[1, 0, 0]));
    rec(0x00D6, &d)
}

/// The records of a cell note: the `OBJ` of its text box (object `id`),
/// the `TXO` with its text, and the `NOTE` that places it.
pub fn note(r: u16, c: u16, id: u16, shown: bool, author: &str, text: &str) -> Vec<u8> {
    // OBJ: ftCmo (type 0x19 = note) and ftEnd.
    let mut o = words(&[0x15, 0x12, 0x19, id, 0x4011]);
    o.extend_from_slice(&[0u8; 12]);
    o.extend(words(&[0, 0]));
    let mut out = rec(0x005D, &o);
    // TXO, then CONTINUE with the text and CONTINUE with the runs.
    let cch = text.encode_utf16().count() as u16;
    let mut t = words(&[0x0212, 0, 0, 0, 0, cch, 16, 0, 0]);
    t.truncate(18);
    out.extend(rec(0x01B6, &t));
    out.extend(rec(0x003C, &utf16(text)));
    out.extend(rec(0x003C, &[0u8; 16]));
    let mut n = words(&[r, c, if shown { 2 } else { 0 }, id]);
    n.extend(words(&[author.encode_utf16().count() as u16]));
    n.extend(utf16(author));
    out.extend(rec(0x001C, &n));
    out
}
pub mod books;
