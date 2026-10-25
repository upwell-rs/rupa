rupa_sql::driver_error! {
    /// Everything that can go wrong running a statement through sqlx.
    pub enum SqlxError {
        /// sqlx (or the database) reported an error.
        Db(sqlx::Error),
        /// A migration connection was used outside a runner-owned migration body.
        #[cfg(feature = "migrations")]
        Migration(rupa_migrate::StateError),
    }
}

impl SqlxError {
    /// The sqlx error, if the database or the connection failed.
    pub fn as_sqlx(&self) -> Option<&sqlx::Error> {
        match self {
            SqlxError::Db(e) => Some(e),
            _ => None,
        }
    }

    /// Whether the connection is unusable after this error (I/O, TLS or
    /// protocol failures), so a pool must not hand it out again.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            SqlxError::Db(
                sqlx::Error::Io(_)
                    | sqlx::Error::Tls(_)
                    | sqlx::Error::Protocol(_)
                    | sqlx::Error::WorkerCrashed
            )
        )
    }
}
