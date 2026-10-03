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

mod eval;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;

use rupa_core::column::ColumnMeta;
use rupa_core::dialect::Memory;
use rupa_core::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use rupa_core::ir::{ColumnRef, Delete, FromClause, Insert, Select, Statement, TableRef, Update};
use rupa_core::row::ValueRows;
use rupa_core::tx::{AsyncTransaction, AsyncTransactional, Transaction, Transactional, TxOptions};
use rupa_core::{DslError, Entity, Expect, ResultError, SqlType, TxError, Value};

use eval::{Scope, eval, keeps, sort_cmp};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MemoryError {
    NoSuchTable(TableRef),
    NoSuchColumn(&'static str),
    DuplicateKey(TableRef),
    NotNull(&'static str),
    Type(String),
    Unsupported(String),
    Dsl(DslError),
    Result(ResultError),
    /// A write inside a read-only transaction.
    ReadOnly,
    Tx(TxError),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::NoSuchTable(t) => write!(f, "no such table {t:?}"),
            MemoryError::NoSuchColumn(c) => write!(f, "no such column `{c}`"),
            MemoryError::DuplicateKey(t) => write!(f, "duplicate primary key in {t:?}"),
            MemoryError::NotNull(c) => write!(f, "NULL in NOT NULL column `{c}`"),
            MemoryError::Type(m) => write!(f, "type error: {m}"),
            MemoryError::Unsupported(m) => write!(f, "unsupported by the memory backend: {m}"),
            MemoryError::Dsl(e) => e.fmt(f),
            MemoryError::Result(e) => e.fmt(f),
            MemoryError::ReadOnly => f.write_str("cannot write in a read-only transaction"),
            MemoryError::Tx(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<ResultError> for MemoryError {
    fn from(e: ResultError) -> Self {
        MemoryError::Result(e)
    }
}

impl From<DslError> for MemoryError {
    fn from(e: DslError) -> Self {
        MemoryError::Dsl(e)
    }
}

impl ExecError for MemoryError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            MemoryError::Result(e) => Some(e),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Table {
    columns: Vec<ColumnMeta>,
    key: Vec<usize>,
    rows: Vec<Vec<Value>>,
    /// Last value handed out per generated integer column (identity semantics).
    sequences: HashMap<usize, i64>,
}

impl Table {
    fn index(&self, name: &'static str) -> Result<usize, MemoryError> {
        self.columns
            .iter()
            .position(|c| c.name == name)
            .ok_or(MemoryError::NoSuchColumn(name))
    }

    /// Validates `row` against NOT NULL and the primary key; `skip` is the
    /// row's own index when it is already in the table.
    fn check(&self, name: TableRef, row: &[Value], skip: Option<usize>) -> Result<(), MemoryError> {
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

struct RowScope<'a> {
    table: &'a Table,
    row: &'a [Value],
}

impl Scope for RowScope<'_> {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        Ok(self.row[self.table.index(col.name)?].clone())
    }
}

/// Scope for expressions evaluated without a row (`LIMIT`, `OFFSET`).
struct NoRow;

impl Scope for NoRow {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        Err(MemoryError::NoSuchColumn(col.name))
    }
}

/// An in-memory database: a synchronous [`Executor`]. For an
/// [`AsyncExecutor`], use [`AsyncMemoryDb`] (`db.into_async()`).
#[derive(Debug, Clone, Default)]
pub struct MemoryDb {
    tables: HashMap<TableRef, Table>,
    dialect: Memory,
    /// Table states to restore on rollback, one per open transaction level.
    snapshots: Vec<HashMap<TableRef, Table>>,
}

impl MemoryDb {
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
        match statement {
            Statement::Select(s) => {
                let (table, rows) = self.select_rows(s)?;
                let idx = s
                    .projection
                    .iter()
                    .map(|c| table.index(c.name))
                    .collect::<Result<Vec<_>, _>>()?;
                let projected = rows
                    .into_iter()
                    .map(|r| idx.iter().map(|&i| r[i].clone()).collect())
                    .collect();
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
        }
    }

    fn select_rows(&self, s: &Select) -> Result<(&Table, Vec<&Vec<Value>>), MemoryError> {
        let FromClause::Table(t) = &s.from;
        let table = self.table(*t)?;
        let mut rows = Vec::new();
        for row in &table.rows {
            if keeps(&RowScope { table, row }, s.filter.as_ref())? {
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
                        .map(|o| eval(&RowScope { table, row }, &o.expr))
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
        Ok((table, rows.into_iter().skip(offset).take(limit).collect()))
    }

    /// Inserts rows and returns them, as stored, in insertion order.
    fn insert(&mut self, i: &Insert) -> Result<Vec<Vec<Value>>, MemoryError> {
        let table = self.table_mut(i.table)?;
        let idx = i
            .columns
            .iter()
            .map(|c| table.index(c))
            .collect::<Result<Vec<_>, _>>()?;
        // Build on a copy and swap it in at the end: a failed multi-row insert
        // leaves no partial result, as in Postgres.
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
                // Every SET expression sees the row as it was before the update.
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

/// The memory backend as an [`AsyncExecutor`]. A separate type so that each
/// backend type implements exactly one executor trait, and `.run` is never
/// ambiguous with both traits in scope.
#[derive(Debug, Clone, Default)]
pub struct AsyncMemoryDb(MemoryDb);

impl AsyncMemoryDb {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or empties) the table for `E`.
    pub fn register<E: Entity>(&mut self) -> &mut Self {
        self.0.register::<E>();
        self
    }

    pub fn inner(&self) -> &MemoryDb {
        &self.0
    }

    pub fn inner_mut(&mut self) -> &mut MemoryDb {
        &mut self.0
    }

    pub fn into_inner(self) -> MemoryDb {
        self.0
    }
}

impl From<MemoryDb> for AsyncMemoryDb {
    fn from(db: MemoryDb) -> Self {
        Self(db)
    }
}

impl MemoryDb {
    /// The same database, as an async executor.
    pub fn into_async(self) -> AsyncMemoryDb {
        AsyncMemoryDb(self)
    }
}

impl AsyncExecutor for AsyncMemoryDb {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.0.dialect
    }

    fn execute(
        &mut self,
        statement: &Statement,
        _expect: Expect,
    ) -> impl Future<Output = Result<Outcome, MemoryError>> + Send {
        std::future::ready(self.0.run_statement(statement))
    }
}

// ---------------------------------------------------------------------------
// Transactions: a stack of table snapshots.
// ---------------------------------------------------------------------------

/// A transaction (or, nested, a savepoint) on a [`MemoryDb`]. Rolled back
/// when dropped without `commit`.
#[derive(Debug)]
pub struct MemoryTx<'t> {
    db: &'t mut MemoryDb,
    /// Number of snapshots on the stack while this transaction is open.
    level: usize,
    read_only: bool,
    done: bool,
}

impl<'t> MemoryTx<'t> {
    fn open(db: &'t mut MemoryDb, read_only: bool) -> Self {
        db.snapshots.push(db.tables.clone());
        let level = db.snapshots.len();
        MemoryTx {
            db,
            level,
            read_only,
            done: false,
        }
    }

    fn nested(&mut self, options: TxOptions) -> Result<MemoryTx<'_>, MemoryError> {
        if !options.is_default() {
            return Err(MemoryError::Tx(TxError::OptionsOnNested));
        }
        Ok(MemoryTx::open(&mut *self.db, self.read_only))
    }

    fn finish(&mut self, commit: bool) {
        debug_assert_eq!(
            self.db.snapshots.len(),
            self.level,
            "transactions end innermost first"
        );
        let snapshot = self
            .db
            .snapshots
            .pop()
            .expect("open transaction has a snapshot");
        if !commit {
            self.db.tables = snapshot;
        }
        self.done = true;
    }

    fn run(&mut self, statement: &Statement) -> Result<Outcome, MemoryError> {
        let writes = matches!(
            statement,
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Raw(_)
        );
        if writes && self.read_only {
            return Err(MemoryError::ReadOnly);
        }
        self.db.run_statement(statement)
    }
}

impl Drop for MemoryTx<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.finish(false);
        }
    }
}

impl Executor for MemoryTx<'_> {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.db.dialect
    }

    fn execute(&mut self, statement: &Statement, _expect: Expect) -> Result<Outcome, MemoryError> {
        self.run(statement)
    }
}

impl Transaction for MemoryTx<'_> {
    fn commit(mut self) -> Result<(), MemoryError> {
        self.finish(true);
        Ok(())
    }

    fn rollback(mut self) -> Result<(), MemoryError> {
        self.finish(false);
        Ok(())
    }
}

impl Transactional for MemoryTx<'_> {
    type Tx<'s>
        = MemoryTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<MemoryTx<'_>, MemoryError> {
        self.nested(options)
    }
}

impl Transactional for MemoryDb {
    type Tx<'t>
        = MemoryTx<'t>
    where
        Self: 't;

    /// The memory backend is single-threaded, so every isolation level
    /// behaves as serializable; read-only is enforced.
    fn begin_with(&mut self, options: TxOptions) -> Result<MemoryTx<'_>, MemoryError> {
        Ok(MemoryTx::open(self, options.read_only))
    }
}

/// [`MemoryTx`] as an async transaction, for [`AsyncMemoryDb`].
#[derive(Debug)]
pub struct AsyncMemoryTx<'t>(MemoryTx<'t>);

impl AsyncExecutor for AsyncMemoryTx<'_> {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.0.db.dialect
    }

    fn execute(
        &mut self,
        statement: &Statement,
        _expect: Expect,
    ) -> impl Future<Output = Result<Outcome, MemoryError>> + Send {
        std::future::ready(self.0.run(statement))
    }
}

impl AsyncTransaction for AsyncMemoryTx<'_> {
    fn commit(self) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(Transaction::commit(self.0))
    }

    fn rollback(self) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(Transaction::rollback(self.0))
    }
}

impl AsyncTransactional for AsyncMemoryTx<'_> {
    type Tx<'s>
        = AsyncMemoryTx<'s>
    where
        Self: 's;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncMemoryTx<'_>, MemoryError>> + Send {
        std::future::ready(self.0.nested(options).map(AsyncMemoryTx))
    }
}

impl AsyncTransactional for AsyncMemoryDb {
    type Tx<'t>
        = AsyncMemoryTx<'t>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncMemoryTx<'_>, MemoryError>> + Send {
        std::future::ready(Ok(AsyncMemoryTx(MemoryTx::open(
            &mut self.0,
            options.read_only,
        ))))
    }
}
