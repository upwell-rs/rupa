//! The driver's error type.

rupa_sql::driver_error! {
    /// Everything that can go wrong running a statement on SQLite.
    pub enum SqliteError {
        /// SQLite reported an error.
        Db(rusqlite::Error),
    }
}
