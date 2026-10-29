//! The driver's error type.

rupa_sql::driver_error! {
    /// Everything that can go wrong running a statement on MySQL.
    pub enum MySqlError {
        /// MySQL reported an error.
        Db(mysql::Error),
        /// A migration connection was used outside a runner-owned attempt body.
        #[cfg(feature = "migrations")]
        Migration(rupa_migrate::StateError),
    }
}
