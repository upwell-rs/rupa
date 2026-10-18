//! The render cache: every cached render must equal an uncached one, with the
//! caller's own values bound.

#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use chrono::{TimeZone, Utc};
use fixtures::{Prefs, User};
use rupa_core::dialect::{MySql, Postgres, Sqlite};
use rupa_core::ir::{
    BinOp, ColumnRef, Direction, DslCall, DslFnDef, ExprNode, FromClause, OrderBy, Select,
    Statement, TableRef,
};
use rupa_core::prelude::*;
use rupa_core::{Dialect, DslError, SecurityContext, Value};
use rupa_sql::cache::MAX_CACHED_SQL_LEN;
use rupa_sql::{CacheStats, RenderCache, render};

const DIALECTS: [&dyn Dialect; 3] = [&Postgres, &MySql, &Sqlite];

/// Renders through `cache` and checks the result against the uncached
/// renderer, text and values.
fn check(cache: &RenderCache, statement: &Statement, dialect: &dyn Dialect) -> String {
    let cached = cache.render(statement, dialect).expect("cached render");
    let plain = render(statement, dialect).expect("render");
    assert_eq!(&*cached.sql, plain.sql, "SQL differs for {statement:?}");
    assert_eq!(
        cached.params, plain.params,
        "params differ for {statement:?}"
    );
    plain.sql
}

fn by_email(email: &str) -> Statement {
    select::<User>()
        .filter(User::EMAIL.eq(email).and(User::ACTIVE.eq(true)))
        .limit(10)
        .all()
        .into_statement()
}

fn stats(cache: &RenderCache) -> (u64, u64, u64) {
    let CacheStats {
        hits,
        misses,
        bypassed,
        ..
    } = cache.stats();
    (hits, misses, bypassed)
}

#[test]
fn same_shape_reuses_sql_and_binds_new_values() {
    let cache = RenderCache::new(64);
    let first = cache.render(&by_email("a@x"), &Postgres).unwrap();
    let second = cache.render(&by_email("b@x"), &Postgres).unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first.sql, &second.sql),
        "one shared text"
    );
    assert_eq!(first.params[0], Value::Text("a@x".into()));
    assert_eq!(
        second.params[0],
        Value::Text("b@x".into()),
        "values are never cached"
    );
    assert_eq!(stats(&cache), (1, 1, 0));
    check(&cache, &by_email("c@x"), &Postgres);
}

#[test]
fn different_shapes_and_dialects_get_their_own_entries() {
    // Roomy: shards fill unevenly, and this test wants no evictions.
    let cache = RenderCache::new(1024);
    let statements = [
        by_email("a"),
        select::<User>()
            .filter(User::EMAIL.ne("a"))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::ID.in_([1, 2]))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::ID.in_([1, 2, 3]))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::ID.in_(Vec::<i64>::new()))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::NICKNAME.is_null())
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::PREFS.path("theme").cast::<String>().eq("dark"))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::PREFS.path("beta").cast::<bool>().eq(true))
            .all()
            .into_statement(),
        select::<User>()
            .filter(User::ID.gt(1))
            .order_by(User::NICKNAME.desc())
            .offset(5)
            .all()
            .into_statement(),
        delete::<User>().by_id(&7).affected().into_statement(),
        update::<User>()
            .set(User::EMAIL, &"z".to_string())
            .filter(User::ID.eq(1))
            .affected()
            .into_statement(),
        raw()
            .sql("SELECT 1 WHERE 1 = ")
            .bind(1i64)
            .rows::<Vec<User>>()
            .into_statement(),
    ];
    for dialect in DIALECTS {
        let mut seen = std::collections::HashSet::new();
        for s in &statements {
            let sql = check(&cache, s, dialect);
            assert!(seen.insert(sql), "distinct shapes render distinctly");
        }
    }
    let (hits, misses, _) = stats(&cache);
    assert_eq!((hits, misses), (0, (statements.len() * 3) as u64));
    for dialect in DIALECTS {
        for s in &statements {
            check(&cache, s, dialect);
        }
    }
    assert_eq!(
        stats(&cache).0,
        (statements.len() * 3) as u64,
        "second round all hits"
    );
}

#[test]
fn multi_row_inserts_bind_every_row() {
    let cache = RenderCache::new(64);
    let user = |id: i64| User {
        id,
        email: format!("{id}@x"),
        nickname: (id % 2 == 0).then(|| format!("n{id}")),
        active: true,
        created_at: Utc.timestamp_opt(id, 0).unwrap(),
        prefs: Prefs {
            theme: "dark".into(),
            beta: false,
        },
        old_prefs: None,
    };
    for dialect in DIALECTS {
        let a = insert::<User>()
            .values(&[user(1), user(2)])
            .affected()
            .into_statement();
        let b = insert::<User>()
            .values(&[user(3), user(4)])
            .affected()
            .into_statement();
        check(&cache, &a, dialect);
        check(&cache, &b, dialect);
    }
    assert_eq!(stats(&cache).0, 3);
}

/// MySQL emulates `NULLS LAST` by rendering a sort key twice, so a parameter
/// inside it fills two placeholders.
#[test]
fn a_parameter_rendered_twice_is_bound_twice() {
    let order = |n: i64| {
        Statement::Select(Select {
            from: FromClause::Table(TableRef::new("t")),
            projection: vec![ColumnRef::new("a")],
            filter: None,
            order_by: vec![OrderBy {
                expr: ExprNode::Binary(
                    BinOp::Eq,
                    Box::new(ExprNode::Column(ColumnRef::new("a"))),
                    Box::new(ExprNode::Param(Value::I64(n))),
                ),
                direction: Direction::Asc,
            }],
            limit: None,
            offset: None,
        })
    };
    let cache = RenderCache::new(8);
    check(&cache, &order(1), &MySql);
    let hit = cache.render(&order(2), &MySql).unwrap();
    assert_eq!(hit.params, [Value::I64(2), Value::I64(2)]);
    check(&cache, &order(3), &MySql);
}

/// A lowering that picks its SQL by looking at a value: shapes are taken after
/// lowering, so each variant gets its own entry instead of a wrong hit.
fn lower_eq_or_any(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    let value = args.pop().unwrap();
    let column = args.pop().unwrap();
    Ok(match value {
        ExprNode::Param(Value::Text(s)) if s == "*" => ExprNode::Param(Value::Bool(true)),
        value => ExprNode::Binary(BinOp::Eq, Box::new(column), Box::new(value)),
    })
}
static EQ_OR_ANY: DslFnDef = DslFnDef {
    name: "eq_or_any",
    lower: lower_eq_or_any,
    eval: None,
};

#[test]
fn value_dependent_lowerings_get_separate_shapes() {
    let q = |v: &str| {
        Statement::Select(Select {
            from: FromClause::Table(TableRef::new("t")),
            projection: vec![ColumnRef::new("a")],
            filter: Some(ExprNode::Dsl(DslCall {
                def: &EQ_OR_ANY,
                args: vec![
                    ExprNode::Column(ColumnRef::new("a")),
                    ExprNode::Param(Value::Text(v.into())),
                ],
            })),
            order_by: vec![],
            limit: None,
            offset: None,
        })
    };
    // Room for both shapes even in one shard: which shard a shape lands in
    // depends on string addresses, so it changes from run to run.
    let cache = RenderCache::new(64);
    for v in ["x", "*", "y", "*"] {
        check(&cache, &q(v), &Postgres);
    }
    assert_eq!(stats(&cache), (2, 2, 0));
}

#[test]
fn security_contexts_and_huge_statements_bypass_the_cache() {
    let cache = RenderCache::new(8);
    let ctx = Statement::ApplySecurity(SecurityContext::new().set("tenant_id", "1"));
    check(&cache, &ctx, &Postgres);
    let many: Vec<i64> = (0..(MAX_CACHED_SQL_LEN as i64 / 4)).collect();
    let huge = select::<User>()
        .filter(User::ID.in_(many))
        .all()
        .into_statement();
    check(&cache, &huge, &Postgres);
    assert_eq!(stats(&cache), (0, 0, 2));
    assert_eq!(cache.stats().entries, 0);
}

#[test]
fn errors_are_returned_not_cached() {
    let cache = RenderCache::new(8);
    let returning = insert::<User>()
        .value(&User {
            id: 1,
            email: "a".into(),
            nickname: None,
            active: true,
            created_at: Utc.timestamp_opt(0, 0).unwrap(),
            prefs: Prefs {
                theme: "t".into(),
                beta: true,
            },
            old_prefs: None,
        })
        .returning_one()
        .into_statement();
    assert!(
        cache.render(&returning, &MySql).is_err(),
        "MySQL has no RETURNING"
    );
    assert!(cache.render(&returning, &MySql).is_err(), "still an error");
    assert_eq!(cache.stats().entries, 0);
    check(&cache, &returning, &Postgres);
}

#[test]
fn capacity_bounds_the_cache() {
    let cache = RenderCache::new(32);
    for n in 1..=200usize {
        let ids: Vec<i64> = (0..n as i64).collect();
        let s = select::<User>()
            .filter(User::ID.in_(ids))
            .all()
            .into_statement();
        check(&cache, &s, &Postgres);
    }
    // 16 shards of ceil(32 / 16) = 2.
    assert!(cache.stats().entries <= 32, "{:?}", cache.stats());
    cache.set_capacity(0);
    assert_eq!(cache.stats().entries, 0);
    check(&cache, &by_email("a"), &Postgres);
    assert_eq!(cache.stats().entries, 0, "capacity 0 caches nothing");
    cache.set_capacity(16);
    check(&cache, &by_email("a"), &Postgres);
    check(&cache, &by_email("b"), &Postgres);
    assert_eq!(cache.stats().hits, 1);
    cache.clear();
    assert_eq!(
        cache.stats(),
        CacheStats {
            capacity: 16,
            ..CacheStats::default()
        }
    );
}

#[test]
fn concurrent_renders_agree() {
    let cache = RenderCache::new(64);
    std::thread::scope(|s| {
        for t in 0..8 {
            let cache = &cache;
            s.spawn(move || {
                for i in 0..200 {
                    let email = format!("{t}-{i}");
                    for dialect in DIALECTS {
                        check(cache, &by_email(&email), dialect);
                    }
                }
            });
        }
    });
    let (hits, misses, _) = stats(&cache);
    assert_eq!(hits + misses, 8 * 200 * 3);
    assert!(misses <= 8 * 3, "each shape misses at most once per thread");
}
