//! The registry entry, for run-time driver selection.

use crate::MySqlPool;

rupa_core::blocking_driver! {
    /// The registry entry for this driver: name `mysql`, scheme `mysql`.
    ///
    /// Builds a [`MySqlPool`], served to async code through
    /// [`Blocking`](rupa_core::exec::Blocking).
    pub struct MySqlDriver {
        name: "mysql",
        dialect: MySql,
        schemes: ["mysql"],
        pool: MySqlPool,
    }
}
