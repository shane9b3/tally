//! Calculation settings (Tools ▸ Options ▸ Calculate; wave 5, W5-3):
//! API ⇄ model conversion, the undoable `SetCalcSettings` op and the
//! "precision as shown" rounding used by the recalc.
//!
//! - **Iterations** (`CalcSettings::iterations`): see [`crate::recalc`];
//!   cyclic components are evaluated Gauss–Seidel until no member changes
//!   by more than `min_change`, else every member gets Err:523. With
//!   iterations off a cycle stays Err:522.
//! - **Precision as shown**: numbers are rounded to the decimals their
//!   cell's number format displays, both when a formula reads them and
//!   when a formula result is stored; `General` is not rounded and percent
//!   formats add two decimals (`tally_numfmt::display_decimals`).
//! - The op is undoable through [`Change::Settings`] and followed by a hard
//!   recalc (every formula is marked dirty).

use tally_api::{
    ApiError, CalcSettingsDto, CriteriaModeDto, DateEpochDto, IterationsDto, TextToNumberDto,
};
use tally_types::{CalcSettings, CriteriaMode, DateEpoch, IterSettings, TextToNumber};

use crate::undo::{Change, Effects};
use crate::workbook::Workbook;

/// Most iteration steps accepted (AOO's dialog allows up to 32767).
pub const MAX_ITERATION_STEPS: u32 = 32_767;

/// Model → API.
pub fn to_api(s: &CalcSettings) -> CalcSettingsDto {
    CalcSettingsDto {
        iterations: s.iterations.map(|i| IterationsDto {
            steps: i.steps,
            min_change: i.min_change,
        }),
        date_epoch: match s.date_epoch {
            DateEpoch::D1899 => DateEpochDto::D1899,
            DateEpoch::D1900 => DateEpochDto::D1900,
            DateEpoch::D1904 => DateEpochDto::D1904,
        },
        case_sensitive: s.case_sensitive,
        precision_as_shown: s.precision_as_shown,
        whole_cell_match: s.whole_cell_match,
        criteria_mode: match s.criteria_mode {
            CriteriaMode::Wildcards => CriteriaModeDto::Wildcards,
            CriteriaMode::Regex => CriteriaModeDto::Regex,
            CriteriaMode::Literal => CriteriaModeDto::Literal,
        },
        text_to_number: match s.text_to_number {
            TextToNumber::GenerateError => TextToNumberDto::GenerateError,
            TextToNumber::TreatAsZero => TextToNumberDto::TreatAsZero,
            TextToNumber::ConvertUnambiguous => TextToNumberDto::ConvertUnambiguous,
            TextToNumber::ConvertLocale => TextToNumberDto::ConvertLocale,
        },
    }
}

/// API → model, validated.
pub fn from_api(d: &CalcSettingsDto) -> Result<CalcSettings, ApiError> {
    let iterations = match d.iterations {
        None => None,
        Some(i) => {
            if i.steps == 0 || i.steps > MAX_ITERATION_STEPS {
                return Err(ApiError::invalid_argument(format!(
                    "iteration steps must be 1–{MAX_ITERATION_STEPS}"
                )));
            }
            if !i.min_change.is_finite() || i.min_change < 0.0 {
                return Err(ApiError::invalid_argument(
                    "the minimum change must be a number ≥ 0",
                ));
            }
            Some(IterSettings {
                steps: i.steps,
                min_change: i.min_change,
            })
        }
    };
    Ok(CalcSettings {
        date_epoch: match d.date_epoch {
            DateEpochDto::D1899 => DateEpoch::D1899,
            DateEpochDto::D1900 => DateEpoch::D1900,
            DateEpochDto::D1904 => DateEpoch::D1904,
        },
        text_to_number: match d.text_to_number {
            TextToNumberDto::GenerateError => TextToNumber::GenerateError,
            TextToNumberDto::TreatAsZero => TextToNumber::TreatAsZero,
            TextToNumberDto::ConvertUnambiguous => TextToNumber::ConvertUnambiguous,
            TextToNumberDto::ConvertLocale => TextToNumber::ConvertLocale,
        },
        criteria_mode: match d.criteria_mode {
            CriteriaModeDto::Wildcards => CriteriaMode::Wildcards,
            CriteriaModeDto::Regex => CriteriaMode::Regex,
            CriteriaModeDto::Literal => CriteriaMode::Literal,
        },
        case_sensitive: d.case_sensitive,
        whole_cell_match: d.whole_cell_match,
        iterations,
        precision_as_shown: d.precision_as_shown,
    })
}

/// `Op::SetCalcSettings`: replaces the settings (undoable) and marks every
/// formula dirty so the transaction's recalc is a hard recalc.
pub fn set_calc_settings(
    wb: &mut Workbook,
    settings: &CalcSettingsDto,
    fx: &mut Effects,
) -> Result<Vec<Change>, ApiError> {
    let new = from_api(settings)?;
    if new == wb.settings {
        return Ok(Vec::new());
    }
    let old = std::mem::replace(&mut wb.settings, new);
    wb.dirty_all();
    // Dates and displayed values may change everywhere.
    let sheets: Vec<tally_types::SheetId> = wb.sheets.iter().map(|s| s.id).collect();
    for s in sheets {
        if let Some(r) = wb.sheet(s).and_then(|sh| sh.used_range()) {
            wb.pending.display_only.push(r);
        }
        fx.layout(s);
    }
    Ok(vec![Change::Settings(Box::new(old))])
}

/// Rounds `x` to `decimals` places (negative: to tens, hundreds, …), as
/// AOO's `rtl::math::round` does for "precision as shown". Values beyond
/// 2⁵³ are left alone (they have no fraction anyway).
pub fn round_to_decimals(x: f64, decimals: i32) -> f64 {
    if !x.is_finite() || x.abs() >= 9.007_199_254_740_992e15 {
        return x;
    }
    let d = decimals.clamp(-15, 15);
    let f = 10f64.powi(d);
    let scaled = x * f;
    if !scaled.is_finite() {
        return x;
    }
    // Round half away from zero, with a small nudge against binary
    // representation (0.285 * 100 = 28.499999…).
    let r = (scaled + scaled.signum() * 1e-9).round() / f;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding() {
        assert_eq!(round_to_decimals(0.333, 2), 0.33);
        assert_eq!(round_to_decimals(0.285, 2), 0.29);
        assert_eq!(round_to_decimals(-0.285, 2), -0.29);
        assert_eq!(round_to_decimals(1234.0, -3), 1000.0);
        assert_eq!(round_to_decimals(1500.0, -3), 2000.0);
        assert_eq!(round_to_decimals(2.5, 0), 3.0);
        assert_eq!(round_to_decimals(1e300, 2), 1e300);
    }

    #[test]
    fn api_round_trip_and_validation() {
        let s = CalcSettings {
            iterations: Some(IterSettings {
                steps: 50,
                min_change: 0.01,
            }),
            precision_as_shown: true,
            ..CalcSettings::default()
        };
        assert_eq!(from_api(&to_api(&s)).unwrap(), s);
        let mut bad = to_api(&s);
        bad.iterations = Some(IterationsDto {
            steps: 0,
            min_change: 0.1,
        });
        assert!(from_api(&bad).is_err());
        bad.iterations = Some(IterationsDto {
            steps: 10,
            min_change: f64::NAN,
        });
        assert!(from_api(&bad).is_err());
    }
}
