//! Sans-IO row access. Drivers adapt their native rows to [`Row`] and their
//! result sets to [`RowCursor`]; decoding into Rust types happens here.

use crate::error::RowError;
use crate::value::{SqlType, Value};

/// One result row, accessed by position. `ty` is the expected SQL type,
/// taken from the projection; drivers use it to pick a native decoder.
pub trait Row {
    fn len(&self) -> usize;
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A lending iterator over result rows.
pub trait RowCursor {
    fn next_row(&mut self) -> Option<Result<&dyn Row, RowError>>;
}

/// Rows that are already [`Value`]s, e.g. from the memory backend or a
/// driver that buffers. The type hint is ignored.
impl Row for [Value] {
    fn len(&self) -> usize {
        <[Value]>::len(self)
    }
    fn get(&self, index: usize, _ty: SqlType) -> Result<Value, RowError> {
        <[Value]>::get(self, index)
            .cloned()
            .ok_or(RowError::IndexOutOfRange {
                index,
                len: <[Value]>::len(self),
            })
    }
}

impl Row for Vec<Value> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        Row::get(self.as_slice(), index, ty)
    }
}

/// Cursor over buffered value rows.
#[derive(Debug, Clone, Default)]
pub struct ValueRows {
    rows: std::vec::IntoIter<Vec<Value>>,
    current: Option<Vec<Value>>,
}

impl ValueRows {
    pub fn new(rows: Vec<Vec<Value>>) -> Self {
        Self {
            rows: rows.into_iter(),
            current: None,
        }
    }
}

impl RowCursor for ValueRows {
    fn next_row(&mut self) -> Option<Result<&dyn Row, RowError>> {
        self.current = Some(self.rows.next()?);
        self.current.as_ref().map(|r| Ok(r as &dyn Row))
    }
}
