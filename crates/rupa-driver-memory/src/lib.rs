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

mod eval;
#[cfg(test)]
mod join_tests;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;

use rupa_core::column::ColumnMeta;
use rupa_core::config::{DbConfig, DriverConfig};
use rupa_core::dialect::Memory;
use rupa_core::erased::BoxFuture;
use rupa_core::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use rupa_core::ir::{
    ColumnRef, Delete, FromClause, Insert, JoinKind, Select, SourceId, Statement, TableRef,
    TxStatement, Update,
};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::row::ValueRows;
use rupa_core::tx::{AsyncTx, AsyncTxConnection, Tx, TxConnection};
use rupa_core::{DslError, Entity, Expect, ResultError, SqlType, TxError, Value};

use eval::{Scope, eval, keeps, sort_cmp};

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
/// Everything that can go wrong evaluating a statement in memory.
pub enum MemoryError {
    /// The table was never registered.
    NoSuchTable(TableRef),
    /// The table has no such column.
    NoSuchColumn(&'static str),
    /// A column names a row source the statement doesn't have.
    NoSuchSource(SourceId),
    /// An insert or update would duplicate a primary key.
    DuplicateKey(TableRef),
    /// `NULL` in a `NOT NULL` column.
    NotNull(&'static str),
    /// A value has the wrong type for an operation.
    Type(String),
    /// The statement uses something the backend can't evaluate (raw SQL,
    /// a DSL function without `eval`, ..).
    Unsupported(String),
    /// A DSL function failed.
    Dsl(DslError),
    /// The rows did not fit the query's result type.
    Result(ResultError),
    /// A write inside a read-only transaction.
    ReadOnly,
    /// The transaction API was misused.
    Tx(TxError),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryError::NoSuchTable(t) => write!(f, "no such table {t:?}"),
            MemoryError::NoSuchColumn(c) => write!(f, "no such column `{c}`"),
            MemoryError::NoSuchSource(s) => write!(f, "no row source {}", s.0),
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

impl From<TxError> for MemoryError {
    fn from(e: TxError) -> Self {
        MemoryError::Tx(e)
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
        if col.source != SourceId(0) {
            return Err(MemoryError::NoSuchSource(col.source));
        }
        Ok(self.row[self.table.index(col.name)?].clone())
    }
}

/// One row of a select's sources: a row per source, `None` where a left
/// join found no match.
type JoinedRow<'a> = Vec<Option<&'a Vec<Value>>>;

/// The tables a select reads, by source.
struct Sources<'a>(Vec<&'a Table>);

impl<'a> Sources<'a> {
    fn new(db: &'a MemoryDb, from: &FromClause) -> Result<Self, MemoryError> {
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
    fn locate(&self, col: ColumnRef) -> Result<(usize, usize), MemoryError> {
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
    fn value(&self, row: &[Option<&Vec<Value>>], col: ColumnRef) -> Result<Value, MemoryError> {
        let (source, index) = self.locate(col)?;
        match row.get(source) {
            None => Err(MemoryError::NoSuchSource(col.source)),
            Some(Some(r)) => Ok(r[index].clone()),
            Some(None) => Ok(Value::Null(self.0[source].columns[index].sql_type)),
        }
    }
}

struct JoinScope<'s, 'a> {
    sources: &'s Sources<'a>,
    row: &'s [Option<&'a Vec<Value>>],
}

impl Scope for JoinScope<'_, '_> {
    fn column(&self, col: ColumnRef) -> Result<Value, MemoryError> {
        self.sources.value(self.row, col)
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
    /// An empty database: register tables before use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or empties) the table for `E`.
    pub fn register<E: Entity>(&mut self) -> &mut Self {
        self.0.register::<E>();
        self
    }

    /// The sync database underneath.
    pub fn inner(&self) -> &MemoryDb {
        &self.0
    }

    /// The sync database underneath, mutably.
    pub fn inner_mut(&mut self) -> &mut MemoryDb {
        &mut self.0
    }

    /// The sync database underneath, by value.
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
pub type MemoryTx<'t> = Tx<&'t mut MemoryDb>;

/// A transaction (or, nested, a savepoint) on an [`AsyncMemoryDb`]. Rolled
/// back when dropped without `commit` (immediately: nothing to await).
pub type AsyncMemoryTx<'t> = AsyncTx<&'t mut AsyncMemoryDb>;

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

    fn defer_rollback(&mut self, depth: u32) {
        // Nothing can fail in memory: roll back right away.
        self.control_now(rupa_core::tx::rollback_to(depth));
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn clean(&mut self) -> Result<(), MemoryError> {
        Ok(())
    }
}

impl AsyncTxConnection for AsyncMemoryDb {
    fn control(
        &mut self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(TxConnection::control(&mut self.0, statement))
    }

    fn defer_rollback(&mut self, depth: u32) {
        TxConnection::defer_rollback(&mut self.0, depth);
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(Ok(()))
    }
}

impl From<MemoryError> for rupa_core::DynError {
    fn from(e: MemoryError) -> Self {
        rupa_core::DynError::new(e)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

type SetupFn = dyn Fn(&mut MemoryDb) + Send + Sync;

/// The registry entry for the memory backend: name `memory`, scheme
/// `memory` (`memory:` is a complete URL).
///
/// Each `connect` creates a fresh, empty database served as a single shared
/// connection ([`Database::single`]); pool settings are ignored. Tables must
/// be registered before use, so give the driver a [`setup`](Self::setup)
/// function:
///
/// ```ignore
/// let registry = Registry::new().with(MemoryDriver::new().setup(|db| {
///     db.register::<User>();
/// }));
/// ```
#[derive(Clone, Default)]
pub struct MemoryDriver {
    setup: Option<std::sync::Arc<SetupFn>>,
}

impl MemoryDriver {
    /// A driver whose databases start without tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runs `f` on every new database, e.g. to register tables or seed rows.
    pub fn setup(mut self, f: impl Fn(&mut MemoryDb) + Send + Sync + 'static) -> Self {
        self.setup = Some(std::sync::Arc::new(f));
        self
    }
}

impl fmt::Debug for MemoryDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryDriver")
            .field("setup", &self.setup.is_some())
            .finish()
    }
}

impl Driver for MemoryDriver {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn dialect(&self) -> rupa_core::DialectId {
        rupa_core::DialectId::Memory
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        let database = match &config.driver {
            DriverConfig::Auto | DriverConfig::Memory => {
                let mut db = MemoryDb::new();
                if let Some(setup) = &self.setup {
                    setup(&mut db);
                }
                Ok(Database::single(db.into_async()))
            }
            other => Err(ConnectError::InvalidOption {
                key: "driver".into(),
                message: format!(
                    "the memory driver was given options for `{}`",
                    other.name().unwrap_or("auto")
                ),
            }),
        };
        Box::pin(std::future::ready(database))
    }
}
