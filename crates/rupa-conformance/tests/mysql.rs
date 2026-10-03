//! The conformance suite against a real MySQL 8 (Docker), through a minimal
//! test-only executor: rupa's MySQL rendering is exercised end to end before
//! a MySQL driver exists (sqlx, milestone 11).
//!
//! The executor does what a real driver must: binds `Value`s, reads typed
//! rows, counts *matched* rows for `UPDATE` (`CLIENT_FOUND_ROWS`), and runs
//! transactions with savepoints.

use std::fmt;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use mysql::prelude::Queryable;
use rupa::core::exec::{ExecError, Executor, Outcome};
use rupa::core::ir::{Statement, TxStatement};
use rupa::core::query::Expect;
use rupa::core::tx::{Transaction, Transactional, TxOptions};
use rupa::core::{MySql, ResultError, RowCursor, RowError, SqlType, TxError, Value};
use rupa_conformance::{Blocking, Harness, MYSQL_DDL};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::SyncRunner;

#[derive(Debug)]
#[allow(dead_code, reason = "read through Debug in failure messages")]
enum TestError {
    Render(rupa_sql::RenderError),
    Db(mysql::Error),
    Result(ResultError),
    Tx(TxError),
}

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for TestError {}

impl From<rupa_sql::RenderError> for TestError {
    fn from(e: rupa_sql::RenderError) -> Self {
        TestError::Render(e)
    }
}

impl From<mysql::Error> for TestError {
    fn from(e: mysql::Error) -> Self {
        TestError::Db(e)
    }
}

impl From<ResultError> for TestError {
    fn from(e: ResultError) -> Self {
        TestError::Result(e)
    }
}

impl ExecError for TestError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            TestError::Result(e) => Some(e),
            _ => None,
        }
    }
}

fn to_mysql(v: &Value) -> mysql::Value {
    use mysql::Value as M;
    let datetime = |t: &NaiveDateTime| {
        M::Date(
            t.year() as u16,
            t.month() as u8,
            t.day() as u8,
            t.hour() as u8,
            t.minute() as u8,
            t.second() as u8,
            t.nanosecond() / 1000,
        )
    };
    match v {
        Value::Null(_) => M::NULL,
        Value::Bool(b) => M::Int(i64::from(*b)),
        Value::I16(n) => M::Int(i64::from(*n)),
        Value::I32(n) => M::Int(i64::from(*n)),
        Value::I64(n) => M::Int(*n),
        Value::F32(n) => M::Float(*n),
        Value::F64(n) => M::Double(*n),
        Value::Text(s) => M::Bytes(s.clone().into_bytes()),
        Value::Bytes(b) => M::Bytes(b.clone()),
        Value::Uuid(u) => M::Bytes(u.hyphenated().to_string().into_bytes()),
        Value::Date(d) => M::Date(d.year() as u16, d.month() as u8, d.day() as u8, 0, 0, 0, 0),
        Value::Time(t) => M::Time(
            false,
            0,
            t.hour() as u8,
            t.minute() as u8,
            t.second() as u8,
            t.nanosecond() / 1000,
        ),
        Value::Timestamp(t) => datetime(t),
        Value::TimestampTz(t) => datetime(&t.naive_utc()),
        Value::Json(j) => M::Bytes(j.to_string().into_bytes()),
        other => panic!("unsupported value {other:?}"),
    }
}

fn from_mysql(v: &mysql::Value, want: SqlType) -> Result<Value, RowError> {
    use mysql::Value as M;
    let bad = || RowError::Driver(format!("cannot read {v:?} as {want:?}"));
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|_| bad());
    let int = |v: &M| -> Result<i64, RowError> {
        match v {
            M::Int(n) => Ok(*n),
            M::UInt(n) => i64::try_from(*n).map_err(|_| bad()),
            M::Bytes(b) => text(b)?.parse().map_err(|_| bad()),
            _ => Err(bad()),
        }
    };
    let datetime = |v: &M| match v {
        M::Date(y, mo, d, h, mi, s, us) => {
            NaiveDate::from_ymd_opt(i32::from(*y), u32::from(*mo), u32::from(*d))
                .and_then(|d| {
                    d.and_hms_micro_opt(u32::from(*h), u32::from(*mi), u32::from(*s), *us)
                })
                .ok_or_else(bad)
        }
        _ => Err(bad()),
    };
    Ok(match (want, v) {
        (_, M::NULL) => Value::Null(want),
        (SqlType::Bool, v) => Value::Bool(int(v)? != 0),
        (SqlType::I16, v) => Value::I16(int(v)?.try_into().map_err(|_| bad())?),
        (SqlType::I32, v) => Value::I32(int(v)?.try_into().map_err(|_| bad())?),
        (SqlType::I64, v) => Value::I64(int(v)?),
        (SqlType::F32 | SqlType::F64, M::Double(f)) => Value::F64(*f),
        (SqlType::F32 | SqlType::F64, M::Float(f)) => Value::F64(f64::from(*f)),
        (SqlType::Text, M::Bytes(b)) => Value::Text(text(b)?),
        (SqlType::Bytes, M::Bytes(b)) => Value::Bytes(b.clone()),
        (SqlType::Uuid, M::Bytes(b)) => Value::Uuid(text(b)?.parse().map_err(|_| bad())?),
        (SqlType::Date, v) => Value::Date(datetime(v)?.date()),
        (SqlType::Time, M::Time(_, _, h, m, s, us)) => Value::Time(
            NaiveTime::from_hms_micro_opt(u32::from(*h), u32::from(*m), u32::from(*s), *us)
                .ok_or_else(bad)?,
        ),
        (SqlType::Timestamp, v) => Value::Timestamp(datetime(v)?),
        (SqlType::TimestampTz, v) => Value::TimestampTz(datetime(v)?.and_utc()),
        (SqlType::Json, M::Bytes(b)) => Value::Json(serde_json::from_slice(b).map_err(|_| bad())?),
        _ => return Err(bad()),
    })
}

struct MyRow(Vec<mysql::Value>);

impl rupa::core::Row for MyRow {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        from_mysql(
            self.0.get(index).ok_or(RowError::IndexOutOfRange {
                index,
                len: self.0.len(),
            })?,
            ty,
        )
    }
}

struct MyRows(std::vec::IntoIter<Vec<mysql::Value>>, Option<MyRow>);

impl RowCursor for MyRows {
    fn next_row(&mut self) -> Option<Result<&dyn rupa::core::Row, RowError>> {
        self.1 = Some(MyRow(self.0.next()?));
        self.1.as_ref().map(|r| Ok(r as &dyn rupa::core::Row))
    }
}

struct MyExec {
    conn: mysql::Conn,
    dialect: MySql,
    pending_rollback: Option<u32>,
}

fn rollback_to(depth: u32) -> TxStatement {
    if depth == 0 {
        TxStatement::Rollback
    } else {
        TxStatement::RollbackToSavepoint(depth)
    }
}

impl MyExec {
    /// Runs a script statement by statement (no multi-statement protocol).
    fn script(&mut self, sql: &str) -> Result<(), TestError> {
        for stmt in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            self.conn.query_drop(stmt)?;
        }
        Ok(())
    }

    fn clean(&mut self) -> Result<(), TestError> {
        if let Some(d) = self.pending_rollback.take() {
            let sql = rupa_sql::render_tx(&rollback_to(d), &self.dialect)?;
            self.script(&sql)?;
        }
        Ok(())
    }

    fn control(&mut self, s: TxStatement) -> Result<(), TestError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&s, &self.dialect)?;
        self.script(&sql)
    }

    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, TestError> {
        self.clean()?;
        let r = rupa_sql::render(statement, &self.dialect)?;
        let params = if r.params.is_empty() {
            mysql::Params::Empty
        } else {
            mysql::Params::Positional(r.params.iter().map(to_mysql).collect())
        };
        if expect == Expect::Affected {
            self.conn.exec_drop(&r.sql, params)?;
            return Ok(Outcome::Affected(self.conn.affected_rows()));
        }
        let rows: Vec<mysql::Row> = self.conn.exec(&r.sql, params)?;
        let rows: Vec<Vec<mysql::Value>> = rows.into_iter().map(mysql::Row::unwrap).collect();
        Ok(Outcome::Rows(Box::new(MyRows(rows.into_iter(), None))))
    }
}

impl Executor for MyExec {
    type Dialect = MySql;
    type Error = TestError;

    fn dialect(&self) -> &MySql {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, TestError> {
        self.run(statement, expect)
    }
}

impl Transactional for MyExec {
    type Tx<'t> = MyTx<'t>;

    fn begin_with(&mut self, options: TxOptions) -> Result<MyTx<'_>, TestError> {
        self.control(TxStatement::Begin(options))?;
        Ok(MyTx {
            exec: self,
            depth: 0,
            done: false,
        })
    }
}

struct MyTx<'t> {
    exec: &'t mut MyExec,
    depth: u32,
    done: bool,
}

impl MyTx<'_> {
    fn finish(&mut self, commit: bool) -> Result<(), TestError> {
        let s = match (commit, self.depth) {
            (true, 0) => TxStatement::Commit,
            (true, d) => TxStatement::ReleaseSavepoint(d),
            (false, d) => rollback_to(d),
        };
        self.exec.control(s)?;
        self.done = true;
        Ok(())
    }
}

impl Drop for MyTx<'_> {
    fn drop(&mut self) {
        if !self.done && self.finish(false).is_err() {
            self.exec.pending_rollback = Some(self.depth);
        }
    }
}

impl Executor for MyTx<'_> {
    type Dialect = MySql;
    type Error = TestError;

    fn dialect(&self) -> &MySql {
        &self.exec.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, TestError> {
        self.exec.run(statement, expect)
    }
}

impl Transaction for MyTx<'_> {
    fn commit(mut self) -> Result<(), TestError> {
        self.finish(true)
    }

    fn rollback(mut self) -> Result<(), TestError> {
        self.finish(false)
    }
}

impl Transactional for MyTx<'_> {
    type Tx<'s>
        = MyTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<MyTx<'_>, TestError> {
        if !options.is_default() {
            return Err(TestError::Tx(TxError::OptionsOnNested));
        }
        let depth = self.depth + 1;
        self.exec.control(TxStatement::Savepoint(depth))?;
        Ok(MyTx {
            exec: &mut *self.exec,
            depth,
            done: false,
        })
    }
}

struct MyHarness(Blocking<MyExec>);

impl Harness for MyHarness {
    type Exec = Blocking<MyExec>;

    async fn reset(&mut self) {
        self.0.0.clean().expect("clean");
        self.0.0.script(MYSQL_DDL).expect("reset schema");
    }

    fn exec(&mut self) -> &mut Blocking<MyExec> {
        &mut self.0
    }
}

#[test]
fn conformance() {
    let node = Mysql::default().start().expect("start mysql container");
    let port = node.get_host_port_ipv4(3306).unwrap();
    let opts = mysql::OptsBuilder::new()
        .ip_or_hostname(Some("127.0.0.1"))
        .tcp_port(port)
        .user(Some("root"))
        .db_name(Some("test"))
        // Count matched (not changed) rows for UPDATE, as Postgres does.
        .additional_capabilities(mysql::consts::CapabilityFlags::CLIENT_FOUND_ROWS);
    // MySQL restarts once during initialization; retry until it accepts.
    let mut attempt = 0;
    let conn = loop {
        match mysql::Conn::new(opts.clone()) {
            Ok(c) => break c,
            Err(e) if attempt < 60 => {
                attempt += 1;
                let _ = e;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => panic!("connect: {e}"),
        }
    };
    let mut h = MyHarness(Blocking(MyExec {
        conn,
        dialect: MySql,
        pending_rollback: None,
    }));
    pollster::block_on(rupa_conformance::run(&mut h));
    pollster::block_on(rupa_conformance::run_transactions(&mut h));
}
