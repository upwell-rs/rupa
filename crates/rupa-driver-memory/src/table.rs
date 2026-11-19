//! Tables and the row scopes expressions are evaluated in.

use std::collections::HashMap;

use rupa_core::Value;
use rupa_core::column::ColumnMeta;
use rupa_core::ir::{ColumnRef, FromClause, SourceId, TableRef};

use crate::MemoryError;
use crate::eval::Scope;
use crate::store::Store;

#[derive(Debug, Clone)]
pub(crate) struct Table {
    pub(crate) columns: Vec<ColumnMeta>,
    pub(crate) key: Vec<usize>,
    pub(crate) rows: Vec<Vec<Value>>,
    /// Last value handed out per generated integer column (identity semantics).
    pub(crate) sequences: HashMap<usize, i64>,
}

impl Table {
    pub(crate) fn index(&self, name: &'static str) -> Result<usize, MemoryError> {
        self.columns
            .iter()
            .position(|c| c.name == name)
            .ok_or(MemoryError::NoSuchColumn(name))
    }

    /// Validates `row` against NOT NULL and the primary key; `skip` is the
    /// row's own index when it is already in the table.
    pub(crate) fn check(
        &self,
        name: TableRef,
        row: &[Value],
        skip: Option<usize>,
    ) -> Result<(), MemoryError> {
        for (meta, v) in self.columns.iter().zip(row) {
            if v.is_null() && !meta.nullable {
                return Err(MemoryError::NotNull(meta.name));
            }
        }

        if !self.key.is_empty() {
            let key = |r: &[Value]| self.key.iter().map(|&i| r[i].clone()).collect::<Vec<_>>();
            let k = key(row);
            let dup = self
                .rows
                .iter()
                .enumerate()
                .any(|(i, r)| Some(i) != skip && key(r) == k);
            if dup {
                return Err(MemoryError::DuplicateKey(name));
            }
        }

        Ok(())
    }
}

pub(crate) struct RowScope<'a> {
    pub(crate) table: &'a Table,
    pub(crate) row: &'a [Value],
}

impl Scope for RowScope<'_> {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        if col.source != SourceId(0) {
            return Err(MemoryError::NoSuchSource(col.source));
        }

        Ok(self.row[self.table.index(col.name)?].clone())
    }
}

/// One row of a select's sources: a row per source, `None` where a left
/// join found no match.
pub(crate) type JoinedRow<'a> = Vec<Option<&'a Vec<Value>>>;

/// The tables a select reads, by source.
pub(crate) struct Sources<'a>(pub(crate) Vec<&'a Table>);

impl<'a> Sources<'a> {
    pub(crate) fn new(db: &'a Store, from: &FromClause) -> Result<Self, MemoryError> {
        let tables = match from {
            FromClause::Table(t) => vec![db.table(*t)?],
            FromClause::Join { base, joins } => std::iter::once(*base)
                .chain(joins.iter().map(|j| j.table))
                .map(|t| db.table(t))
                .collect::<Result<_, _>>()?,
        };
        Ok(Self(tables))
    }

    /// Where `col` lives: its source and its index in that source's rows.
    pub(crate) fn locate(&self, col: ColumnRef) -> Result<(usize, usize), MemoryError> {
        let source = usize::from(col.source.0);
        let table = self
            .0
            .get(source)
            .ok_or(MemoryError::NoSuchSource(col.source))?;
        Ok((source, table.index(col.name)?))
    }

    /// `col` of `row`; typed `NULL` for a source a left join left unmatched.
    /// While a join's `ON` is evaluated, `row` holds only the sources joined so
    /// far, so a later source is `NoSuchSource` too.
    pub(crate) fn value(
        &self,
        row: &[Option<&Vec<Value>>],
        col: ColumnRef,
    ) -> Result<Value, MemoryError> {
        let (source, index) = self.locate(col)?;
        match row.get(source) {
            None => Err(MemoryError::NoSuchSource(col.source)),
            Some(Some(r)) => Ok(r[index].clone()),
            Some(None) => Ok(Value::Null(self.0[source].columns[index].sql_type)),
        }
    }
}

pub(crate) struct JoinScope<'s, 'a> {
    pub(crate) sources: &'s Sources<'a>,
    pub(crate) row: &'s [Option<&'a Vec<Value>>],
}

impl Scope for JoinScope<'_, '_> {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        self.sources.value(self.row, col)
    }
}

/// Scope for expressions evaluated without a row (`LIMIT`, `OFFSET`).
pub(crate) struct NoRow;

impl Scope for NoRow {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        Err(MemoryError::NoSuchColumn(col.name))
    }
}
