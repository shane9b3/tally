//! Cell values and matrices.

use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ErrorKind;

/// A cell or formula result value.
///
/// Numbers are `f64` as in OpenOffice. `Bool` stays distinct from numbers
/// (ODF and OOXML both store booleans) but coerces to 1/0 in arithmetic.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Value {
    /// An empty cell.
    #[default]
    Empty,
    /// A number (dates and times are serial numbers).
    Number(f64),
    /// Text.
    Text(Arc<str>),
    /// A boolean.
    Bool(bool),
    /// An error value.
    Error(ErrorKind),
    /// An array result (array formulas, inline arrays).
    Array(Arc<Matrix>),
}

impl Value {
    /// Convenience constructor for text.
    pub fn text(s: impl Into<Arc<str>>) -> Self {
        Value::Text(s.into())
    }

    /// Returns `true` for [`Value::Empty`].
    pub const fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// Returns the error kind if this is an error value.
    pub const fn as_error(&self) -> Option<ErrorKind> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// Returns the number if this is a number value (no coercion).
    pub const fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.into())
    }
}

impl From<ErrorKind> for Value {
    fn from(e: ErrorKind) -> Self {
        Value::Error(e)
    }
}

/// A row-major 2-D array of values.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Matrix {
    /// Number of rows.
    pub rows: u32,
    /// Number of columns.
    pub cols: u32,
    /// `rows * cols` values, row by row.
    pub data: Vec<Value>,
}

impl Matrix {
    /// Creates a `rows × cols` matrix with every element set to `fill`.
    pub fn new_filled(rows: u32, cols: u32, fill: Value) -> Self {
        let len = rows as usize * cols as usize;
        Self {
            rows,
            cols,
            data: vec![fill; len],
        }
    }

    /// Wraps row-major `data`; returns `None` if its length is not `rows * cols`.
    pub fn from_vec(rows: u32, cols: u32, data: Vec<Value>) -> Option<Self> {
        (data.len() == rows as usize * cols as usize).then_some(Self { rows, cols, data })
    }

    fn index(&self, row: u32, col: u32) -> Option<usize> {
        (row < self.rows && col < self.cols)
            .then(|| row as usize * self.cols as usize + col as usize)
    }

    /// The element at (`row`, `col`), or `None` if out of bounds.
    pub fn get(&self, row: u32, col: u32) -> Option<&Value> {
        self.index(row, col).and_then(|i| self.data.get(i))
    }

    /// Mutable access to the element at (`row`, `col`).
    pub fn get_mut(&mut self, row: u32, col: u32) -> Option<&mut Value> {
        self.index(row, col).and_then(|i| self.data.get_mut(i))
    }

    /// Sets an element; returns `false` (and does nothing) if out of bounds.
    pub fn set(&mut self, row: u32, col: u32, value: Value) -> bool {
        match self.get_mut(row, col) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_helpers() {
        assert!(Value::default().is_empty());
        assert_eq!(Value::from(2.0).as_number(), Some(2.0));
        assert_eq!(Value::from("x"), Value::text("x"));
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(ErrorKind::NA).as_error(), Some(ErrorKind::NA));
        assert_eq!(Value::text("1").as_number(), None);
        assert_eq!(Value::Number(1.0).as_error(), None);
    }

    #[test]
    fn matrix_basics() {
        let mut m = Matrix::new_filled(2, 3, Value::Number(0.0));
        assert_eq!(m.data.len(), 6);
        assert!(m.set(1, 2, Value::text("z")));
        assert!(!m.set(2, 0, Value::Empty));
        assert_eq!(m.get(1, 2), Some(&Value::text("z")));
        assert_eq!(m.data[5], Value::text("z"), "row-major");
        assert_eq!(m.get(0, 3), None);
        assert_eq!(m.get(2, 0), None);
        if let Some(v) = m.get_mut(0, 0) {
            *v = Value::Bool(true);
        }
        assert_eq!(m.get(0, 0), Some(&Value::Bool(true)));
        assert!(Matrix::from_vec(2, 2, vec![Value::Empty; 3]).is_none());
        let m2 = Matrix::from_vec(1, 2, vec![1.0.into(), 2.0.into()]).unwrap();
        assert_eq!(m2.get(0, 1), Some(&Value::Number(2.0)));
        assert_eq!(Matrix::new_filled(0, 5, Value::Empty).data.len(), 0);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let m =
            Matrix::from_vec(1, 2, vec![Value::text("a"), Value::Error(ErrorKind::Div0)]).unwrap();
        let values = vec![
            Value::Empty,
            Value::Number(1.5),
            Value::text("hi"),
            Value::Bool(false),
            Value::Error(ErrorKind::Ref),
            Value::Array(Arc::new(m)),
        ];
        let json = serde_json::to_string(&values).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Value>>(&json).unwrap(), values);
    }
}
