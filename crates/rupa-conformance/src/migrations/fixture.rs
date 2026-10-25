//! Migration bodies shared by the cases and by driver-specific migration tests.

use rupa::migrate::schema::SchemaOp;
use rupa::migrate::schema::{ColumnDef, ColumnDefault, ColumnType, Index, Table};
use rupa::migrate::{
    ArtifactInput, Error, Metadata, MigrationInfo, MigrationKind, Plan, Planned, PlannedMigration,
    Policy,
};
use rupa::{raw, select};

/// Declares a fixture's metadata with its own source as the artifact closure.
macro_rules! fixture_info {
    ($version:literal, $description:literal) => {
        fixture_info!($version, $description, TransactionalRequired);
    };
    ($version:literal, $description:literal, $policy:ident) => {
        fn info(&self) -> &'static Metadata {
            const INFO: Metadata = Metadata {
                version: $version,
                description: $description,
                policy: Policy::$policy,
                kind: MigrationKind::Incremental,
                inputs: &[ArtifactInput::new("fixture.rs", include_str!("fixture.rs"))],
            };

            &INFO
        }
    };
}

/// A declared script fixture using the same immutable closure as its body.
#[derive(Debug, Clone, Copy)]
pub struct Body(pub &'static Metadata);

impl MigrationInfo for Body {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

/// A script fixture at `version`.
pub fn body(version: &'static str, sql: &'static str) -> Body {
    with_policy(version, sql, Policy::TransactionalRequired)
}

/// A script fixture with an explicit execution policy.
pub fn with_policy(version: &'static str, sql: &'static str, policy: Policy) -> Body {
    Body(Box::leak(Box::new(Metadata {
        version,
        description: "script fixture",
        policy,
        kind: MigrationKind::Incremental,
        inputs: Box::leak(vec![ArtifactInput::new("body.sql", sql)].into_boxed_slice()),
    })))
}

/// A planned fixture whose plan comes from a function; wrap it in `Planned`.
pub struct Plans(pub &'static Metadata, pub fn() -> Plan);

impl MigrationInfo for Plans {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl PlannedMigration for Plans {
    fn plan(&self) -> Plan {
        (self.1)()
    }
}

/// Metadata of a planned Rust migration: no inputs, its plan is its identity.
pub fn planned(version: &'static str) -> &'static Metadata {
    planned_with(version, Policy::TransactionalRequired)
}

/// Planned metadata with an explicit execution policy.
pub fn planned_with(version: &'static str, policy: Policy) -> &'static Metadata {
    Box::leak(Box::new(Metadata {
        version,
        description: "planned fixture",
        policy,
        kind: MigrationKind::Incremental,
        inputs: &[],
    }))
}

rupa::migrate::sql_migration! {
    /// Creates notes using an embedded SQL source.
    pub struct CreateNotes {
        version: "1",
        description: "create notes",
        up: "create_notes.sql",
    }
}

/// The typed result read by [`Normalize`].
#[derive(rupa::Entity)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

/// Compiled Rust migration that transforms typed query results between SQL steps.
#[derive(Debug)]
pub struct Normalize;

impl MigrationInfo for Normalize {
    fixture_info!("2", "normalize note titles in Rust");
}

rupa::migrate::migration_catalog! {
    /// A statically dispatched SQL/Rust catalog.
    pub enum Mixed, error MixedError {
        Schema(Planned<CreateNotes>),
        Rust(Normalize),
    }
}

/// A migration with an application-specific error distinct from the driver's error.
pub struct ApplicationFailure;

impl MigrationInfo for ApplicationFailure {
    fixture_info!("2", "application error");
}

rupa::migrate::migration_catalog! {
    /// Heterogeneous static errors do not require boxing the migration body.
    pub enum Failing, error FailingError {
        Sql(Planned<CreateNotes>),
        Custom(ApplicationFailure),
    }
}

/// Uses the typed-query surface to attempt transaction control.
pub struct RawCommit;

impl MigrationInfo for RawCommit {
    fixture_info!("1", "raw control");
}

/// Panics after transactional DDL to exercise rollback on session destruction.
pub struct Interrupted;

impl MigrationInfo for Interrupted {
    fixture_info!("1", "interruption");
}

/// Panics after a nontransactional statement, whose effect then survives.
pub struct InterruptedAutocommit;

impl MigrationInfo for InterruptedAutocommit {
    fixture_info!("1", "nontransactional interruption", NonTransactional);
}

/// The author shape the schema migrations below create.
#[derive(Debug, rupa::Entity)]
#[entity(table = "authors")]
pub struct Author {
    #[id(generated)]
    pub id: i64,
    pub name: String,
}

/// A post referencing its author.
#[derive(Debug, rupa::Entity)]
#[entity(table = "posts")]
pub struct Post {
    #[id(generated)]
    pub id: i64,
    #[column(references = Author)]
    pub author_id: i64,
    pub title: String,
    pub body: Option<String>,
}

/// Creates the author and post tables with an author index. The indexed
/// column is bounded, so MySQL can key it.
pub fn create_tables() -> Plan {
    Plan::new()
        .create_table(Table::of::<Author>())
        .create_table(Table::of::<Post>())
        .create_index(Index::on("posts_author", Post::AUTHOR_ID))
}

/// Adds a defaulted column to posts, then inserts a post.
pub fn add_pinned() -> Plan {
    Plan::new()
        .add_column_to(
            <Post as rupa::Entity>::TABLE,
            ColumnDef::new("pinned", ColumnType::Bool)
                .default(ColumnDefault::Value(rupa::core::Value::Bool(false))),
        )
        .sql("INSERT INTO authors (name) VALUES ('ada'); INSERT INTO posts (author_id, title) VALUES (1, 'hello');")
}

/// A scripted migration mixing schema operations with arbitrary steps.
pub struct Scripted(pub &'static Metadata);

impl MigrationInfo for Scripted {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

/// Emits every fixture body for one mode: `.await` follows each context call
/// in async mode only, so both modes run identical bodies.
macro_rules! bodies {
    ([$($async:tt)*] [$($await:tt)*]) => {
        impl<S: Session> Migration<S> for Body {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.script(self.0.inputs[0].contents) $($await)*
            }
        }

        impl<S: Session> Migration<S> for Normalize {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                let notes = context.run(select::<Note>().all()) $($await)*?;

                for note in notes {
                    let title = note.title.trim().to_uppercase();

                    context.run(
                        raw()
                            .sql("UPDATE notes SET title = ")
                            .bind(title)
                            .sql(" WHERE id = ")
                            .bind(note.id)
                            .execute::<u64>(),
                    ) $($await)*?;
                }

                Ok(())
            }
        }

        impl<S: Session> Migration<S> for ApplicationFailure {
            type Error = std::io::Error;

            $($async)* fn up(&self, _: &mut Context<'_, S>) -> Result<(), Self::Error> {
                Err(std::io::Error::other("application rejected migration"))
            }
        }

        impl<S: Session> Migration<S> for RawCommit {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.script("CREATE TABLE marker(id INTEGER)") $($await)*?;
                context.run(raw().sql("COMMIT").execute::<u64>()) $($await)*?;

                Ok(())
            }
        }

        impl<S: Session> Migration<S> for Interrupted {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.script("CREATE TABLE lost(id INTEGER)") $($await)*?;

                panic!("injected migration interruption");
            }
        }

        impl<S: Session> Migration<S> for InterruptedAutocommit {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.script("CREATE TABLE survived(id INTEGER)") $($await)*?;

                panic!("injected nontransactional interruption");
            }
        }

        impl<S: Session> Migration<S> for Scripted {
            type Error = Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.schema(&SchemaOp::CreateTable(Table::of::<Author>())) $($await)*?;
                context.script("INSERT INTO authors (name) VALUES ('grace')") $($await)*
            }
        }
    };
}

/// The fixtures' sync bodies.
mod sync_bodies {
    use rupa::migrate::sync::{Context, Migration, Session};

    use super::*;

    bodies!([] []);
}

/// The fixtures' async bodies.
mod async_bodies {
    use rupa::migrate::asynchronous::{Context, Migration, Session};

    use super::*;

    bodies!([async] [.await]);
}
