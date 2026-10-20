//! The v0.4 ops: charts and pictures (wave 5, W5-1). Dispatch pattern as
//! `ops_v03`: every op records the sheet meta it replaces through
//! `with_meta` (`Change::SheetMeta`) and reports its effects.
//!
//! W5-0a: every op is recognised and refused with `OpError { code:
//! "unsupported" }`, so the UI and the shell can wire the commands
//! before the behaviour exists.
//!
//! TODO(W5-1): implement `AddChart` (derive series from `source` when
//! `series` is empty; id through `meta.alloc_id()`, reported in
//! `fx.created`), `UpdateChart`, `MoveResizeChart`, `DeleteChart`,
//! `MoveChartToSheet`, `AddImage`, `DeleteImage`, `MoveResizeImage`; all
//! refused on a protected sheet; set `fx.objects_changed`.

use tally_api::{ApiError, Op};

use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Applies a v0.4 op; `None` for the other ops.
pub fn apply(
    wb: &mut Workbook,
    op: &Op,
    fx: &mut Effects,
) -> Option<Result<Vec<Change>, ApiError>> {
    let _ = (wb, fx);
    let what = match op {
        Op::AddChart { .. }
        | Op::UpdateChart { .. }
        | Op::MoveResizeChart { .. }
        | Op::DeleteChart { .. }
        | Op::MoveChartToSheet { .. } => "charts",
        Op::AddImage { .. } | Op::DeleteImage { .. } | Op::MoveResizeImage { .. } => "pictures",
        _ => return None,
    };
    Some(Err(unsupported(what)))
}

/// The refusal every stubbed op returns.
pub(crate) fn unsupported(what: &str) -> ApiError {
    ApiError::unsupported(format!("{what} are coming in wave 5"))
}
