//! Decoding joined rows: tuples of entities.

use std::fmt;

use crate::entity::{Entity, FromRow};
use crate::error::{ResultError, RowError};
use crate::row::Row;
use crate::value::{SqlType, Value};

/// A view of `row` starting at column `base`, so an entity's `from_row` (which
/// reads its columns from 0) can decode its part of a joined row.
#[derive(Clone, Copy)]
pub struct OffsetRow<'a> {
    pub(super) row: &'a dyn Row,
    pub(super) base: usize,
}

impl<'a> OffsetRow<'a> {
    /// `row`, from column `base` on.
    pub fn new(row: &'a dyn Row, base: usize) -> Self {
        Self { row, base }
    }
}

impl fmt::Debug for OffsetRow<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OffsetRow")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl Row for OffsetRow<'_> {
    fn len(&self) -> usize {
        self.row.len().saturating_sub(self.base)
    }

    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        self.row.get(self.base + index, ty)
    }
}

/// One element of a joined row: an entity, or `Option<E>` for a left-joined
/// entity. Tuples of these implement [`FromRow`].
pub trait JoinPart: Sized {
    /// The number of columns this part reads.
    fn width() -> usize;
    /// Decodes this part from the columns of `row` starting at `base`.
    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError>;
}

impl<E: Entity> JoinPart for E {
    fn width() -> usize {
        E::columns().len()
    }

    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError> {
        E::from_row(&OffsetRow::new(row, base))
    }
}

/// `None` when every id column is `NULL`: an id is never `NULL` in a stored
/// row, so that is a left join that found no match.
impl<E: Entity> JoinPart for Option<E> {
    fn width() -> usize {
        E::columns().len()
    }

    fn decode(row: &dyn Row, base: usize) -> Result<Self, ResultError> {
        let columns = E::columns();
        for id in E::ID_COLUMNS {
            let index = columns
                .iter()
                .position(|c| c.name == *id)
                .expect("id columns are columns of the entity");
            if !row.get(base + index, columns[index].sql_type)?.is_null() {
                return E::decode(row, base).map(Some);
            }
        }

        Ok(None)
    }
}

macro_rules! tuple_from_row {
    ($($R:ident),+) => {
        impl<$($R: JoinPart),+> FromRow for ($($R,)+) {
            #[allow(unused_assignments, reason = "the last part's width is not needed")]
            fn from_row(row: &dyn Row) -> Result<Self, ResultError> {
                let mut base = 0;
                Ok(($({
                    let part = $R::decode(row, base)?;
                    base += $R::width();
                    part
                },)+))
            }
        }
    };
}

tuple_from_row!(R0, R1);
tuple_from_row!(R0, R1, R2);
tuple_from_row!(R0, R1, R2, R3);
tuple_from_row!(R0, R1, R2, R3, R4);
tuple_from_row!(R0, R1, R2, R3, R4, R5);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7, R8);
tuple_from_row!(R0, R1, R2, R3, R4, R5, R6, R7, R8, R9);
