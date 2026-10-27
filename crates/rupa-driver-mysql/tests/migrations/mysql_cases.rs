use mysql::prelude::Queryable;
use rupa::migrate::sync::{Context, Migration, Session};
use rupa::migrate::{
    ArtifactInput, Catalog, Engine, Error, Metadata, MigrationInfo, MigrationKind, Policy,
    RunError, StateError, Target, VersionScheme,
};
use rupa::raw;
use rupa_conformance::migrations::fixture::with_policy;
use rupa_conformance::migrations::sync::{Harness, Target as _};
use rupa_driver_mysql::{MySqlError, MySqlExecutor};

use super::My;

pub(super) fn run(my: &mut My) {
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            my.reset();
            eprintln!("mysql migrations {}", stringify!($case));
            $case(my);
        )*};
    }

    cases!(
        application_transactions_are_rejected,
        connections_without_a_database_are_rejected,
        named_lock_uses_the_canonical_target_identity,
        sql_mode_is_set_for_the_run_and_restored,
        backslashes_are_literal_in_migration_scripts,
        double_quotes_are_strings_under_ansi_sessions,
        lost_named_lock_poisons_instead_of_completing,
        rust_bodies_run_typed_queries_without_a_transaction,
        ordinary_renames_and_indexes_work,
    );
}

fn numbered<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn rejected_history<T, E: std::error::Error, M: std::error::Error>(
    result: Result<T, RunError<E, M>>,
) -> bool {
    matches!(
        result,
        Err(RunError::Infrastructure {
            source: Error::History(_),
            ..
        })
    )
}

fn lock_name() -> String {
    Target {
        engine: Engine::MySql,
        database: "test",
        namespace: "test",
    }
    .lock_name()
}

fn application_transactions_are_rejected(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE kept (id INTEGER);",
        Policy::NonTransactional,
    )];

    let mut open = my.connect();
    open.connection()
        .query_drop("START TRANSACTION WITH CONSISTENT SNAPSHOT")
        .expect("application transaction");
    let mut plain = my.connect();
    plain
        .connection()
        .query_drop("START TRANSACTION")
        .expect("untouched application transaction");
    let mut manual = my.connect();
    manual
        .connection()
        .query_drop("SET autocommit = 0")
        .expect("manual commit mode");

    assert!(rejected_history(open.apply(&numbered(&entries))));
    assert!(rejected_history(plain.apply(&numbered(&entries))));
    assert!(rejected_history(manual.apply(&numbered(&entries))));
    assert!(!my.table_exists("_rupa_migrations"));
}

fn connections_without_a_database_are_rejected(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE kept (id INTEGER);",
        Policy::NonTransactional,
    )];
    let opts = mysql::OptsBuilder::from_opts(my.opts.clone()).db_name(None::<String>);

    let mut session = MySqlExecutor::connect(opts).expect("connect without a database");

    assert!(rejected_history(session.apply(&numbered(&entries))));
}

fn named_lock_uses_the_canonical_target_identity(my: &mut My) {
    let held = format!("SELECT IS_USED_LOCK('{}') IS NOT NULL", lock_name());

    let mut owner = my.connect();
    owner
        .acquire("primary", VersionScheme::Numbered)
        .expect("acquire");

    assert_eq!(my.count(&held), 1, "any driver can derive the held lock");

    owner.release().expect("release");

    assert_eq!(my.count(&held), 0);
}

fn sql_mode_is_set_for_the_run_and_restored(my: &mut My) {
    let mode = |session: &mut MySqlExecutor| -> String {
        session
            .connection()
            .query_first("SELECT @@SESSION.sql_mode")
            .expect("sql_mode")
            .expect("one row")
    };

    let mut session = my.connect();
    let original = mode(&mut session);
    session
        .acquire("primary", VersionScheme::Numbered)
        .expect("acquire");
    let during = mode(&mut session);
    session.release().expect("release");

    assert!(!original.contains("NO_BACKSLASH_ESCAPES"));
    assert!(during.contains("NO_BACKSLASH_ESCAPES"), "{during}");
    assert_eq!(mode(&mut session), original);
}

fn backslashes_are_literal_in_migration_scripts(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE paths (p VARCHAR(16)); INSERT INTO paths VALUES ('C:\\'), ('a;b');",
        Policy::NonTransactional,
    )];

    my.connect()
        .apply(&numbered(&entries))
        .expect("a backslash before a quote does not escape it");

    assert_eq!(
        my.count("SELECT COUNT(*) FROM paths WHERE p IN (CONCAT('C:', CHAR(92)), 'a;b')"),
        2
    );
}

fn double_quotes_are_strings_under_ansi_sessions(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE quoted (q VARCHAR(32)); INSERT INTO quoted VALUES (\"_rupa_migrations\");",
        Policy::NonTransactional,
    )];

    let mut session = my.connect();
    session
        .connection()
        .query_drop("SET SESSION sql_mode = 'ANSI'")
        .expect("ANSI session");
    let original: String = session
        .connection()
        .query_first("SELECT @@SESSION.sql_mode")
        .expect("sql_mode")
        .expect("one row");

    session
        .apply(&numbered(&entries))
        .expect("a double-quoted name is a string, not a reserved table");
    let restored: String = session
        .connection()
        .query_first("SELECT @@SESSION.sql_mode")
        .expect("sql_mode")
        .expect("one row");

    assert!(original.contains("ANSI_QUOTES"), "{original}");
    assert_eq!(restored, original);
    assert_eq!(
        my.count("SELECT COUNT(*) FROM quoted WHERE q = '_rupa_migrations'"),
        1
    );
    assert_eq!(
        my.count("SELECT COUNT(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'applied'"),
        1
    );
}

fn lost_named_lock_poisons_instead_of_completing(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE marker(id INTEGER); PREPARE escape FROM 'DO RELEASE_ALL_LOCKS()'; EXECUTE escape;",
        Policy::NonTransactional,
    )];

    let mut session = my.connect();

    assert!(
        session.apply(&numbered(&entries)).is_err(),
        "string-literal dynamic SQL passes the grammar, the lock check refuses completion"
    );
    assert!(matches!(
        raw().sql("SELECT 1").execute::<u64>().execute(&mut session),
        Err(MySqlError::Migration(StateError::Poisoned))
    ));
    assert!(my.table_exists("marker"), "autocommit effects persist");
    assert_eq!(
        my.count("SELECT COUNT(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'applied'"),
        0
    );
    assert_eq!(
        my.count("SELECT COUNT(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'running'"),
        1
    );
}

/// Seeds notes with bound values from Rust between statements.
struct SeedNotes;

impl MigrationInfo for SeedNotes {
    fn info(&self) -> &'static Metadata {
        const INFO: Metadata = Metadata {
            version: "1",
            description: "seed notes from Rust",
            policy: Policy::NonTransactional,
            kind: MigrationKind::Incremental,
            inputs: &[ArtifactInput::new(
                "mysql_cases.rs",
                include_str!("mysql_cases.rs"),
            )],
        };

        &INFO
    }
}

impl<S: Session> Migration<S> for SeedNotes {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("CREATE TABLE notes (title VARCHAR(64) NOT NULL)")?;

        for title in ["it's", "semi;colon"] {
            raw()
                .sql("INSERT INTO notes (title) VALUES (")
                .bind(title.to_uppercase())
                .sql(")")
                .execute::<u64>()
                .execute(context)?;
        }

        Ok(())
    }
}

fn rust_bodies_run_typed_queries_without_a_transaction(my: &mut My) {
    let entries = [SeedNotes];

    let report = my
        .connect()
        .apply(&numbered(&entries))
        .expect("Rust body applies");

    assert_eq!(report.applied.len(), 1);
    assert_eq!(
        my.count("SELECT COUNT(*) FROM notes WHERE title IN ('IT''S', 'SEMI;COLON')"),
        2
    );
}

fn ordinary_renames_and_indexes_work(my: &mut My) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE notes (old_name VARCHAR(64)); ALTER TABLE notes RENAME COLUMN old_name TO title; RENAME TABLE notes TO renamed; INSERT INTO renamed VALUES ('ok'); CREATE UNIQUE INDEX renamed_title ON renamed (title); DROP INDEX renamed_title ON renamed;",
        Policy::NonTransactional,
    )];

    my.connect()
        .apply(&numbered(&entries))
        .expect("ordinary DDL must remain supported");

    assert_eq!(
        my.count("SELECT COUNT(*) FROM renamed WHERE title = 'ok'"),
        1
    );
}
