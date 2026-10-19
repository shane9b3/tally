//! Wires `tally-assist`'s [`RuleProvider`] into the engine's ⌘K hook
//! ([`EngineService::set_assist_provider`]).
//!
//! The engine and `tally-assist` each define a `SheetView`/`AssistRequest`
//! (neither crate depends on the other); [`EngineSheet`] adapts the engine's
//! view, and suggestions are mapped to the API DTO here.

use tally_api::{ActionArg, Suggestion};
use tally_assist::{AssistProvider as _, RuleProvider};
use tally_engine::service::{AssistRequest as EngineRequest, SheetView as EngineView};
use tally_engine::EngineService;
use tally_types::{CellAddr, CellRange, SheetId, Value};

/// The engine's sheet view seen through `tally_assist::SheetView`.
struct EngineSheet<'a>(EngineView<'a>);

impl tally_assist::SheetView for EngineSheet<'_> {
    fn value(&self, row: u32, col: u32) -> Value {
        self.0.value(row, col)
    }

    fn used(&self) -> Option<CellRange> {
        let sheet = self.0.sheet();
        self.0
            .used()
            .map(|r| CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1))
    }

    fn sheet(&self) -> SheetId {
        self.0.sheet()
    }
}

/// Maps one `tally-assist` suggestion to the API DTO.
pub fn to_dto(s: tally_assist::Suggestion) -> Suggestion {
    Suggestion {
        title: s.title,
        formula: s.formula,
        explanation: s.explanation,
        preview: s.preview,
        confidence: s.confidence,
        action: s.action,
        action_args: s
            .action_args
            .into_iter()
            .map(|(key, value)| ActionArg { key, value })
            .collect(),
    }
}

/// Runs the rule provider for one engine request.
pub fn suggest(rules: &RuleProvider, req: &EngineRequest<'_>) -> Vec<Suggestion> {
    let sheet = req.sheet.sheet();
    let view = EngineSheet(req.sheet);
    let active = CellAddr::new(sheet, req.selection.active.r, req.selection.active.c);
    let selection = req
        .selection
        .ranges
        .first()
        .map(|r| CellRange::new(sheet, r.r0, r.c0, r.r1, r.c1));
    let preview = |text: &str| req.preview(text);
    let request = tally_assist::AssistRequest {
        query: req.query,
        view: &view,
        active,
        selection,
        preview: Some(&preview),
    };
    rules.suggest(&request).into_iter().map(to_dto).collect()
}

/// Installs the rule-based provider on `service`.
pub fn install(service: &mut EngineService) {
    let rules = RuleProvider::new();
    service.set_assist_provider(Box::new(move |req| suggest(&rules, req)));
}

/// The engine with file IO and ⌘K assist, as the app runs it.
pub fn build_service() -> EngineService {
    let mut service = EngineService::with_default_codec();
    install(&mut service);
    service
}
