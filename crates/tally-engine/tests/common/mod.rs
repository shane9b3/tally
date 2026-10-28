//! Shared helpers for the integration tests.
#![allow(dead_code, clippy::unwrap_used)]

use tally_api::{
    ApplyResult, CellPos, DocId, DocumentService, Op, RangeDto, SelectionDto, Transaction,
};
use tally_engine::EngineService;
use tally_types::{CellAddr, SheetId, Value};

pub fn tx(ops: Vec<Op>) -> Transaction {
    Transaction {
        ops,
        label: Some("test".into()),
        selection_before: None,
        selection_after: None,
        group: None,
    }
}

pub fn range(a1: &str) -> RangeDto {
    let r = tally_types::CellRange::from_a1(0, a1).unwrap_or_else(|| panic!("bad range {a1}"));
    RangeDto {
        r0: r.r0,
        c0: r.c0,
        r1: r.r1,
        c1: r.c1,
    }
}

pub fn sel(sheet: SheetId, a1: &str) -> SelectionDto {
    let r = range(a1);
    SelectionDto {
        sheet,
        ranges: vec![r],
        active: CellPos { r: r.r0, c: r.c0 },
    }
}

pub fn input(sheet: SheetId, a1: &str, text: &str) -> Op {
    let a = CellAddr::from_a1(sheet, a1).unwrap_or_else(|| panic!("bad cell {a1}"));
    Op::SetCellInput {
        sheet,
        r: a.row,
        c: a.col,
        input: text.into(),
    }
}

pub struct Fixture {
    pub svc: EngineService,
    pub doc: DocId,
    pub sheet: SheetId,
}

impl Fixture {
    pub fn new() -> Self {
        let mut svc = EngineService::new();
        let info = svc.new_workbook(None).unwrap();
        let sheet = info.sheets[0].id;
        Self {
            svc,
            doc: info.doc,
            sheet,
        }
    }

    pub fn apply(&mut self, ops: Vec<Op>) -> ApplyResult {
        self.svc.apply(self.doc, tx(ops)).unwrap()
    }

    pub fn set(&mut self, a1: &str, text: &str) -> ApplyResult {
        let op = input(self.sheet, a1, text);
        self.apply(vec![op])
    }

    pub fn value(&self, a1: &str) -> Value {
        self.value_on(self.sheet, a1)
    }

    pub fn value_on(&self, sheet: SheetId, a1: &str) -> Value {
        let a = CellAddr::from_a1(sheet, a1).unwrap();
        self.svc.document(self.doc).unwrap().wb.value(a)
    }

    pub fn text(&self, a1: &str) -> String {
        let a = CellAddr::from_a1(self.sheet, a1).unwrap();
        let blocks = self
            .svc
            .cells(
                self.doc,
                self.sheet,
                &[RangeDto {
                    r0: a.row,
                    c0: a.col,
                    r1: a.row,
                    c1: a.col,
                }],
            )
            .unwrap();
        blocks[0]
            .cells
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default()
    }

    pub fn data(&self) -> tally_model::WorkbookData {
        self.svc.document(self.doc).unwrap().wb.to_data()
    }
}
