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
//! - v0.3: [`condfmt`] (conditional formatting), [`validation`],
//!   [`annotations`] (notes, links), [`protection`], [`pagesetup`] (page
//!   setup, document properties), [`autofit`] (row heights), [`goalseek`],
//!   [`fexpr`] (rule formulas) and [`ops_v03`] (their ops);
//! - [`document`]: one open document (workbook + undo + version);
//! - [`service`]: `EngineService`, the `DocumentService` implementation;
//! - wave 5 (v0.4–v0.6, W5-0a skeletons): [`charts`] and [`ops_v04`]
//!   (charts, pictures), [`pivot`], [`outline`], [`scenarios`],
//!   [`cellstyles`], [`tools`] (data tools, assists, solver) and
//!   [`ops_v05`] (their ops), [`print`] (pagination);
//! - W5-3: [`calcsettings`] (iterations, precision as shown, the
//!   `SetCalcSettings` op), [`detective`] (precedents/dependents),
//!   [`external`] (external-link policy), [`solver`] (linear solver),
//!   [`print`] (pagination and print jobs through `tally-io-pdf`).
//!
//! See `docs/architecture-plan.md` §3.2, §3.6, §4.3, §4.4 and §7 WP1.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod annotations;
pub mod autocomplete;
pub mod autofit;
pub mod calcsettings;
pub mod cellstyles;
pub mod charts;
pub mod clipboard;
pub mod condfmt;
pub mod convert;
pub mod deps;
pub mod detective;
pub mod document;
pub mod eval;
pub mod external;
pub mod fexpr;
pub mod fill;
pub mod filter;
pub mod find;
pub mod fonts;
pub mod formula_cells;
pub mod goalseek;
pub mod images;
pub mod moves;
pub mod names;
pub mod navigator;
pub mod ops;
pub mod ops_v03;
pub mod ops_v04;
pub mod ops_v05;
pub mod outline;
pub mod pagesetup;
pub mod pivot;
pub mod print;
pub mod protection;
pub mod r1c1;
pub mod rangeindex;
pub mod recalc;
pub mod scenarios;
pub mod service;
pub mod solver;
pub mod sort;
pub mod storage;
pub mod structure;
pub mod styles;
pub mod tools;
pub mod undo;
pub mod units;
pub mod validation;
pub mod workbook;

pub use document::Document;
#[cfg(feature = "io")]
pub use service::IoCodec;
pub use service::{EngineService, FileCodec, UnsupportedCodec};
pub use workbook::Workbook;
