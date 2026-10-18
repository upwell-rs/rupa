//! In-memory RUPA backend. It evaluates the query IR directly and never
//! renders SQL, which keeps the IR honest: anything expressible in it must
//! have a meaning without a database.
//!
//! Semantics follow Postgres where they are observable: three-valued logic,
//! `NULL` ordering (`NULLS LAST` ascending, `NULLS FIRST` descending),
//! `LIKE`, JSON `->`/`->>`, primary-key uniqueness and `NOT NULL` columns.
//!
//! Known differences from Postgres:
//! - Text compares by byte order (Postgres' `C` collation), not by locale.
//! - Type errors surface when a row is evaluated, not when the query is planned.
//! - `->>` on a JSON object or array is unsupported, because jsonb normalizes
//!   their text form.
//! - Raw SQL, and DSL functions without an `eval`, are unsupported.
//!
//! Tables must be registered (the moral equivalent of a migration) before use.

#![warn(missing_docs)]

#[cfg(feature = "async")]
mod asynchronous;
#[cfg(feature = "dynamic")]
mod driver;
mod error;
mod eval;
#[cfg(test)]
mod join_tests;
mod table;

use std::collections::HashMap;

use rupa_core::column::ColumnMeta;
use rupa_core::dialect::Memory;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::{
    Delete, FromClause, Insert, JoinKind, Select, Statement, TableRef, TxStatement, Update,
};
use rupa_core::row::ValueRows;
use rupa_core::tx::{Tx, TxConnection};
use rupa_core::{Entity, Expect, SqlType, Value};

#[cfg(feature = "async")]
pub use asynchronous::{AsyncMemoryDb, AsyncMemoryTx};
#[cfg(feature = "dynamic")]
pub use driver::MemoryDriver;
pub use error::MemoryError;
use eval::{eval, keeps, sort_cmp};
use table::{JoinScope, JoinedRow, NoRow, RowScope, Sources, Table};

/// An in-memory database: a synchronous [`Executor`]. With the `async`
/// feature, `db.into_async()` makes it an async executor (`AsyncMemoryDb`).
#[derive(Debug, Clone, Default)]
pub struct MemoryDb {
    tables: HashMap<TableRef, Table>,
    dialect: Memory,
    /// Table states to restore on rollback, one per open transaction level.
    snapshots: Vec<HashMap<TableRef, Table>>,
    /// Whether the open transaction is read-only.
    read_only: bool,
}

impl MemoryDb {
    /// An empty database: register tables before use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or empties) the table for `E`.
    pub fn register<E: Entity>(&mut self) -> &mut Self {
        self.create_table(E::TABLE, E::columns().to_vec(), E::ID_COLUMNS)
    }

    /// Creates (or empties) a table from column metadata.
    pub fn create_table(
        &mut self,
        table: TableRef,
        columns: Vec<ColumnMeta>,
        key: &[&'static str],
    ) -> &mut Self {
        let key = key
            .iter()
            .map(|k| {
                columns
                    .iter()
                    .position(|c| c.name == *k)
                    .expect("key column must be one of the columns")
            })
            .collect();
        self.tables.insert(
            table,
            Table {
                columns,
                key,
                rows: Vec::new(),
                sequences: HashMap::new(),
            },
        );
        self
    }

    /// All rows of a table, in insertion order.
    pub fn rows(&self, table: TableRef) -> Option<&[Vec<Value>]> {
        self.tables.get(&table).map(|t| t.rows.as_slice())
    }

    fn table(&self, t: TableRef) -> Result<&Table, MemoryError> {
        self.tables.get(&t).ok_or(MemoryError::NoSuchTable(t))
    }

    fn table_mut(&mut self, t: TableRef) -> Result<&mut Table, MemoryError> {
        self.tables.get_mut(&t).ok_or(MemoryError::NoSuchTable(t))
    }

    fn run_statement(&mut self, statement: &Statement) -> Result<Outcome, MemoryError> {
        let writes = matches!(
            statement,
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Raw(_)
        );
        if writes && self.read_only {
            return Err(MemoryError::ReadOnly);
        }

        match statement {
            Statement::Select(s) => {
                let (sources, rows) = self.select_rows(s)?;
                let projected = rows
                    .iter()
                    .map(|r| {
                        s.projection
                            .iter()
                            .map(|c| sources.value(r, *c))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Outcome::Rows(Box::new(ValueRows::new(projected))))
            }
            Statement::Exists(s) => {
                let (_, rows) = self.select_rows(s)?;
                Ok(Outcome::Rows(Box::new(ValueRows::new(vec![vec![
                    Value::Bool(!rows.is_empty()),
                ]]))))
            }
            Statement::Insert(i) => {
                let rows = self.insert(i)?;
                match &i.returning {
                    None => Ok(Outcome::Affected(rows.len() as u64)),
                    Some(cols) => {
                        let table = self.table(i.table)?;
                        let idx = cols
                            .iter()
                            .map(|c| table.index(c.name))
                            .collect::<Result<Vec<_>, _>>()?;
                        let projected = rows
                            .into_iter()
                            .map(|r| idx.iter().map(|&i| r[i].clone()).collect())
                            .collect();
                        Ok(Outcome::Rows(Box::new(ValueRows::new(projected))))
                    }
                }
            }
            Statement::Update(u) => self.update(u).map(Outcome::Affected),
            Statement::Delete(d) => self.delete(d).map(Outcome::Affected),
            Statement::Raw(_) => Err(MemoryError::Unsupported("raw SQL".into())),
            Statement::ApplySecurity(_) => Err(MemoryError::Unsupported(
                "row-level security (no native policies)".into(),
            )),
        }
    }

    /// The source rows a select keeps, ordered and paged. Joins are nested
    /// loops: each row of the earlier sources pairs with every matching row of
    /// the joined table, in table order (or one unmatched row for a left join).
    fn select_rows(&self, s: &Select) -> Result<(Sources<'_>, Vec<JoinedRow<'_>>), MemoryError> {
        let sources = Sources::new(self, &s.from)?;
        let mut joined: Vec<JoinedRow<'_>> =
            sources.0[0].rows.iter().map(|r| vec![Some(r)]).collect();
        if let FromClause::Join { joins, .. } = &s.from {
            for (join, table) in joins.iter().zip(&sources.0[1..]) {
                let mut next = Vec::new();
                for partial in joined {
                    let mut matched = false;
                    for r in &table.rows {
                        let mut candidate = partial.clone();
                        candidate.push(Some(r));
                        let scope = JoinScope {
                            sources: &sources,
                            row: &candidate,
                        };
                        if keeps(&scope, Some(&join.on))? {
                            matched = true;
                            next.push(candidate);
                        }
                    }

                    if !matched && join.kind == JoinKind::Left {
                        let mut padded = partial;
                        padded.push(None);
                        next.push(padded);
                    }
                }

                joined = next;
            }
        }

        let mut rows = Vec::new();
        for row in joined {
            if keeps(
                &JoinScope {
                    sources: &sources,
                    row: &row,
                },
                s.filter.as_ref(),
            )? {
                rows.push(row);
            }
        }

        if !s.order_by.is_empty() {
            let mut keyed = rows
                .into_iter()
                .map(|row| {
                    let keys = s
                        .order_by
                        .iter()
                        .map(|o| {
                            eval(
                                &JoinScope {
                                    sources: &sources,
                                    row: &row,
                                },
                                &o.expr,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((keys, row))
                })
                .collect::<Result<Vec<_>, MemoryError>>()?;
            let mut err = None;
            keyed.sort_by(|(a, _), (b, _)| {
                for ((x, y), o) in a.iter().zip(b).zip(&s.order_by) {
                    let ord = sort_cmp(x, y).unwrap_or_else(|e| {
                        err.get_or_insert(e);
                        std::cmp::Ordering::Equal
                    });
                    let ord = match o.direction {
                        rupa_core::ir::Direction::Asc => ord,
                        rupa_core::ir::Direction::Desc => ord.reverse(),
                    };
                    if ord.is_ne() {
                        return ord;
                    }
                }

                std::cmp::Ordering::Equal
            });
            if let Some(e) = err {
                return Err(e);
            }

            rows = keyed.into_iter().map(|(_, r)| r).collect();
        }

        let count = |e: &Option<rupa_core::ir::ExprNode>| -> Result<Option<usize>, MemoryError> {
            match e {
                None => Ok(None),
                Some(e) => match eval(&NoRow, e)? {
                    Value::I64(n) if n >= 0 => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
                    Value::Null(_) => Ok(None),
                    other => Err(MemoryError::Type(format!(
                        "LIMIT/OFFSET must be a non-negative integer, got {other:?}"
                    ))),
                },
            }
        };
        let offset = count(&s.offset)?.unwrap_or(0);
        let limit = count(&s.limit)?.unwrap_or(usize::MAX);
        let rows = rows.into_iter().skip(offset).take(limit).collect();
        Ok((sources, rows))
    }

    /// Inserts rows and returns them, as stored, in insertion order.
    ///
    /// Build on a copy and swap it in at the end: a failed multi-row insert
    /// leaves no partial result, as in Postgres.
    fn insert(&mut self, i: &Insert) -> Result<Vec<Vec<Value>>, MemoryError> {
        let table = self.table_mut(i.table)?;
        let idx = i
            .columns
            .iter()
            .map(|c| table.index(c))
            .collect::<Result<Vec<_>, _>>()?;
        let mut next = table.clone();
        let mut inserted = Vec::with_capacity(i.rows.len());
        for exprs in &i.rows {
            let mut row: Vec<Value> = next
                .columns
                .iter()
                .map(|c| Value::Null(c.sql_type))
                .collect();
            for (&col, e) in idx.iter().zip(exprs) {
                row[col] = eval(&NoRow, e)?;
            }

            for (col, meta) in next.columns.iter().enumerate() {
                if meta.generated && !idx.contains(&col) {
                    let seq = next.sequences.entry(col).or_insert(0);
                    *seq += 1;
                    row[col] = match meta.sql_type {
                        SqlType::I16 => Value::I16(i16::try_from(*seq).map_err(|_| {
                            MemoryError::Type(format!("identity overflow in `{}`", meta.name))
                        })?),
                        SqlType::I32 => Value::I32(i32::try_from(*seq).map_err(|_| {
                            MemoryError::Type(format!("identity overflow in `{}`", meta.name))
                        })?),
                        SqlType::I64 => Value::I64(*seq),
                        other => {
                            return Err(MemoryError::Unsupported(format!(
                                "generated column `{}` of type {other:?} (only integer identities are generated)",
                                meta.name
                            )));
                        }
                    };
                }
            }

            next.check(i.table, &row, None)?;
            next.rows.push(row.clone());
            inserted.push(row);
        }

        *table = next;
        Ok(inserted)
    }

    /// Every SET expression sees the row as it was before the update.
    fn update(&mut self, u: &Update) -> Result<u64, MemoryError> {
        let table = self.table(u.table)?;
        let set = u
            .set
            .iter()
            .map(|(c, e)| Ok((table.index(c)?, e)))
            .collect::<Result<Vec<_>, MemoryError>>()?;
        let mut changes = Vec::new();
        for (i, row) in table.rows.iter().enumerate() {
            let scope = RowScope { table, row };
            if keeps(&scope, u.filter.as_ref())? {
                let mut new = row.clone();
                for (col, e) in &set {
                    new[*col] = eval(&scope, e)?;
                }

                changes.push((i, new));
            }
        }

        let table = self.table_mut(u.table)?;
        let mut next = table.clone();
        for (i, new) in &changes {
            next.rows[*i] = new.clone();
        }

        for (i, new) in &changes {
            next.check(u.table, new, Some(*i))?;
        }

        *table = next;
        Ok(changes.len() as u64)
    }

    fn delete(&mut self, d: &Delete) -> Result<u64, MemoryError> {
        let table = self.table(d.table)?;
        let mut keep = Vec::with_capacity(table.rows.len());
        for row in &table.rows {
            keep.push(!keeps(&RowScope { table, row }, d.filter.as_ref())?);
        }

        let table = self.table_mut(d.table)?;
        let before = table.rows.len();
        let mut it = keep.into_iter();
        table.rows.retain(|_| it.next().unwrap_or(true));
        Ok((before - table.rows.len()) as u64)
    }
}

impl Executor for MemoryDb {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, _expect: Expect) -> Result<Outcome, MemoryError> {
        self.run_statement(statement)
    }
}

/// A transaction (or, nested, a savepoint) on a [`MemoryDb`]. Rolled back
/// when dropped without `commit`.
pub type MemoryTx<'t> = Tx<&'t mut MemoryDb>;

impl MemoryDb {
    /// Applies a transaction-control statement to the snapshot stack.
    ///
    /// The backend is single-threaded, so every isolation level behaves as
    /// serializable; read-only is enforced.
    fn control_now(&mut self, statement: TxStatement) {
        match statement {
            TxStatement::Begin(options) => {
                self.read_only = options.read_only;
                self.snapshots.push(self.tables.clone());
            }
            TxStatement::Savepoint(_) => self.snapshots.push(self.tables.clone()),
            TxStatement::Commit | TxStatement::ReleaseSavepoint(_) => {
                self.snapshots.pop();
            }
            TxStatement::Rollback | TxStatement::RollbackToSavepoint(_) => {
                if let Some(snapshot) = self.snapshots.pop() {
                    self.tables = snapshot;
                }
            }
        }

        if self.snapshots.is_empty() {
            self.read_only = false;
        }
    }
}

impl TxConnection for MemoryDb {
    fn control(&mut self, statement: TxStatement) -> Result<(), MemoryError> {
        self.control_now(statement);
        Ok(())
    }

    /// Nothing can fail in memory: roll back right away.
    fn defer_rollback(&mut self, depth: u32) {
        self.control_now(rupa_core::tx::rollback_to(depth));
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn clean(&mut self) -> Result<(), MemoryError> {
        Ok(())
    }
}
