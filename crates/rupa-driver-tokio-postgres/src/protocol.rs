//! What both PostgreSQL drivers need to implement the shared migration
//! protocol (`rupa_migrate::protocol::postgres`). Not public API.

use rupa_core::{SqlType, Value};
use rupa_migrate::StateError;
use rupa_migrate::protocol::SqlState;
use tokio_postgres::Row;

use crate::shared::{Param, PgError, read};

impl From<StateError> for PgError {
    fn from(state: StateError) -> Self {
        Self::Migration(state)
    }
}

impl SqlState for PgError {
    fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Db(error) => error.code().map(|code| code.code().to_owned()),
            _ => None,
        }
    }
}

/// `values` as parameters.
pub fn params(values: &[Value]) -> Vec<Param<'_>> {
    values.iter().map(Param).collect()
}

/// Decodes each row's columns as `columns`.
pub fn values(rows: &[Row], columns: &[SqlType]) -> Result<Vec<Vec<Value>>, PgError> {
    rows.iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(index, want)| read(row, index, *want).map_err(|e| PgError::Result(e.into())))
                .collect()
        })
        .collect()
}
