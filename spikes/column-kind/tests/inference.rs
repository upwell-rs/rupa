#![allow(
    clippy::needless_borrow,
    reason = "the borrows are the autoref mechanism under test"
)]

use chrono::{DateTime, Utc};
use rupa_spike_column_kind::__private::{JsonLevel as _, Probe, ScalarLevel as _, kind_of};
use rupa_spike_column_kind::{ColumnKind, ColumnMeta, Entity, JsonPath, Value, col};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Prefs {
    theme: String,
    beta: bool,
}

#[derive(Debug, PartialEq, Entity)]
#[entity(table = "users")]
struct User {
    #[id]
    id: i64,
    email: String,            // ScalarColumn *and* Serialize -> scalar must win
    nickname: Option<String>, // nullable scalar
    #[column(name = "created")]
    created_at: DateTime<Utc>,
    token: uuid::Uuid,
    avatar: Vec<u8>,          // bytes, not a JSON array
    prefs: Prefs,             // serde only -> inferred JSON
    old_prefs: Option<Prefs>, // nullable JSON: None is SQL NULL, not JSON null
    tags: Vec<String>,        // serde only -> JSON
    #[column(json)]
    forced: String, // explicit override beats inference
}

fn meta(field: &'static str) -> ColumnMeta {
    User::columns()
        .into_iter()
        .find(|c| c.field == field)
        .unwrap()
}

#[test]
fn kinds_are_inferred() {
    use ColumnKind::*;
    let expect = [
        ("id", Scalar, false),
        ("email", Scalar, false),
        ("nickname", Scalar, true),
        ("created_at", Scalar, false),
        ("token", Scalar, false),
        ("avatar", Scalar, false),
        ("prefs", Json, false),
        ("old_prefs", Json, true),
        ("tags", Json, false),
        ("forced", Json, false),
    ];
    for (field, kind, nullable) in expect {
        let m = meta(field);
        assert_eq!((m.kind, m.nullable), (kind, nullable), "field {field}");
    }
    assert_eq!(meta("created_at").name, "created");
}

fn sample() -> User {
    User {
        id: 7,
        email: "a@b.c".into(),
        nickname: None,
        created_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        token: uuid::Uuid::nil(),
        avatar: vec![1, 2, 3],
        prefs: Prefs {
            theme: "dark".into(),
            beta: true,
        },
        old_prefs: None,
        tags: vec!["x".into()],
        forced: "plain".into(),
    }
}

#[test]
fn values_roundtrip() {
    let u = sample();
    let vals = u.to_values();
    assert_eq!(vals[1], Value::Text("a@b.c".into()));
    assert_eq!(vals[2], Value::Null);
    assert_eq!(vals[5], Value::Bytes(vec![1, 2, 3]));
    assert_eq!(
        vals[6],
        Value::Json(serde_json::json!({"theme": "dark", "beta": true}))
    );
    assert_eq!(
        vals[7],
        Value::Null,
        "None in Option<Json> must be SQL NULL"
    );
    assert_eq!(vals[9], Value::Json(serde_json::json!("plain")));
    assert_eq!(User::from_values(vals).unwrap(), u);
}

#[test]
fn typed_handles_carry_kind_in_the_type() {
    let prefs = col!(User::prefs);
    assert_eq!(prefs.kind(), ColumnKind::Json);
    assert_eq!(
        prefs.path("theme"),
        JsonPath {
            column: "prefs",
            path: vec!["theme"]
        }
    );

    // The handle type is nameable after the fact, just not computable in a type position.
    let created: rupa_spike_column_kind::Column<
        User,
        DateTime<Utc>,
        rupa_spike_column_kind::Scalar,
    > = col!(User::created_at);
    assert_eq!(created.name(), "created");
}

/// Documents the hazard the derive's generic-parameter check guards against:
/// inside generic code, dispatch follows the *bounds in scope*, not the type
/// the code is eventually instantiated with.
#[test]
fn dispatch_in_generic_context_follows_bounds() {
    fn kind_for<T: Serialize + for<'de> Deserialize<'de>>() -> ColumnKind {
        kind_of::<T, _>(&(&&&Probe::<(), T>::new("x")).__rupa_codec())
    }
    assert_eq!(kind_for::<i64>(), ColumnKind::Json); // i64 is ScalarColumn, yet JSON was picked

    let concrete = kind_of::<i64, _>(&(&&&Probe::<(), i64>::new("x")).__rupa_codec());
    assert_eq!(concrete, ColumnKind::Scalar);
}

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}

/// Documents the main semantic hazard of inference: a serde newtype over a
/// scalar compiles fine and becomes a JSON column.
#[test]
fn serde_newtype_over_scalar_is_inferred_json() {
    #[derive(Serialize, Deserialize)]
    #[serde(transparent)]
    struct Email(String);

    #[derive(Entity)]
    #[entity(table = "t")]
    struct T {
        #[id]
        id: i64,
        email: Email,
    }
    assert_eq!(col!(T::email).kind(), ColumnKind::Json);
    let _ = T {
        id: 1,
        email: Email(String::new()),
    }
    .id;
}
