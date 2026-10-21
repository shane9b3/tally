//! The Navigator (F5) and the document statistics (wave 5, W5-2).

use tally_api::{ApiError, DocumentStatsDto, NavItem, NavigatorDto, RangeDto};
use tally_formula::{Expr, RefExpr};
use tally_types::CellRange;

use crate::workbook::Workbook;

fn range_dto(r: &CellRange) -> RangeDto {
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

fn excerpt(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = one.chars().take(60).collect();
    if one.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// The Navigator's entries.
pub fn navigator(wb: &Workbook) -> Result<NavigatorDto, ApiError> {
    let mut out = NavigatorDto::default();
    for s in &wb.sheets {
        out.sheets.push(NavItem {
            title: s.name.to_string(),
            sheet: Some(s.id),
            range: s.used_range().map(|r| range_dto(&r)),
            id: None,
        });
        for d in s.meta.database_ranges.iter() {
            out.database_ranges.push(NavItem {
                title: d.name.clone(),
                sheet: Some(s.id),
                range: Some(RangeDto {
                    r0: d.range.r0,
                    c0: d.range.c0,
                    r1: d.range.r1,
                    c1: d.range.c1,
                }),
                id: None,
            });
        }
        for c in s.meta.charts.iter() {
            let a = &c.data.anchor;
            out.charts.push(NavItem {
                title: c.data.name.clone(),
                sheet: Some(s.id),
                range: Some(RangeDto {
                    r0: a.row,
                    c0: a.col,
                    r1: a.row,
                    c1: a.col,
                }),
                id: Some(c.id),
            });
        }
        for i in s.meta.images.iter() {
            out.images.push(NavItem {
                title: i.data.name.clone(),
                sheet: Some(s.id),
                range: Some(RangeDto {
                    r0: i.data.anchor.row,
                    c0: i.data.anchor.col,
                    r1: i.data.anchor.row,
                    c1: i.data.anchor.col,
                }),
                id: Some(i.id),
            });
        }
        for p in s.meta.pivots.iter() {
            let (r, c) = p.data.target;
            out.pivots.push(NavItem {
                title: p.data.name.clone(),
                sheet: Some(s.id),
                range: Some(p.data.output.map_or(
                    RangeDto {
                        r0: r,
                        c0: c,
                        r1: r,
                        c1: c,
                    },
                    |o| RangeDto {
                        r0: o.r0,
                        c0: o.c0,
                        r1: o.r1,
                        c1: o.c1,
                    },
                )),
                id: Some(p.id),
            });
        }
        for ((r, c), n) in s.meta.notes.iter().take(10_000) {
            out.notes.push(NavItem {
                title: excerpt(&n.text),
                sheet: Some(s.id),
                range: Some(RangeDto {
                    r0: *r,
                    c0: *c,
                    r1: *r,
                    c1: *c,
                }),
                id: None,
            });
        }
        for sc in s.meta.scenarios.iter() {
            out.scenarios.push(NavItem {
                title: sc.data.name.clone(),
                sheet: Some(s.id),
                range: sc.data.ranges.first().map(|r| RangeDto {
                    r0: r.r0,
                    c0: r.c0,
                    r1: r.r1,
                    c1: r.c1,
                }),
                id: Some(sc.id),
            });
        }
    }
    for (_, e) in wb.names.defined() {
        let Some(def) = &e.def else { continue };
        // A plain range reference is a jump target.
        let target = match &def.formula.expr {
            Expr::Ref(RefExpr::Cell(_) | RefExpr::Area(..)) => {
                let p = wb.precedents_of(&def.formula, def.base);
                (p.len() == 1).then(|| (p[0].sheet, range_dto(&p[0])))
            }
            _ => None,
        };
        out.names.push(NavItem {
            title: e.name.to_string(),
            sheet: target.map(|t| t.0).or(e.scope),
            range: target.map(|t| t.1),
            id: None,
        });
    }
    out.names.sort_by_key(|n| n.title.to_lowercase());
    Ok(out)
}

/// File ▸ Properties ▸ Statistics.
pub fn statistics(wb: &Workbook) -> DocumentStatsDto {
    DocumentStatsDto {
        sheets: wb.sheets.len() as u32,
        cells: wb.cell_count() as u64,
        formulas: wb.formulas.iter().count() as u64,
        pages: None,
        charts: wb.sheets.iter().map(|s| s.meta.charts.len() as u32).sum(),
        notes: wb.sheets.iter().map(|s| s.meta.notes.len() as u32).sum(),
        names: wb.names.len() as u32,
    }
}
