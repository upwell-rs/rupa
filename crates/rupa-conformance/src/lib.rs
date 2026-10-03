//! Behavioural test suite shared by every driver. Each driver's tests supply a
//! [`Harness`] and call [`run`]. Because the same assertions run against the
//! memory backend and real databases, passing everywhere means the backends agree.
//!
//! Cases are written once, against [`AsyncExecutor`]. Sync drivers run them
//! through [`Blocking`], which still calls the driver's own `Executor::execute`.

pub mod fixture;

use std::fmt::Debug;
use std::future::Future;

use rupa::core::dialect::Dialect;
use rupa::core::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use rupa::core::ir::{DslFnDef, ExprNode, Statement};
use rupa::core::tx::{
    AsyncTransaction, AsyncTransactional, IsolationLevel, Transaction, Transactional, TxOptions,
};
use rupa::core::{DialectId, DslError, Expect, ResultError, Value};
use rupa::prelude::*;

pub use fixture::{Item, NewNote, Note, NotePatch, POSTGRES_DDL, SetPinned, items};

/// Gives the suite a fresh, empty schema per case.
pub trait Harness {
    type Exec: AsyncExecutor;

    /// Drops and recreates the conformance tables, empty.
    fn reset(&mut self) -> impl Future<Output = ()>;

    fn exec(&mut self) -> &mut Self::Exec;
}

/// Runs a sync [`Executor`] as an [`AsyncExecutor`] by completing immediately.
#[derive(Debug)]
pub struct Blocking<E>(pub E);

impl<E: Executor + Send> AsyncExecutor for Blocking<E> {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        self.0.dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        std::future::ready(self.0.execute(statement, expect))
    }
}

impl<T: Transaction + Send> AsyncTransaction for Blocking<T> {
    fn commit(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.commit())
    }

    fn rollback(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.rollback())
    }
}

impl<E: Transactional + Send> AsyncTransactional for Blocking<E> {
    type Tx<'t>
        = Blocking<E::Tx<'t>>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Blocking<E::Tx<'_>>, E::Error>> + Send {
        std::future::ready(self.0.begin_with(options).map(Blocking))
    }
}

/// Runs every case, each on a freshly reset schema. The case name is printed
/// before it runs, so a failure's captured output says which case it was.
pub async fn run<H: Harness>(h: &mut H) {
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            h.reset().await;
            eprintln!("conformance {}", stringify!($case));
            cases::$case(h.exec()).await;
        )*};
    }
    cases!(
        case_insert_and_select,
        case_single_row_shapes,
        case_comparisons,
        case_three_valued_logic,
        case_in_lists,
        case_like_patterns,
        case_order_with_nulls,
        case_limit_offset,
        case_exists,
        case_update,
        case_delete,
        case_json_paths,
        case_nullable_json,
        case_timestamps,
        case_dsl_lowering_and_eval_agree,
        case_duplicate_key_is_an_error,
        case_failed_multi_row_insert_is_atomic,
        case_generated_ids_and_returning,
        case_get_by_id,
        case_update_entity_and_patches,
        case_delete_one_and_by_id,
    );
}

/// Runs the transaction cases. Needs a transactional executor.
pub async fn run_transactions<H: Harness>(h: &mut H)
where
    H::Exec: AsyncTransactional,
{
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            h.reset().await;
            eprintln!("conformance {}", stringify!($case));
            tx_cases::$case(h.exec()).await;
        )*};
    }
    cases!(
        tx_commit_persists,
        tx_rollback_discards,
        tx_drop_rolls_back,
        tx_is_an_executor,
        tx_nested_rollback_keeps_outer,
        tx_nested_drop_keeps_outer,
        tx_nested_commit_outer_rollback,
        tx_closure_commits_or_rolls_back,
        tx_read_only_rejects_writes,
        tx_options_on_nested_are_rejected,
        tx_isolation_levels,
        tx_sequential_after_drop,
    );
}

/// `upper(a) = upper(b)`: lowered for SQL dialects, evaluated by the memory backend.
pub static CI_EQ: DslFnDef = DslFnDef {
    name: "ci_eq",
    lower: lower_ci_eq,
    eval: Some(eval_ci_eq),
};

fn lower_ci_eq(dialect: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    if args.len() != 2 {
        return Err(DslError::Arity {
            function: "ci_eq",
            expected: 2,
            got: args.len(),
        });
    }
    let (b, a) = (args.pop().unwrap(), args.pop().unwrap());
    match dialect.id() {
        DialectId::Postgres => Ok(ExprNode::Binary(
            rupa::core::ir::BinOp::Eq,
            Box::new(ExprNode::Call("upper", vec![a])),
            Box::new(ExprNode::Call("upper", vec![b])),
        )),
        other => Err(DslError::unsupported("ci_eq", other)),
    }
}

fn eval_ci_eq(args: &[Value]) -> Result<Value, DslError> {
    match args {
        [Value::Text(a), Value::Text(b)] => Ok(Value::Bool(a.to_uppercase() == b.to_uppercase())),
        [Value::Null(_), _] | [_, Value::Null(_)] => Ok(Value::Null(rupa::core::SqlType::Bool)),
        other => Err(DslError::Other(format!(
            "ci_eq: unexpected arguments {other:?}"
        ))),
    }
}

fn result_error<T: Debug, E: ExecError>(r: Result<T, E>) -> ResultError {
    match r {
        Ok(v) => panic!("expected a result error, got Ok({v:?})"),
        Err(e) => e
            .result_error()
            .cloned()
            .unwrap_or_else(|| panic!("expected a result error, got {e}")),
    }
}

mod cases {
    use super::*;

    async fn seed<E: AsyncExecutor>(e: &mut E) {
        let all = items();
        let n = e
            .run(insert::<Item>().values(&all).affected())
            .await
            .expect("seed");
        assert_eq!(n, all.len() as u64);
    }

    async fn ids<E: AsyncExecutor>(e: &mut E, cond: Expr<bool>) -> Vec<i64> {
        let rows = e
            .run(
                select::<Item>()
                    .filter(cond)
                    .order_by(col!(Item::id).asc())
                    .all(),
            )
            .await
            .expect("select");
        rows.into_iter().map(|i| i.id).collect()
    }

    pub async fn case_insert_and_select<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let all = e
            .run(select::<Item>().order_by(col!(Item::id).asc()).all())
            .await
            .unwrap();
        assert_eq!(all, items(), "rows must round-trip exactly");
    }

    pub async fn case_single_row_shapes<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let one = e
            .run(select::<Item>().filter(col!(Item::id).eq(2)).one())
            .await
            .unwrap();
        assert_eq!(one.name, "banana");
        let none = e
            .run(select::<Item>().filter(col!(Item::id).eq(99)).optional())
            .await
            .unwrap();
        assert_eq!(none, None);
        let some = e
            .run(select::<Item>().filter(col!(Item::id).eq(3)).optional())
            .await
            .unwrap();
        assert_eq!(some.map(|i| i.id), Some(3));

        let r = e
            .run(select::<Item>().filter(col!(Item::id).eq(99)).one())
            .await;
        assert_eq!(result_error(r), ResultError::NotFound);
        let r = e
            .run(select::<Item>().filter(col!(Item::active).eq(true)).one())
            .await;
        assert_eq!(result_error(r), ResultError::TooManyRows);
        let r = e
            .run(
                select::<Item>()
                    .filter(col!(Item::active).eq(true))
                    .optional(),
            )
            .await;
        assert_eq!(result_error(r), ResultError::TooManyRows);
    }

    pub async fn case_comparisons<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(ids(e, col!(Item::qty).eq(7)).await, [3]);
        assert_eq!(ids(e, col!(Item::qty).ne(7)).await, [1, 2, 4]);
        assert_eq!(ids(e, col!(Item::qty).lt(7)).await, [1, 4]);
        assert_eq!(ids(e, col!(Item::qty).le(7)).await, [1, 3, 4]);
        assert_eq!(ids(e, col!(Item::qty).gt(3)).await, [2, 3]);
        assert_eq!(ids(e, col!(Item::qty).ge(3)).await, [1, 2, 3]);
        assert_eq!(ids(e, col!(Item::name).gt("banana")).await, [3, 4]);
        assert_eq!(
            ids(e, col!(Item::active).eq(false).or(col!(Item::qty).eq(0))).await,
            [2, 4]
        );
    }

    pub async fn case_three_valued_logic<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        // NULL = 'red' is NULL, and NOT NULL is still NULL: neither keeps rows 2 and 4.
        assert_eq!(ids(e, col!(Item::label).eq("red")).await, [1]);
        assert_eq!(ids(e, !col!(Item::label).eq("red")).await, [3]);
        assert_eq!(ids(e, col!(Item::label).is_null()).await, [2, 4]);
        assert_eq!(ids(e, col!(Item::label).is_not_null()).await, [1, 3]);
        // NULL OR TRUE is TRUE; NULL AND FALSE is FALSE.
        assert_eq!(
            ids(e, col!(Item::label).eq("x").or(col!(Item::qty).eq(12))).await,
            [2]
        );
        assert_eq!(
            ids(e, !(col!(Item::label).eq("x").and(col!(Item::qty).eq(-5)))).await,
            [1, 2, 3, 4]
        );
    }

    pub async fn case_in_lists<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(ids(e, col!(Item::id).in_([1i64, 3, 99])).await, [1, 3]);
        assert_eq!(ids(e, col!(Item::id).not_in([1i64, 3])).await, [2, 4]);
        assert_eq!(
            ids(e, col!(Item::id).in_(Vec::<i64>::new())).await,
            Vec::<i64>::new()
        );
        assert_eq!(
            ids(e, col!(Item::id).not_in(Vec::<i64>::new())).await,
            [1, 2, 3, 4]
        );
        // x NOT IN (1, NULL) is never TRUE.
        let null =
            Expr::<Option<i64>>::from_node(ExprNode::Param(Value::Null(rupa::core::SqlType::I64)));
        assert_eq!(
            ids(
                e,
                col!(Item::id).not_in([Expr::from_node(ExprNode::Param(Value::I64(1))), null])
            )
            .await,
            Vec::<i64>::new()
        );
    }

    pub async fn case_like_patterns<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(ids(e, col!(Item::name).like("%an%")).await, [2]);
        assert_eq!(ids(e, col!(Item::name).like("_pple")).await, [1]);
        assert_eq!(ids(e, col!(Item::label).like("dark\\_%")).await, [3]);
        assert_eq!(ids(e, col!(Item::label).like("%")).await, [1, 3]);
    }

    pub async fn case_order_with_nulls<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let asc = e
            .run(
                select::<Item>()
                    .order_by(col!(Item::label).asc())
                    .order_by(col!(Item::id).asc())
                    .all(),
            )
            .await
            .unwrap();
        assert_eq!(
            asc.iter().map(|i| i.id).collect::<Vec<_>>(),
            [3, 1, 2, 4],
            "NULLS LAST ascending"
        );
        let desc = e
            .run(
                select::<Item>()
                    .order_by(col!(Item::label).desc())
                    .order_by(col!(Item::id).desc())
                    .all(),
            )
            .await
            .unwrap();
        assert_eq!(
            desc.iter().map(|i| i.id).collect::<Vec<_>>(),
            [4, 2, 1, 3],
            "NULLS FIRST descending"
        );
    }

    pub async fn case_limit_offset<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let page = e
            .run(
                select::<Item>()
                    .order_by(col!(Item::id).asc())
                    .limit(2)
                    .offset(1)
                    .all(),
            )
            .await
            .unwrap();
        assert_eq!(page.iter().map(|i| i.id).collect::<Vec<_>>(), [2, 3]);
        let past_end = e
            .run(
                select::<Item>()
                    .order_by(col!(Item::id).asc())
                    .offset(10)
                    .all(),
            )
            .await
            .unwrap();
        assert!(past_end.is_empty());
        let zero = e.run(select::<Item>().limit(0).all()).await.unwrap();
        assert!(zero.is_empty());
    }

    pub async fn case_exists<E: AsyncExecutor>(e: &mut E) {
        assert!(
            !e.run(select::<Item>().exists()).await.unwrap(),
            "empty table"
        );
        seed(e).await;
        assert!(
            e.run(
                select::<Item>()
                    .filter(col!(Item::name).eq("cherry"))
                    .exists()
            )
            .await
            .unwrap()
        );
        assert!(
            !e.run(select::<Item>().filter(col!(Item::name).eq("fig")).exists())
                .await
                .unwrap()
        );
    }

    pub async fn case_update<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let meta = fixture::Meta {
            score: 100,
            flag: false,
            tags: vec!["new".into()],
            nested: None,
            note: Some("n".into()),
        };
        let n = e
            .run(
                update::<Item>()
                    .set(col!(Item::meta), &meta)
                    .set_expr(col!(Item::label), col!(Item::name))
                    .filter(col!(Item::active).eq(true))
                    .affected(),
            )
            .await
            .unwrap();
        assert_eq!(n, 3);
        let rows = e
            .run(select::<Item>().order_by(col!(Item::id).asc()).all())
            .await
            .unwrap();
        for item in &rows {
            if item.active {
                assert_eq!(item.meta, meta);
                assert_eq!(
                    item.label.as_deref(),
                    Some(item.name.as_str()),
                    "SET sees the pre-update row"
                );
            } else {
                assert_eq!(item, &items()[1]);
            }
        }
        let any = e
            .run(
                update::<Item>()
                    .set(col!(Item::qty), &1)
                    .filter(col!(Item::id).eq(99))
                    .build::<bool>(),
            )
            .await
            .unwrap();
        assert!(!any);
    }

    pub async fn case_delete<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(
            e.run(delete::<Item>().filter(col!(Item::qty).lt(5)).affected())
                .await
                .unwrap(),
            2
        );
        assert_eq!(ids(e, col!(Item::id).gt(0)).await, [2, 3]);
        assert!(
            !e.run(
                delete::<Item>()
                    .filter(col!(Item::id).eq(1))
                    .build::<bool>()
            )
            .await
            .unwrap()
        );
        assert_eq!(e.run(delete::<Item>().affected()).await.unwrap(), 2);
    }

    pub async fn case_json_paths<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(
            ids(e, col!(Item::meta).path("nested").key("k").text().eq("w")).await,
            [3]
        );
        assert_eq!(
            ids(e, col!(Item::meta).path("tags").index(0).text().eq("a")).await,
            [1]
        );
        assert_eq!(
            ids(e, col!(Item::meta).path("flag").cast::<bool>().eq(true)).await,
            [1, 3]
        );
        assert_eq!(
            ids(e, col!(Item::meta).path("score").cast::<i64>().gt(6)).await,
            [2, 3]
        );
        // A missing key and a JSON null are both SQL NULL under ->>.
        assert_eq!(
            ids(e, col!(Item::meta).path("missing").text().is_null()).await,
            [1, 2, 3, 4]
        );
        assert_eq!(
            ids(e, col!(Item::meta).path("note").text().is_null()).await,
            [1, 2, 3, 4]
        );
        assert_eq!(
            ids(e, col!(Item::meta).path("nested").key("k").text().is_null()).await,
            [1, 4]
        );
        // ...but under ->, a JSON null is a value, not SQL NULL.
        let note_json = Expr::<bool>::from_node(ExprNode::Unary(
            rupa::core::ir::UnOp::IsNull,
            Box::new(col!(Item::meta).path("note").json().into_node()),
        ));
        assert_eq!(ids(e, note_json).await, Vec::<i64>::new());
    }

    pub async fn case_nullable_json<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        assert_eq!(
            ids(e, col!(Item::extra).is_null()).await,
            [1, 3, 4],
            "None is SQL NULL, not JSON null"
        );
        assert_eq!(
            ids(e, col!(Item::extra).path("score").cast::<i64>().eq(1)).await,
            [2]
        );
        e.run(
            update::<Item>()
                .set(col!(Item::extra), &None)
                .filter(col!(Item::id).eq(2))
                .affected(),
        )
        .await
        .unwrap();
        assert_eq!(
            ids(e, col!(Item::extra).is_not_null()).await,
            Vec::<i64>::new()
        );
    }

    pub async fn case_timestamps<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let cutoff = items()[1].created_at;
        assert_eq!(ids(e, col!(Item::created_at).gt(cutoff)).await, [3, 4]);
        let latest = e
            .run(
                select::<Item>()
                    .order_by(col!(Item::created_at).desc())
                    .limit(1)
                    .one(),
            )
            .await
            .unwrap();
        assert_eq!(latest.created_at, items()[3].created_at);
    }

    pub async fn case_dsl_lowering_and_eval_agree<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let ci = |s: &str| {
            Expr::<bool>::dsl(
                &CI_EQ,
                vec![
                    ExprNode::Column(col!(Item::name).column_ref()),
                    ExprNode::Param(Value::Text(s.into())),
                ],
            )
        };
        assert_eq!(ids(e, ci("CHERRY")).await, [3]);
        assert_eq!(ids(e, !ci("Cherry")).await, [1, 2, 4]);
    }

    pub async fn case_duplicate_key_is_an_error<E: AsyncExecutor>(e: &mut E) {
        seed(e).await;
        let r = e.run(insert::<Item>().value(&items()[0]).affected()).await;
        assert!(r.is_err(), "duplicate primary key must fail");
    }

    pub async fn case_failed_multi_row_insert_is_atomic<E: AsyncExecutor>(e: &mut E) {
        let mut batch = items();
        batch.push(items()[0].clone());
        assert!(
            e.run(insert::<Item>().values(&batch).affected())
                .await
                .is_err()
        );
        assert!(
            !e.run(select::<Item>().exists()).await.unwrap(),
            "no partial insert"
        );
    }
    fn new_note(title: &str) -> NewNote {
        NewNote {
            title: title.into(),
            pinned: false,
        }
    }

    pub async fn case_generated_ids_and_returning<E: AsyncExecutor>(e: &mut E) {
        let first = e
            .run(insert::<Note>().value(&new_note("first")).returning_one())
            .await
            .unwrap();
        assert_eq!(
            first,
            Note {
                id: 1,
                title: "first".into(),
                body: None,
                pinned: false
            }
        );

        let more = e
            .run(
                insert::<Note>()
                    .values(&[new_note("b"), new_note("c")])
                    .returning_all(),
            )
            .await
            .unwrap();
        assert_eq!(more.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 3]);

        // One model: the entity inserts itself; its generated id is ignored.
        let draft = Note {
            id: 0,
            title: "draft".into(),
            body: Some("text".into()),
            pinned: true,
        };
        let saved = e
            .run(insert::<Note>().value(&draft).returning_one())
            .await
            .unwrap();
        assert_eq!(saved, Note { id: 4, ..draft });

        let n = e
            .run(insert::<Note>().value(&new_note("plain")).affected())
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    pub async fn case_get_by_id<E: AsyncExecutor>(e: &mut E) {
        e.run(
            insert::<Note>()
                .values(&[new_note("a"), new_note("b")])
                .affected(),
        )
        .await
        .unwrap();
        let b = e.run(get::<Note>(&2)).await.unwrap();
        assert_eq!(b.map(|n| n.title), Some("b".to_string()));
        assert_eq!(e.run(get::<Note>(&99)).await.unwrap(), None);
    }

    pub async fn case_update_entity_and_patches<E: AsyncExecutor>(e: &mut E) {
        let mut a = e
            .run(insert::<Note>().value(&new_note("a")).returning_one())
            .await
            .unwrap();
        e.run(insert::<Note>().value(&new_note("b")).affected())
            .await
            .unwrap();

        // Full entity, by id.
        a.title = "a2".into();
        a.body = Some("body".into());
        assert_eq!(e.run(update::<Note>().one(&a).affected()).await.unwrap(), 1);
        assert_eq!(e.run(get::<Note>(&1)).await.unwrap(), Some(a.clone()));

        // Keyed patch: `None` fields stay as they are.
        let patch = NotePatch {
            id: 1,
            body: Some(None),
            pinned: None,
        };
        e.run(update::<Note>().one(&patch).affected())
            .await
            .unwrap();
        let got = e.run(get::<Note>(&1)).await.unwrap().unwrap();
        assert_eq!(got, Note { body: None, ..a });

        // Unkeyed patch over a filter.
        let n = e
            .run(
                update::<Note>()
                    .apply(&SetPinned { pinned: Some(true) })
                    .filter(col!(Note::id).gt(0))
                    .affected(),
            )
            .await
            .unwrap();
        assert_eq!(n, 2);
        let pinned = e
            .run(select::<Note>().filter(col!(Note::pinned).eq(true)).all())
            .await
            .unwrap();
        assert_eq!(pinned.len(), 2);
    }

    pub async fn case_delete_one_and_by_id<E: AsyncExecutor>(e: &mut E) {
        let all = e
            .run(
                insert::<Note>()
                    .values(&[new_note("a"), new_note("b"), new_note("c")])
                    .returning_all(),
            )
            .await
            .unwrap();
        assert_eq!(
            e.run(delete::<Note>().one(&all[0]).affected())
                .await
                .unwrap(),
            1
        );
        assert!(
            e.run(delete::<Note>().by_id(&2).build::<bool>())
                .await
                .unwrap()
        );
        assert!(
            !e.run(delete::<Note>().by_id(&2).build::<bool>())
                .await
                .unwrap()
        );
        let left = e.run(select::<Note>().all()).await.unwrap();
        assert_eq!(left.iter().map(|n| n.id).collect::<Vec<_>>(), [3]);
    }
}

mod tx_cases {
    use super::*;

    fn note(title: &str) -> NewNote {
        NewNote {
            title: title.into(),
            pinned: false,
        }
    }

    async fn titles<E: AsyncExecutor>(e: &mut E) -> Vec<String> {
        let notes = e
            .run(select::<Note>().order_by(col!(Note::id).asc()).all())
            .await
            .expect("select");
        notes.into_iter().map(|n| n.title).collect()
    }

    async fn add<E: AsyncExecutor>(e: &mut E, title: &str) {
        e.run(insert::<Note>().value(&note(title)).affected())
            .await
            .expect("insert");
    }

    /// An error type for closure transactions: a database error, or a
    /// deliberate abort.
    #[derive(Debug)]
    enum Abort<X> {
        Db(#[allow(dead_code)] X),
        Explicit,
    }

    impl<X> From<X> for Abort<X> {
        fn from(e: X) -> Self {
            Abort::Db(e)
        }
    }

    pub async fn tx_commit_persists<E: AsyncTransactional>(e: &mut E) {
        let mut tx = e.begin().await.unwrap();
        add(&mut tx, "a").await;
        tx.commit().await.unwrap();
        assert_eq!(titles(e).await, ["a"]);
    }

    pub async fn tx_rollback_discards<E: AsyncTransactional>(e: &mut E) {
        add(e, "kept").await;
        let mut tx = e.begin().await.unwrap();
        add(&mut tx, "gone").await;
        assert_eq!(
            titles(&mut tx).await,
            ["kept", "gone"],
            "a tx sees its own writes"
        );
        tx.rollback().await.unwrap();
        assert_eq!(titles(e).await, ["kept"]);
    }

    pub async fn tx_drop_rolls_back<E: AsyncTransactional>(e: &mut E) {
        {
            let mut tx = e.begin().await.unwrap();
            add(&mut tx, "gone").await;
        }
        assert!(
            titles(e).await.is_empty(),
            "a dropped tx must not leave its writes behind"
        );
    }

    pub async fn tx_is_an_executor<E: AsyncTransactional>(e: &mut E) {
        /// Generic over any async executor: works on a connection or a tx.
        async fn count<X: AsyncExecutor>(x: &mut X) -> usize {
            x.run(select::<Note>().all()).await.unwrap().len()
        }
        let mut tx = e.begin().await.unwrap();
        add(&mut tx, "a").await;
        assert_eq!(count(&mut tx).await, 1);
        tx.commit().await.unwrap();
        assert_eq!(count(e).await, 1);
    }

    pub async fn tx_nested_rollback_keeps_outer<E: AsyncTransactional>(e: &mut E) {
        let mut outer = e.begin().await.unwrap();
        add(&mut outer, "outer").await;
        let mut inner = outer.begin().await.unwrap();
        add(&mut inner, "inner").await;
        inner.rollback().await.unwrap();
        add(&mut outer, "after").await;
        outer.commit().await.unwrap();
        assert_eq!(titles(e).await, ["outer", "after"]);
    }

    pub async fn tx_nested_drop_keeps_outer<E: AsyncTransactional>(e: &mut E) {
        let mut outer = e.begin().await.unwrap();
        add(&mut outer, "outer").await;
        {
            let mut inner = outer.begin().await.unwrap();
            add(&mut inner, "inner").await;
        }
        assert_eq!(
            titles(&mut outer).await,
            ["outer"],
            "the dropped savepoint is rolled back"
        );
        outer.commit().await.unwrap();
        assert_eq!(titles(e).await, ["outer"]);
    }

    pub async fn tx_nested_commit_outer_rollback<E: AsyncTransactional>(e: &mut E) {
        let mut outer = e.begin().await.unwrap();
        let mut inner = outer.begin().await.unwrap();
        add(&mut inner, "inner").await;
        inner.commit().await.unwrap();
        assert_eq!(titles(&mut outer).await, ["inner"]);
        outer.rollback().await.unwrap();
        assert!(
            titles(e).await.is_empty(),
            "committing a savepoint does not commit the outer tx"
        );
    }

    pub async fn tx_closure_commits_or_rolls_back<E: AsyncTransactional>(e: &mut E) {
        let ok: Result<u64, Abort<E::Error>> = e
            .transaction(async |tx: &mut E::Tx<'_>| {
                Ok(tx
                    .run(insert::<Note>().value(&note("ok")).affected())
                    .await?)
            })
            .await;
        assert_eq!(ok.unwrap(), 1);

        let aborted: Result<(), Abort<E::Error>> = e
            .transaction(async |tx: &mut E::Tx<'_>| {
                tx.run(insert::<Note>().value(&note("aborted")).affected())
                    .await?;
                Err(Abort::Explicit)
            })
            .await;
        assert!(matches!(aborted, Err(Abort::Explicit)));
        assert_eq!(titles(e).await, ["ok"]);
    }

    pub async fn tx_read_only_rejects_writes<E: AsyncTransactional>(e: &mut E) {
        add(e, "a").await;
        let mut tx = e
            .begin_with(TxOptions::default().read_only())
            .await
            .unwrap();
        assert_eq!(titles(&mut tx).await, ["a"], "reads work");
        let write = tx.run(insert::<Note>().value(&note("b")).affected()).await;
        assert!(
            write.is_err(),
            "writes in a read-only transaction must fail"
        );
        tx.rollback().await.unwrap();
        assert_eq!(titles(e).await, ["a"]);
    }

    pub async fn tx_options_on_nested_are_rejected<E: AsyncTransactional>(e: &mut E) {
        let mut tx = e.begin().await.unwrap();
        let rejected = tx
            .begin_with(TxOptions::default().read_only())
            .await
            .is_err();
        assert!(
            rejected,
            "options on a savepoint must be an error, not ignored"
        );
        tx.rollback().await.unwrap();
    }

    pub async fn tx_isolation_levels<E: AsyncTransactional>(e: &mut E) {
        for level in [
            IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable,
        ] {
            let mut tx = e
                .begin_with(TxOptions::default().isolation(level))
                .await
                .unwrap();
            add(&mut tx, "x").await;
            tx.commit().await.unwrap();
        }
        assert_eq!(titles(e).await.len(), 3);
    }

    pub async fn tx_sequential_after_drop<E: AsyncTransactional>(e: &mut E) {
        drop(e.begin().await.unwrap());
        let mut tx = e.begin().await.unwrap();
        add(&mut tx, "second").await;
        tx.commit().await.unwrap();
        let mut tx = e.begin().await.unwrap();
        let mut inner = tx.begin().await.unwrap();
        add(&mut inner, "nested").await;
        drop(inner);
        drop(tx);
        assert_eq!(titles(e).await, ["second"]);
    }
}
