//! Startup: declared databases check or apply their migrations before
//! anything that injects them is built.

mod support;

use std::error::Error;
use std::sync::Arc;

use rupa_example_upwell::{Example, NoteRepository, Notes};
use support::{Scratch, pending, source};

/// `error` and every source, joined.
fn chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut current = error.source();

    while let Some(source) = current {
        text.push_str(": ");
        text.push_str(&source.to_string());
        current = source.source();
    }

    text
}

#[tokio::test]
async fn apply_migrates_before_the_services_that_need_the_database() {
    let scratch = Scratch::new("startup-apply");

    let app = Example::builder()
        .expect("the generated builder constructs")
        .config_source(source(&scratch.toml("apply")))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect("the application builds after applying migrations");
    let notes = app
        .container()
        .resolve::<Arc<Notes>>()
        .await
        .expect("Notes resolves")
        .expect("Notes is a component");
    let repository = app
        .container()
        .resolve::<Arc<dyn NoteRepository>>()
        .await
        .expect("the repository resolves")
        .expect("the database provides the repository");

    assert_eq!(pending(&scratch.database("primary.db")), 0);
    assert!(notes.count().is_ok(), "the notes table exists");
    assert!(
        repository
            .titled("missing")
            .expect("the provided repository queries the pool")
            .is_none()
    );
}

#[tokio::test]
async fn check_stops_startup_while_migrations_are_pending() {
    let scratch = Scratch::new("startup-check");

    let error = Example::builder()
        .expect("the generated builder constructs")
        .config_source(source(&scratch.toml("check")))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect_err("a pending schema stops startup");
    let message = chain(&error);

    assert!(message.contains("pending migration"), "{message}");
    assert_eq!(pending(&scratch.database("primary.db")), 4);
}

#[tokio::test]
async fn check_passes_once_the_schema_is_current() {
    let scratch = Scratch::new("startup-current");

    Example::builder()
        .expect("the generated builder constructs")
        .config_source(source(&scratch.toml("apply")))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect("the first start applies");

    Example::builder()
        .expect("the generated builder constructs")
        .config_source(source(&scratch.toml("check")))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect("a current schema passes the check");
}

#[test]
fn each_database_binding_declares_its_fields_for_tooling() {
    use rupa::upwell::databases::Settings;
    use rupa_example_upwell::{Primary, Reporting};
    use upwell::ConfigProperties;

    assert!(<Settings<Primary> as ConfigProperties>::SCHEMA.is_declared());
    assert!(<Settings<Reporting> as ConfigProperties>::SCHEMA.is_declared());
}
