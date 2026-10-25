//! A native and a sqlx runner on one target: each excludes the other while
//! it owns the target, and both read the history either one wrote.

use rupa::migrate::{Catalog, Error, HistoryEntry, Policy, VersionScheme, asynchronous, sync};
use rupa_conformance::migrations::fixture::{Body, with_policy};
use tokio::runtime::Runtime;

/// Runs the interop checks with fresh native sessions from `native` and
/// fresh sqlx sessions from `sqlx`, applying through sqlx under `policy`.
/// Each step runs on its own: sync drivers block, so they never run inside
/// the runtime.
pub(crate) fn run<N, A>(
    native: impl Fn() -> N,
    sqlx: impl Fn() -> A,
    runtime: &Runtime,
    policy: Policy,
) where
    N: sync::Session,
    A: asynchronous::Session,
    Body: sync::Migration<N> + asynchronous::Migration<A>,
{
    let entries = [with_policy(
        "1",
        "CREATE TABLE interop (id INTEGER);",
        policy,
    )];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");
    let mut owner = native();
    let mut competitor = sqlx();

    owner
        .acquire("primary", VersionScheme::Numbered)
        .expect("native runner acquires");

    assert!(
        matches!(
            runtime.block_on(competitor.acquire("primary", VersionScheme::Numbered)),
            Err(Error::Busy)
        ),
        "sqlx must not acquire a target the native runner owns"
    );

    owner.release().expect("native runner releases");

    let applier = sqlx();
    let report = runtime
        .block_on(asynchronous::Apply::apply(&catalog, applier))
        .expect("sqlx applies");
    let mut reader = sqlx();
    let sqlx_history = runtime.block_on(reader.history()).expect("sqlx history");
    let native_history = native().history().expect("native history");

    assert_eq!(report.applied.len(), 1);
    assert_eq!(summary(&native_history), summary(&sqlx_history));
    assert_eq!(summary(&native_history)[0].0, 1);

    let mut owner = sqlx();

    runtime
        .block_on(owner.acquire("primary", VersionScheme::Numbered))
        .expect("sqlx runner acquires");

    assert!(
        matches!(
            native().acquire("primary", VersionScheme::Numbered),
            Err(Error::Busy)
        ),
        "the native runner must not acquire a target sqlx owns"
    );

    runtime
        .block_on(owner.release())
        .expect("sqlx runner releases");

    let report = sync::Apply::apply(&catalog, native()).expect("native applies");

    assert!(
        report.applied.is_empty(),
        "sqlx's history is the native runner's"
    );
}

fn summary(history: &[HistoryEntry]) -> Vec<(u64, String, Vec<u8>)> {
    history
        .iter()
        .map(|entry| {
            (
                entry.installation_rank,
                entry.version.clone(),
                entry.checksum.as_bytes().to_vec(),
            )
        })
        .collect()
}
