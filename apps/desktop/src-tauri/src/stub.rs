//! `StubService`: a temporary [`DocumentService`] so the shell runs before
//! the engine (WP1) exists. It creates empty workbooks and answers layout,
//! cell and style queries with defaults; every other method returns
//! `ApiError::unsupported`. Swapped out in one line in `lib.rs`.

use std::collections::HashMap;
use std::path::Path;

use tally_api::{
    ApiError, ApplyResult, BordersDto, CellBlock, CellEditInfo, CellPos, ClipboardOut, Direction,
    DocId, DocumentService, FileFormat, FunctionInfo, HAlign, RangeDto, Result, SelectionDto,
    SelectionStats, SheetInfo, SheetLayout, StyleDto, Suggestion, Transaction, VAlign,
    WorkbookInfo,
};
use tally_types::SheetId;

const SHEET: SheetId = 1;

struct StubDoc {
    title: String,
}

/// See the module docs.
#[derive(Default)]
pub struct StubService {
    next_doc: u32,
    docs: HashMap<u32, StubDoc>,
}

impl StubService {
    /// An empty service.
    pub fn new() -> Self {
        Self::default()
    }

    fn doc(&self, doc: DocId) -> Result<&StubDoc> {
        self.docs
            .get(&doc.0)
            .ok_or_else(|| ApiError::not_found(format!("no document {}", doc.0)))
    }

    fn check_sheet(&self, doc: DocId, sheet: SheetId) -> Result<()> {
        self.doc(doc)?;
        if sheet == SHEET {
            Ok(())
        } else {
            Err(ApiError::not_found(format!("no sheet {sheet}")))
        }
    }

    fn info(&self, doc: DocId) -> Result<WorkbookInfo> {
        let d = self.doc(doc)?;
        Ok(WorkbookInfo {
            doc,
            title: d.title.clone(),
            path: None,
            sheets: vec![SheetInfo {
                id: SHEET,
                name: "Sheet1".into(),
                tab_color: None,
                hidden: false,
            }],
            active_sheet: SHEET,
            dirty: false,
            version: 0,
        })
    }
}

fn unsupported(what: &str) -> ApiError {
    ApiError::unsupported(format!(
        "{what} is not available yet: the engine (WP1) is not integrated"
    ))
}

fn default_style(id: u32) -> StyleDto {
    StyleDto {
        id,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        font_family: None,
        font_size_pt: 10.0,
        color: None,
        fill: None,
        h_align: HAlign::General,
        v_align: VAlign::Bottom,
        wrap: false,
        indent: 0,
        borders: BordersDto::default(),
        number_format: "General".into(),
        display: None,
    }
}

impl DocumentService for StubService {
    fn new_workbook(&mut self, template: Option<&str>) -> Result<WorkbookInfo> {
        self.next_doc += 1;
        let id = self.next_doc;
        let title = template
            .and_then(|t| Path::new(t).file_stem().and_then(|s| s.to_str()))
            .map_or_else(|| format!("Untitled {id}"), str::to_owned);
        self.docs.insert(id, StubDoc { title });
        self.info(DocId(id))
    }

    fn open(&mut self, _path: &Path) -> Result<WorkbookInfo> {
        Err(unsupported("Opening files"))
    }

    fn save(
        &mut self,
        _doc: DocId,
        _path: Option<&Path>,
        _format: FileFormat,
    ) -> Result<WorkbookInfo> {
        Err(unsupported("Saving"))
    }

    fn close(&mut self, doc: DocId) -> Result<()> {
        self.docs
            .remove(&doc.0)
            .map(|_| ())
            .ok_or_else(|| ApiError::not_found(format!("no document {}", doc.0)))
    }

    fn sheet_layout(&self, doc: DocId, sheet: SheetId) -> Result<SheetLayout> {
        self.check_sheet(doc, sheet)?;
        Ok(SheetLayout {
            sheet,
            used: None,
            default_col_width_px: 100.0,
            default_row_height_px: 24.0,
            col_runs: Vec::new(),
            row_runs: Vec::new(),
            frozen: None,
            merges: Vec::new(),
        })
    }

    fn cells(&self, doc: DocId, sheet: SheetId, ranges: &[RangeDto]) -> Result<Vec<CellBlock>> {
        self.check_sheet(doc, sheet)?;
        Ok(ranges
            .iter()
            .map(|range| CellBlock {
                sheet,
                range: *range,
                version: 0,
                cells: Vec::new(),
            })
            .collect())
    }

    fn styles(&self, doc: DocId, ids: &[u32]) -> Result<Vec<StyleDto>> {
        self.doc(doc)?;
        Ok(ids.iter().map(|&id| default_style(id)).collect())
    }

    fn cell_edit_info(
        &self,
        _doc: DocId,
        _sheet: SheetId,
        _r: u32,
        _c: u32,
    ) -> Result<CellEditInfo> {
        Err(unsupported("Cell editing"))
    }

    fn apply(&mut self, _doc: DocId, _tx: Transaction) -> Result<ApplyResult> {
        Err(unsupported("Editing"))
    }

    fn undo(&mut self, _doc: DocId) -> Result<ApplyResult> {
        Err(unsupported("Undo"))
    }

    fn redo(&mut self, _doc: DocId) -> Result<ApplyResult> {
        Err(unsupported("Redo"))
    }

    fn selection_stats(&self, _doc: DocId, _sel: &SelectionDto) -> Result<SelectionStats> {
        Err(unsupported("Selection statistics"))
    }

    fn copy(&mut self, _doc: DocId, _sel: &SelectionDto) -> Result<ClipboardOut> {
        Err(unsupported("Copy"))
    }

    fn navigate_edge(
        &self,
        _doc: DocId,
        _sheet: SheetId,
        _from: CellPos,
        _dir: Direction,
    ) -> Result<CellPos> {
        Err(unsupported("Edge navigation"))
    }

    fn resolve_name_box(&self, _doc: DocId, _text: &str) -> Result<SelectionDto> {
        Err(unsupported("The name box"))
    }

    fn list_functions(&self) -> Vec<FunctionInfo> {
        Vec::new()
    }

    fn assist_suggest(
        &self,
        _doc: DocId,
        _query: &str,
        _sel: &SelectionDto,
    ) -> Result<Vec<Suggestion>> {
        Err(unsupported("Assist"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_workbook_and_queries() -> Result<()> {
        let mut s = StubService::new();
        let info = s.new_workbook(None)?;
        assert_eq!(info.title, "Untitled 1");
        let layout = s.sheet_layout(info.doc, info.active_sheet)?;
        assert!((layout.default_col_width_px - 100.0).abs() < f32::EPSILON);
        let blocks = s.cells(
            info.doc,
            info.active_sheet,
            &[RangeDto {
                r0: 0,
                c0: 0,
                r1: 63,
                c1: 31,
            }],
        )?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(s.styles(info.doc, &[0, 3])?.len(), 2);
        assert_eq!(
            s.undo(info.doc).map_err(|e| e.code),
            Err(ApiError::UNSUPPORTED.to_string())
        );
        s.close(info.doc)?;
        assert!(s.close(info.doc).is_err());
        Ok(())
    }

    #[test]
    fn template_title() -> Result<()> {
        let mut s = StubService::new();
        assert_eq!(
            s.new_workbook(Some("/x/templates/budget.csv"))?.title,
            "budget"
        );
        Ok(())
    }
}
