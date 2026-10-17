//! Buffered result rows.

use rupa_core::{RowCursor, RowError, SqlType, Value};
use sqlx::Row as _;

use crate::Backend;

struct SqlxRow<DB: Backend>(DB::Row);

impl<DB: Backend> rupa_core::Row for SqlxRow<DB> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        let len = self.0.len();
        if index >= len {
            return Err(RowError::IndexOutOfRange { index, len });
        }
        DB::read(&self.0, index, ty)
    }
}

/// Cursor over rows fetched in full.
pub(crate) struct SqlxRows<DB: Backend> {
    rows: std::vec::IntoIter<DB::Row>,
    current: Option<SqlxRow<DB>>,
}

impl<DB: Backend> SqlxRows<DB> {
    pub(crate) fn new(rows: Vec<DB::Row>) -> Self {
        Self {
            rows: rows.into_iter(),
            current: None,
        }
    }
}

impl<DB: Backend> RowCursor for SqlxRows<DB> {
    fn next_row(&mut self) -> Option<Result<&dyn rupa_core::Row, RowError>> {
        self.current = Some(SqlxRow(self.rows.next()?));
        self.current.as_ref().map(|r| Ok(r as &dyn rupa_core::Row))
    }
}

/// A sqlx decoding error as a [`RowError`].
pub(crate) fn driver(index: usize) -> impl Fn(sqlx::Error) -> RowError {
    move |e| RowError::Driver(format!("column {index}: {e}"))
}

/// An integer read from column `index`, as the integer type `want`
/// (narrowed if it fits).
pub(crate) fn int(n: i64, want: SqlType, index: usize) -> Result<Value, RowError> {
    let out_of_range =
        || RowError::Driver(format!("value {n} in column {index} does not fit {want:?}"));
    Ok(match want {
        SqlType::I16 => Value::I16(n.try_into().map_err(|_| out_of_range())?),
        SqlType::I32 => Value::I32(n.try_into().map_err(|_| out_of_range())?),
        _ => Value::I64(n),
    })
}
