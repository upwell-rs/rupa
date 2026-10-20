//! Migration control stays inside the native Postgres executor. The protocol
//! is shared with the async driver and instantiated here in sync mode.

use crate::PgExecutor;

rupa_driver_tokio_postgres::__pg_migrations!(sync);
