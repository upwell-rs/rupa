//! Derived entities and capabilities, end to end on the memory backend.

use chrono::{DateTime, Utc};
use rupa::core::{ColumnKind, Entity as _, Generated, SqlType, Value};
use rupa::prelude::*;
use rupa_driver_memory::MemoryDb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Prefs {
    theme: String,
}

#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "users", schema = "app")]
struct User {
    #[id(generated)]
    id: i64,
    email: String,
    nickname: Option<String>,
    #[column(name = "created")]
    created_at: DateTime<Utc>,
    prefs: Prefs,
    old_prefs: Option<Prefs>,
    #[column(json)]
    raw_label: String,
    #[column(generated)]
    version: i32,
}

/// Typed insert input: no id, no generated version.
#[derive(Insertable)]
#[insertable(entity = User)]
struct NewUser {
    email: String,
    created_at: DateTime<Utc>,
    prefs: Prefs,
    raw_label: String,
}

/// Partial update of one user.
#[derive(Updatable)]
#[updatable(entity = User)]
struct UserPatch {
    #[id]
    id: i64,
    nickname: Option<Option<String>>,
    prefs: Option<Prefs>,
}

/// Partial update without an id: applied to a filter.
#[derive(Updatable)]
#[updatable(entity = User)]
struct Rename {
    email: Option<String>,
}

/// An entity with no capabilities: readable, never writable.
#[derive(Debug, Entity)]
#[entity(table = "audit")]
struct Audit {
    #[id]
    id: i64,
    message: String,
}

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

fn new_user(email: &str) -> NewUser {
    NewUser {
        email: email.into(),
        created_at: at(1_700_000_000),
        prefs: Prefs {
            theme: "dark".into(),
        },
        raw_label: "x".into(),
    }
}

fn db() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<User>();
    db
}

#[test]
fn entity_metadata() {
    assert_eq!(
        User::TABLE,
        rupa::core::ir::TableRef::with_schema("app", "users")
    );
    assert_eq!(User::ID_COLUMNS, ["id"]);
    let cols = User::COLUMNS;
    let names: Vec<_> = cols.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "id",
            "email",
            "nickname",
            "created",
            "prefs",
            "old_prefs",
            "raw_label",
            "version"
        ]
    );
    let kind = |n: &str| cols.iter().find(|c| c.name == n).unwrap();
    assert_eq!(
        (kind("prefs").kind, kind("prefs").sql_type),
        (ColumnKind::Json, SqlType::Json)
    );
    assert!(kind("old_prefs").nullable && kind("old_prefs").kind == ColumnKind::Json);
    assert_eq!(
        kind("raw_label").kind,
        ColumnKind::Json,
        "#[column(json)] overrides scalar inference"
    );
    assert_eq!(
        (
            kind("id").generated,
            kind("version").generated,
            kind("email").generated
        ),
        (Some(Generated::Identity), Some(Generated::Database), None)
    );
    assert!(kind("id").primary_key && !kind("email").primary_key);
    assert_eq!(
        User::RAW_LABEL.kind(),
        ColumnKind::Json,
        "the column handle respects the override too"
    );
    assert_eq!(User::CREATED_AT.name(), "created");
}

#[test]
fn companion_insert_returns_generated_values() {
    let db = db();
    let ada: User = insert::<User>()
        .value(&new_user("ada@x"))
        .returning_one()
        .execute(&db)
        .unwrap();
    let both: Vec<User> = insert::<User>()
        .values(&[new_user("b@x"), new_user("c@x")])
        .returning_all()
        .execute(&db)
        .unwrap();
    assert_eq!(ada.id, 1);
    assert_eq!(
        ada.version, 1,
        "generated integer columns are filled by the memory identity"
    );
    assert_eq!(both.iter().map(|u| u.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(ada.nickname, None, "omitted nullable fields are NULL");
}

#[test]
fn one_model_insert_skips_generated_columns() {
    let db = db();
    let draft = User {
        id: 0, // ignored: generated
        email: "e@x".into(),
        nickname: Some("e".into()),
        created_at: at(1),
        prefs: Prefs {
            theme: "light".into(),
        },
        old_prefs: None,
        raw_label: "l".into(),
        version: 0, // ignored: generated
    };
    let saved = insert::<User>()
        .value(&draft)
        .returning_one()
        .execute(&db)
        .unwrap();
    assert_eq!(
        saved,
        User {
            id: 1,
            version: 1,
            ..draft
        }
    );
}

#[test]
fn get_update_and_delete() {
    let db = db();
    let mut user = insert::<User>()
        .value(&new_user("ada@x"))
        .returning_one()
        .execute(&db)
        .unwrap();
    insert::<User>()
        .value(&new_user("bob@x"))
        .affected()
        .execute(&db)
        .unwrap();

    assert_eq!(get::<User>(&1).execute(&db).unwrap().as_ref(), Some(&user));
    assert_eq!(get::<User>(&99).execute(&db).unwrap(), None);

    // Full-entity update by id.
    user.email = "ada@new".into();
    assert_eq!(
        update::<User>().one(&user).affected().execute(&db).unwrap(),
        1
    );
    assert_eq!(
        get::<User>(&1).execute(&db).unwrap().unwrap().email,
        "ada@new"
    );

    // Keyed patch: only `Some` fields change.
    let patch = UserPatch {
        id: 1,
        nickname: Some(Some("ada".into())),
        prefs: None,
    };
    update::<User>()
        .one(&patch)
        .affected()
        .execute(&db)
        .unwrap();
    let got = get::<User>(&1).execute(&db).unwrap().unwrap();
    assert_eq!(
        (got.nickname.as_deref(), got.prefs.theme.as_str()),
        (Some("ada"), "dark")
    );

    // Unkeyed patch: applied to a filter.
    let n = update::<User>()
        .apply(&Rename {
            email: Some("same@x".into()),
        })
        .filter(User::ID.gt(0))
        .affected()
        .execute(&db);
    assert_eq!(n.unwrap(), 2);

    assert_eq!(
        delete::<User>().one(&got).affected().execute(&db).unwrap(),
        1
    );
    assert_eq!(
        delete::<User>().by_id(&2).affected().execute(&db).unwrap(),
        1
    );
    assert!(!select::<User>().exists().execute(&db).unwrap());
}

#[test]
fn update_values_follow_the_patch() {
    let patch = UserPatch {
        id: 3,
        nickname: Some(None),
        prefs: None,
    };
    assert_eq!(
        patch.update_values(),
        [("nickname", Value::Null(SqlType::Text))]
    );
    assert_eq!(Updatable::<User>::key(&patch), rupa::Keyed(3));
    assert_eq!(Rename { email: None }.update_values(), []);
}

#[test]
fn entities_without_capabilities_are_readable() {
    let mut db = MemoryDb::new();
    db.register::<Audit>();
    assert_eq!(select::<Audit>().all().execute(&db).unwrap().len(), 0);
}

#[test]
fn ui() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}

/// A user scalar type: not recognized by name, so its kind is selected at
/// compile time through `ScalarColumn`.
#[derive(Debug, Clone, PartialEq)]
struct Slug(String);

impl rupa::core::ScalarColumn for Slug {
    type Base = Slug;
    const SQL_TYPE: SqlType = SqlType::Text;

    fn to_value(&self) -> Value {
        Value::Text(self.0.clone())
    }

    fn from_value(v: Value) -> Result<Self, rupa::DecodeError> {
        String::from_value(v).map(Slug)
    }
}

/// An alias hides the scalar name; the compile-time selection still applies.
type Label = String;

#[derive(Debug, Entity)]
#[entity(table = "pages")]
struct Page {
    #[id]
    id: i64,
    slug: Slug,
    label: Label,
    prefs: Option<Prefs>,
}

#[test]
fn kinds_resolve_for_named_scalars_user_scalars_aliases_and_json() {
    let _: Column<Page, i64, Scalar> = Page::ID;
    let _: Column<Page, Slug, Scalar> = Page::SLUG;
    let _: Column<Page, Label, Scalar> = Page::LABEL;
    let _: Column<Page, Option<Prefs>, Json> = Page::PREFS;

    assert_eq!(
        Page::SLUG.encode(&Slug("a".into())),
        Value::Text("a".into())
    );
    assert_eq!(Page::PREFS.encode(&None), Value::Null(SqlType::Json));
}
