//! The registry entry, for run-time driver selection.

use crate::PgPool;

rupa_core::blocking_driver! {
    /// The registry entry for this driver: name `postgres-sync`, schemes
    /// `postgres` and `postgresql`.
    ///
    /// Builds a [`PgPool`] (without TLS unless `tls-rustls` is on), served to
    /// async code through [`Blocking`](rupa_core::exec::Blocking).
    pub struct PgDriver {
        name: "postgres-sync",
        dialect: Postgres,
        schemes: ["postgres", "postgresql"],
        pool: PgPool,
    }
}
