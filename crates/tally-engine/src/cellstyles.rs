//! Named cell styles (wave 5, W5-2 / W5-6D): `Default`, `Result`,
//! `Result2`, `Heading`, `Heading1` plus user styles, each with a parent
//! chain. A cell's `StyleData::style_name` names the style it is based
//! on; direct formatting sits on top. Redefining a style re-interns every
//! cell style based on it.
//!
//! Storage: `Workbook::cell_styles` (`tally_model::CellStyleData`, fully
//! resolved attributes). ODS `office:styles` with
//! `style:parent-style-name`; XLSX `cellStyleXfs`/`cellStyles`.
//!
//! TODO(W5-2): `define` (resolve through the parent chain, re-intern
//! dependants, `Change::CellStyles` undo), `delete` (fall back to the
//! parent), `apply` (clear direct formatting, set `style_name`),
//! `to_api`, the built-in table, and `STYLE()` support through
//! `EvalContext::request_style`.

/// The built-in style names, in Styles panel order.
pub const BUILTIN: [&str; 5] = ["Default", "Result", "Result2", "Heading", "Heading1"];

/// `true` for a built-in style name (case-sensitive, as AOO).
pub fn is_builtin(name: &str) -> bool {
    BUILTIN.contains(&name)
}
