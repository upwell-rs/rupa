//! With exactly one mode enabled, the crate root and prelude expose it under
//! the shared names; these functions compile only if the names resolve to the
//! mode's own traits. With both, each mode is reached through its module.

#![cfg(any(feature = "sync", feature = "async"))]
#![allow(dead_code)]

use rupa_migrate::Catalog;

#[cfg(all(feature = "sync", not(feature = "async")))]
fn root_names_are_the_sync_traits<S, M>(catalog: &Catalog<'_, M>, connection: S)
where
    S: rupa_migrate::sync::Session,
    M: rupa_migrate::sync::Migration<S>,
{
    use rupa_migrate::prelude::*;

    fn root<S: rupa_migrate::Session, M: rupa_migrate::Migration<S>>() {}

    root::<S, M>();

    let _ = catalog.apply(connection);
}

#[cfg(all(feature = "async", not(feature = "sync")))]
fn root_names_are_the_async_traits<S, M>(catalog: &Catalog<'_, M>, connection: S)
where
    S: rupa_migrate::asynchronous::Session,
    M: rupa_migrate::asynchronous::Migration<S> + Send + Sync,
{
    use rupa_migrate::prelude::*;

    fn root<S: rupa_migrate::Session, M: rupa_migrate::Migration<S>>() {}

    root::<S, M>();

    drop(catalog.apply(connection));
}

#[cfg(all(feature = "sync", feature = "async"))]
fn both_modes_are_reached_through_their_modules<S, A, M>(
    catalog: &Catalog<'_, M>,
    sync: S,
    asynchronous: A,
) where
    S: rupa_migrate::sync::Session,
    A: rupa_migrate::asynchronous::Session,
    M: rupa_migrate::sync::Migration<S> + rupa_migrate::asynchronous::Migration<A> + Send + Sync,
{
    let _ = rupa_migrate::sync::Apply::apply(catalog, sync);

    drop(rupa_migrate::asynchronous::Apply::apply(
        catalog,
        asynchronous,
    ));
}
