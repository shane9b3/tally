//! Rule-based formula assist for Tally (WP8).
//!
//! Offline natural-language-to-formula suggestions behind the command
//! palette. No network, no LLM: [`RuleProvider`] detects the data region
//! around the cursor ([`region`]), reads the query with a synonym lexicon
//! ([`lexicon`]) and builds validated CalcA1 en-US formulas ([`rules`]).
//! See `docs/architecture-plan.md` section 4.7.
//!
//! # Engine integration
//!
//! * Implement [`SheetView`] over the active sheet (`value` returns the
//!   cell's computed value, `used` the used range, `sheet` its id).
//! * The `preview` callback of [`AssistRequest`] evaluates a formula text
//!   (with the leading `=`, CalcA1 en-US dialect) in a sandbox at the active
//!   cell without committing, returning the resulting [`Value`].
//! * Convert to `tally_api::dto::Suggestion` in the engine (this crate does
//!   not depend on `tally-api`): `title`, `formula`, `explanation`,
//!   `preview`, `confidence` and `action` map one to one; `kind` is implied
//!   by `action.is_some()`; `action_args` (column letters, values) has no DTO
//!   field and is for the command that runs `action`.

pub mod lexicon;
pub mod region;
pub mod rules;

use tally_formula::SimpleResolver;
use tally_types::{CellAddr, CellRange, SheetId, Value};

/// Read-only view of a sheet, implemented by the engine.
pub trait SheetView {
    /// The computed value at 0-based `row`, `col` (`Value::Empty` if blank).
    fn value(&self, row: u32, col: u32) -> Value;
    /// The used range, or `None` for an empty sheet.
    fn used(&self) -> Option<CellRange>;
    /// The sheet this view reads.
    fn sheet(&self) -> SheetId;
}

/// A request for suggestions.
pub struct AssistRequest<'a> {
    /// The user's text.
    pub query: &'a str,
    /// The sheet to read.
    pub view: &'a dyn SheetView,
    /// The active cell (formulas are validated relative to it).
    pub active: CellAddr,
    /// The current selection, if any.
    pub selection: Option<CellRange>,
    /// Evaluates a formula text in a sandbox at `active`.
    #[allow(clippy::type_complexity)]
    pub preview: Option<&'a dyn Fn(&str) -> Option<Value>>,
}

/// What a suggestion does when accepted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    /// Insert `formula` into the active cell.
    InsertFormula,
    /// Run the command named by `action`.
    Command,
}

/// One suggestion (mirrors `tally_api::dto::Suggestion`).
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Title shown in the list.
    pub title: String,
    /// Formula to insert (CalcA1 en-US, `,` separators).
    pub formula: Option<String>,
    /// Why this suggestion fits.
    pub explanation: String,
    /// Formatted preview of the result.
    pub preview: Option<String>,
    /// 0..=1.
    pub confidence: f32,
    /// Command id such as `filter.byColumn`, `chart.byColumn`, `format.currency`.
    pub action: Option<String>,
    /// Formula or command.
    pub kind: SuggestionKind,
    /// Arguments for `action` (for example `column` = `G`, `value` = `Paid`).
    pub action_args: Vec<(String, String)>,
}

/// A source of suggestions.
pub trait AssistProvider: Send + Sync {
    /// Stable provider id.
    fn id(&self) -> &str;
    /// Suggestions for a request, best first.
    fn suggest(&self, req: &AssistRequest) -> Vec<Suggestion>;
}

/// The always-available offline provider.
#[derive(Debug)]
pub struct RuleProvider {
    resolver: SimpleResolver,
}

impl Default for RuleProvider {
    fn default() -> Self {
        Self {
            resolver: SimpleResolver::with_default_functions(),
        }
    }
}

impl RuleProvider {
    /// Creates the provider.
    pub fn new() -> Self {
        Self::default()
    }
}

impl AssistProvider for RuleProvider {
    fn id(&self) -> &str {
        "rules"
    }

    fn suggest(&self, req: &AssistRequest) -> Vec<Suggestion> {
        rules::suggest_rules(req, &self.resolver)
    }
}
