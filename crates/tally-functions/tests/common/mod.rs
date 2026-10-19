//! Shared helpers for the integration tests.
#![allow(dead_code, unused_imports)]

use std::sync::Arc;

pub use tally_functions::mock::{call, call_raw, call_with, MockArgs, MockEvalContext};
pub use tally_functions::EvalValue;
pub use tally_types::{ErrorKind, Matrix, Value};

pub fn n(x: f64) -> EvalValue {
    EvalValue::number(x)
}
pub fn t(s: &str) -> EvalValue {
    EvalValue::text(s)
}
pub fn b(x: bool) -> EvalValue {
    EvalValue::bool(x)
}
pub fn e(k: ErrorKind) -> EvalValue {
    EvalValue::error(k)
}
pub fn empty() -> EvalValue {
    EvalValue::Scalar(Value::Empty)
}
pub fn missing() -> EvalValue {
    EvalValue::Missing
}
/// A row-major inline array.
pub fn arr(rows: u32, cols: u32, data: Vec<Value>) -> EvalValue {
    EvalValue::Array(Arc::new(Matrix::from_vec(rows, cols, data).unwrap()))
}
pub fn err(k: ErrorKind) -> Value {
    Value::Error(k)
}
pub fn num(x: f64) -> Value {
    Value::Number(x)
}
pub fn txt(s: &str) -> Value {
    Value::text(s)
}
/// A reference on sheet 0, e.g. `rg("A1:B3")`.
pub fn rg(a1: &str) -> EvalValue {
    EvalValue::Range(tally_types::CellRange::from_a1(0, a1).unwrap())
}
