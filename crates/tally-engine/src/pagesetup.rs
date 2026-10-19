//! Page setup (print model only; no rendering yet) and document
//! properties: API ⇄ model conversions and how print ranges, repeated
//! rows/columns and manual breaks follow row/column changes.

use std::collections::BTreeSet;

use tally_api::{
    ApiError, DocPropertiesDto, HeaderFooter, Margins, Orientation, PageOrder, PageScale,
    PageSetupDto, Paper, RangeDto,
};
use tally_model::{
    CellRangeLocal, DocMeta, HeaderFooterData, MarginsData, OrientationData, PageOrderData,
    PageScaleData, PageSetupData, PaperData,
};
use tally_types::{MAX_COLS, MAX_ROWS};

use crate::storage::SheetMeta;
use crate::structure::{point_after, span_after, Shape};

/// Longest header/footer region text.
const MAX_HF_TEXT: usize = 4096;
/// Longest document property text.
const MAX_PROP_TEXT: usize = 64 * 1024;

fn mm(hmm: u32) -> f32 {
    hmm as f32 / 100.0
}

fn hmm(mm: f32) -> Result<u32, ApiError> {
    if !mm.is_finite() || !(0.0..=10_000.0).contains(&mm) {
        return Err(ApiError::invalid_argument("bad length"));
    }
    Ok((mm * 100.0).round() as u32)
}

fn hf_to_api(h: &HeaderFooterData) -> HeaderFooter {
    HeaderFooter {
        left: h.left.clone(),
        center: h.center.clone(),
        right: h.right.clone(),
    }
}

fn hf_from_api(h: &HeaderFooter) -> Result<HeaderFooterData, ApiError> {
    for t in [&h.left, &h.center, &h.right] {
        if t.len() > MAX_HF_TEXT {
            return Err(ApiError::invalid_argument("header/footer text is too long"));
        }
    }
    Ok(HeaderFooterData {
        left: h.left.clone(),
        center: h.center.clone(),
        right: h.right.clone(),
    })
}

/// The API view (with the sheet's manual breaks).
pub fn to_api(meta: &SheetMeta) -> PageSetupDto {
    let p = &meta.page_setup;
    PageSetupDto {
        orientation: match p.orientation {
            OrientationData::Portrait => Orientation::Portrait,
            OrientationData::Landscape => Orientation::Landscape,
        },
        paper: match p.paper {
            PaperData::A4 => Paper::A4,
            PaperData::A3 => Paper::A3,
            PaperData::A5 => Paper::A5,
            PaperData::Letter => Paper::Letter,
            PaperData::Legal => Paper::Legal,
            PaperData::Executive => Paper::Executive,
            PaperData::Tabloid => Paper::Tabloid,
            PaperData::B5 => Paper::B5,
            PaperData::Custom {
                width_hmm,
                height_hmm,
            } => Paper::Custom {
                width_mm: mm(width_hmm),
                height_mm: mm(height_hmm),
            },
        },
        margins: Margins {
            top_mm: mm(p.margins.top),
            bottom_mm: mm(p.margins.bottom),
            left_mm: mm(p.margins.left),
            right_mm: mm(p.margins.right),
            header_mm: mm(p.margins.header),
            footer_mm: mm(p.margins.footer),
        },
        scale: match p.scale {
            PageScaleData::Percent(n) => PageScale::Percent { percent: n },
            PageScaleData::FitTo { width, height } => PageScale::FitTo { width, height },
        },
        print_range: p.print_range.map(|r| RangeDto {
            r0: r.r0,
            c0: r.c0,
            r1: r.r1,
            c1: r.c1,
        }),
        repeat_rows: p.repeat_rows.map(|(a, b)| [a, b]),
        repeat_cols: p.repeat_cols.map(|(a, b)| [a, b]),
        header: p.header.as_ref().map(hf_to_api),
        footer: p.footer.as_ref().map(hf_to_api),
        print_gridlines: p.print_gridlines,
        print_headers: p.print_headers,
        center_h: p.center_h,
        center_v: p.center_v,
        page_order: match p.page_order {
            PageOrderData::TopToBottom => PageOrder::TopToBottom,
            PageOrderData::LeftToRight => PageOrder::LeftToRight,
        },
        first_page_number: p.first_page_number,
        row_breaks: meta.row_breaks.iter().copied().collect(),
        col_breaks: meta.col_breaks.iter().copied().collect(),
    }
}

fn span(v: Option<[u32; 2]>, limit: u32) -> Result<Option<(u32, u32)>, ApiError> {
    match v {
        None => Ok(None),
        Some([a, b]) if a <= b && b < limit => Ok(Some((a, b))),
        Some(_) => Err(ApiError::invalid_argument("bad repeat range")),
    }
}

/// The model form of an API page setup (validated).
pub fn from_api(d: &PageSetupDto) -> Result<PageSetupData, ApiError> {
    let paper = match d.paper {
        Paper::A4 => PaperData::A4,
        Paper::A3 => PaperData::A3,
        Paper::A5 => PaperData::A5,
        Paper::Letter => PaperData::Letter,
        Paper::Legal => PaperData::Legal,
        Paper::Executive => PaperData::Executive,
        Paper::Tabloid => PaperData::Tabloid,
        Paper::B5 => PaperData::B5,
        Paper::Custom {
            width_mm,
            height_mm,
        } => {
            let (w, h) = (hmm(width_mm)?, hmm(height_mm)?);
            if w < 1000 || h < 1000 {
                return Err(ApiError::invalid_argument("the paper is too small"));
            }
            PaperData::Custom {
                width_hmm: w,
                height_hmm: h,
            }
        }
    };
    let scale = match d.scale {
        PageScale::Percent { percent } => {
            if !(10..=400).contains(&percent) {
                return Err(ApiError::invalid_argument("scale must be 10–400 %"));
            }
            PageScaleData::Percent(percent)
        }
        PageScale::FitTo { width, height } => {
            if width > 1000 || height > 1000 {
                return Err(ApiError::invalid_argument("too many pages"));
            }
            PageScaleData::FitTo { width, height }
        }
    };
    let print_range = match d.print_range {
        None => None,
        Some(r) if r.r0 <= r.r1 && r.c0 <= r.c1 && r.r1 < MAX_ROWS && r.c1 < MAX_COLS => {
            Some(CellRangeLocal {
                r0: r.r0,
                c0: r.c0,
                r1: r.r1,
                c1: r.c1,
            })
        }
        Some(_) => return Err(ApiError::invalid_argument("bad print range")),
    };
    let m = &d.margins;
    Ok(PageSetupData {
        orientation: match d.orientation {
            Orientation::Portrait => OrientationData::Portrait,
            Orientation::Landscape => OrientationData::Landscape,
        },
        paper,
        margins: MarginsData {
            top: hmm(m.top_mm)?,
            bottom: hmm(m.bottom_mm)?,
            left: hmm(m.left_mm)?,
            right: hmm(m.right_mm)?,
            header: hmm(m.header_mm)?,
            footer: hmm(m.footer_mm)?,
        },
        scale,
        print_range,
        repeat_rows: span(d.repeat_rows, MAX_ROWS)?,
        repeat_cols: span(d.repeat_cols, MAX_COLS)?,
        header: d.header.as_ref().map(hf_from_api).transpose()?,
        footer: d.footer.as_ref().map(hf_from_api).transpose()?,
        print_gridlines: d.print_gridlines,
        print_headers: d.print_headers,
        center_h: d.center_h,
        center_v: d.center_v,
        page_order: match d.page_order {
            PageOrder::TopToBottom => PageOrderData::TopToBottom,
            PageOrder::LeftToRight => PageOrderData::LeftToRight,
        },
        first_page_number: d.first_page_number,
    })
}

/// Adjusts the print range, repeated rows/columns and breaks for a
/// row/column change.
pub fn adjust_for_structure(meta: &mut SheetMeta, sh: &Shape) {
    let p = &mut meta.page_setup;
    if let Some(r) = p.print_range {
        p.print_range = if sh.rows {
            span_after(sh, r.r0, r.r1).map(|(r0, r1)| CellRangeLocal { r0, r1, ..r })
        } else {
            span_after(sh, r.c0, r.c1).map(|(c0, c1)| CellRangeLocal { c0, c1, ..r })
        };
    }
    if sh.rows {
        if let Some((a, b)) = p.repeat_rows {
            p.repeat_rows = span_after(sh, a, b);
        }
    } else if let Some((a, b)) = p.repeat_cols {
        p.repeat_cols = span_after(sh, a, b);
    }
    let breaks = if sh.rows {
        &mut meta.row_breaks
    } else {
        &mut meta.col_breaks
    };
    if !breaks.is_empty() {
        let old = std::mem::take(breaks);
        *breaks = old
            .into_iter()
            .filter_map(|b| point_after(sh, b))
            .filter(|b| *b > 0)
            .collect::<BTreeSet<u32>>();
    }
}

fn clean(s: &Option<String>) -> Result<Option<String>, ApiError> {
    match s {
        Some(t) if t.len() > MAX_PROP_TEXT => {
            Err(ApiError::invalid_argument("a property is too long"))
        }
        Some(t) if t.trim().is_empty() => Ok(None),
        Some(t) => Ok(Some(t.clone())),
        None => Ok(None),
    }
}

/// The API view of the document metadata.
pub fn props_to_api(m: &DocMeta) -> DocPropertiesDto {
    DocPropertiesDto {
        title: m.title.clone(),
        subject: m.subject.clone(),
        keywords: m.keywords.clone(),
        description: m.description.clone(),
        author: m.author.clone(),
        created: m.created.clone(),
        modified: m.modified.clone(),
        generator: m.generator.clone(),
    }
}

/// Applies the editable properties of `p` to `m`.
pub fn apply_props(m: &mut DocMeta, p: &DocPropertiesDto) -> Result<(), ApiError> {
    if p.keywords.len() > 1000 || p.keywords.iter().any(|k| k.len() > 1024) {
        return Err(ApiError::invalid_argument("too many or too long keywords"));
    }
    m.title = clean(&p.title)?;
    m.subject = clean(&p.subject)?;
    m.description = clean(&p.description)?;
    m.author = clean(&p.author)?;
    m.keywords = p
        .keywords
        .iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_structure() {
        let mut meta = SheetMeta::default();
        let mut dto = to_api(&meta);
        dto.paper = Paper::Letter;
        dto.scale = PageScale::FitTo {
            width: 1,
            height: 0,
        };
        dto.print_range = Some(RangeDto {
            r0: 2,
            c0: 0,
            r1: 9,
            c1: 3,
        });
        dto.repeat_rows = Some([0, 1]);
        meta.page_setup = from_api(&dto).unwrap();
        meta.row_breaks.insert(5);
        let back = to_api(&meta);
        assert_eq!(back.paper, Paper::Letter);
        assert_eq!(back.row_breaks, vec![5]);
        assert_eq!(back.margins.top_mm, 20.0);
        let ins = Shape {
            sheet: 1,
            rows: true,
            at: 1,
            count: 2,
            insert: true,
        };
        adjust_for_structure(&mut meta, &ins);
        assert_eq!(meta.page_setup.repeat_rows, Some((0, 3)));
        assert_eq!(meta.page_setup.print_range.map(|r| r.r0), Some(4));
        assert!(meta.row_breaks.contains(&7));
        dto.scale = PageScale::Percent { percent: 5 };
        assert!(from_api(&dto).is_err());
    }
}
