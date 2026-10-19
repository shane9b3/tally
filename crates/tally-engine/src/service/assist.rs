//! The ⌘K assist hook. The engine has no assist logic of its own; the shell
//! (or a later patch) installs a provider, e.g. one wrapping `tally-assist`,
//! with [`super::EngineService::set_assist_provider`]. Providers read the
//! sheet through [`SheetView`].

use tally_api::{DocId, RangeDto, SelectionDto, Suggestion};
use tally_types::{CellAddr, SheetId, Value};

use crate::document::range_dto;
use crate::workbook::Workbook;

/// Read-only view of one sheet for assist providers.
#[derive(Clone, Copy)]
pub struct SheetView<'a> {
    wb: &'a Workbook,
    sheet: SheetId,
}

impl<'a> SheetView<'a> {
    /// A view of `sheet`.
    pub fn new(wb: &'a Workbook, sheet: SheetId) -> Self {
        Self { wb, sheet }
    }

    /// The sheet id.
    pub fn sheet(&self) -> SheetId {
        self.sheet
    }

    /// The value at (`r`, `c`) (formula results for formula cells).
    pub fn value(&self, r: u32, c: u32) -> Value {
        self.wb.value(CellAddr::new(self.sheet, r, c))
    }

    /// The display text at (`r`, `c`).
    pub fn text(&self, r: u32, c: u32) -> String {
        super::view::display_text(self.wb, CellAddr::new(self.sheet, r, c))
    }

    /// The number format code at (`r`, `c`).
    pub fn number_format(&self, r: u32, c: u32) -> String {
        let st = self.wb.style_at(CellAddr::new(self.sheet, r, c));
        self.wb.styles.get(st).format_code.to_string()
    }

    /// Bounding box of non-empty cells.
    pub fn used(&self) -> Option<RangeDto> {
        self.wb
            .sheet(self.sheet)
            .and_then(|s| s.used_range())
            .map(|r| range_dto(&r))
    }

    /// The sheet name.
    pub fn name(&self) -> String {
        self.wb
            .sheet(self.sheet)
            .map_or_else(String::new, |s| s.name.to_string())
    }
}

/// One ⌘K request.
pub struct AssistRequest<'a> {
    /// The document.
    pub doc: DocId,
    /// What the user typed.
    pub query: &'a str,
    /// The current selection.
    pub selection: &'a SelectionDto,
    /// The selection's sheet.
    pub sheet: SheetView<'a>,
}

/// An assist provider.
pub type AssistProvider = Box<dyn Fn(&AssistRequest<'_>) -> Vec<Suggestion> + Send + Sync>;
