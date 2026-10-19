//! Calculation engine for Tally (WP1).
//!
//! Module map:
//! - [`storage`]: `ColumnStore` (BTreeMap per column), style runs, dimension runs, `Sheet`;
//! - [`styles`]: the deduplicating `StyleTable` with parsed number formats;
//! - [`formula_cells`]: the slot map of formula cells;
//! - [`deps`]: the reverse dependency index (`DepIndex`) and rectangle coalescing;
//! - [`eval`]: the evaluator (operators, implicit intersection, array context);
//! - [`recalc`]: dirty propagation, topological order with cycle detection,
//!   and the engine's `EvalContext`;
//! - [`workbook`]: the `Workbook` and its cell mutators;
//! - [`structure`]: inserting and deleting rows and columns;
//! - [`convert`]: `Workbook::from_data` / `to_data` (the `tally-model` interchange);
//! - [`ops`] and [`undo`]: transactions, inverse snapshots, the undo stack;
//! - [`clipboard`]: internal copy snapshots, TSV/HTML export, paste and
//!   paste special;
//! - [`names`]: named ranges and expressions (table, expansion, validation);
//! - [`filter`]: AutoFilter criteria and filtered rows;
//! - [`sort`], [`fill`], [`find`], [`moves`]: sort, fill series, find &
//!   replace, cut/move;
//! - [`document`]: one open document (workbook + undo + version);
//! - [`service`]: `EngineService`, the `DocumentService` implementation.
//!
//! See `docs/architecture-plan.md` §3.2, §3.6, §4.3, §4.4 and §7 WP1.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clipboard;
pub mod convert;
pub mod deps;
pub mod document;
pub mod eval;
pub mod fill;
pub mod filter;
pub mod find;
pub mod formula_cells;
pub mod moves;
pub mod names;
pub mod ops;
pub mod recalc;
pub mod service;
pub mod sort;
pub mod storage;
pub mod structure;
pub mod styles;
pub mod undo;
pub mod units;
pub mod workbook;

pub use document::Document;
#[cfg(feature = "io")]
pub use service::IoCodec;
pub use service::{EngineService, FileCodec, UnsupportedCodec};
pub use workbook::Workbook;
