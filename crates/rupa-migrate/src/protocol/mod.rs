//! The engine migration session protocols, shared by every driver of an
//! engine. Not public API: driver crates enable the `protocol` feature.
//!
//! Each engine module (`postgres`, `mysql`, `sqlite`, present with a mode)
//! owns the lock identity and journal table of its engine, so two drivers on
//! one database exclude each other and read the same history by construction. The steps are written once and instantiated per mode as
//! `<engine>::sync` and `<engine>::asynchronous`, over the mode's `Io` trait,
//! which is all a driver implements; `__session!` then forwards its `Session`
//! to the steps. Without a mode, only [`SqlState`] exists.

#[cfg(any(feature = "sync", feature = "async"))]
mod engines;

#[cfg(any(feature = "sync", feature = "async"))]
pub use engines::*;

/// The control IO a migration session needs, without blocking.
#[cfg(feature = "sync")]
pub mod sync {
    use rupa_core::{SqlType, Value};

    /// Framework-owned control statements on one dedicated connection. They
    /// bypass the session's user-statement checks.
    pub trait Io {
        /// The driver's error, which carries refused user statements.
        type Error: std::error::Error
            + super::SqlState
            + From<crate::StateError>
            + Send
            + Sync
            + 'static;

        /// The engine's per-connection migration state.
        type State;

        /// The migration state stored on this connection.
        fn state(&mut self) -> &mut Self::State;

        /// Runs `sql` with `params` bound, decoding each row's columns as `columns`.
        fn query(
            &mut self,
            sql: &str,
            params: &[Value],
            columns: &[SqlType],
        ) -> Result<Vec<Vec<Value>>, Self::Error>;

        /// Runs `sql` with `params` bound, returning the affected-row count.
        fn execute(&mut self, sql: &str, params: &[Value]) -> Result<u64, Self::Error>;

        /// Runs statements without parameters or preparing.
        fn batch(&mut self, sql: &str) -> Result<(), Self::Error>;

        /// Sends a rollback an application transaction dropped while open owes.
        fn clean(&mut self) -> Result<(), Self::Error>;

        /// Forgets prepared statements, whose plans DDL may have invalidated.
        fn forget_statements(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// SQLite's client-side controls, which have no SQL form.
    pub trait SqliteIo: Io {
        /// Whether no transaction is open.
        fn is_autocommit(&mut self) -> Result<bool, Self::Error>;

        /// Installs (`true`) or removes the [`authorize`](crate::protocol::sqlite::authorize)
        /// policy, forgetting prepared statements authorized under the other.
        fn authorizer(&mut self, enabled: bool) -> Result<(), Self::Error>;
    }
}

/// The control IO a migration session needs, awaited.
#[cfg(feature = "async")]
pub mod asynchronous {
    use rupa_core::{SqlType, Value};

    /// Framework-owned control statements on one dedicated connection. They
    /// bypass the session's user-statement checks.
    pub trait Io: Send {
        /// The driver's error, which carries refused user statements.
        type Error: std::error::Error
            + super::SqlState
            + From<crate::StateError>
            + Send
            + Sync
            + 'static;

        /// The engine's per-connection migration state.
        type State: Send;

        /// The migration state stored on this connection.
        fn state(&mut self) -> &mut Self::State;

        /// Runs `sql` with `params` bound, decoding each row's columns as `columns`.
        fn query(
            &mut self,
            sql: &str,
            params: &[Value],
            columns: &[SqlType],
        ) -> impl Future<Output = Result<Vec<Vec<Value>>, Self::Error>> + Send;

        /// Runs `sql` with `params` bound, returning the affected-row count.
        fn execute(
            &mut self,
            sql: &str,
            params: &[Value],
        ) -> impl Future<Output = Result<u64, Self::Error>> + Send;

        /// Runs statements without parameters or preparing.
        fn batch(&mut self, sql: &str) -> impl Future<Output = Result<(), Self::Error>> + Send;

        /// Sends a rollback an application transaction dropped while open owes.
        fn clean(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;

        /// Forgets prepared statements, whose plans DDL may have invalidated.
        fn forget_statements(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
            async { Ok(()) }
        }
    }

    /// SQLite's client-side controls, which have no SQL form.
    pub trait SqliteIo: Io {
        /// Whether no transaction is open.
        fn is_autocommit(&mut self) -> impl Future<Output = Result<bool, Self::Error>> + Send;

        /// Installs (`true`) or removes the [`authorize`](crate::protocol::sqlite::authorize)
        /// policy, forgetting prepared statements authorized under the other.
        fn authorizer(
            &mut self,
            enabled: bool,
        ) -> impl Future<Output = Result<(), Self::Error>> + Send;
    }
}

/// The SQLSTATE of a driver error, for the few failures a protocol classifies.
pub trait SqlState {
    /// The five-character SQLSTATE the server reported, if any.
    fn sqlstate(&self) -> Option<String>;
}

/// Implements a migration `Session` on a driver's executor by forwarding to
/// an engine protocol: `__session!(sync, postgres, PgExecutor)`. The executor
/// implements the mode's [`Io`](sync::Io) with the engine's `State`, and an
/// inherent `execute_migration_statement(&mut self, statement, expect)` for
/// the typed statements of migration bodies.
#[doc(hidden)]
#[macro_export]
macro_rules! __session {
    (sync, $engine:ident, $executor:ty) => {
        $crate::__session!(@emit [] [] sync sync, $engine, $executor);
    };
    (async, $engine:ident, $executor:ty) => {
        $crate::__session!(@emit [async] [.await] asynchronous asynchronous, $engine, $executor);
    };
    (@emit [$($async:tt)*] [$($await:tt)*] $mode:ident $session_mode:ident, $engine:ident, $executor:ty) => {
        impl $crate::$session_mode::Session for $executor {
            const POLICIES: &'static [$crate::Policy] = $crate::protocol::$engine::POLICIES;

            $($async)* fn acquire(
                &mut self,
                stream: &str,
                scheme: $crate::VersionScheme,
            ) -> Result<Vec<i64>, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::acquire(self, stream, scheme) $($await)*
            }

            $($async)* fn history(
                &mut self,
            ) -> Result<Vec<$crate::HistoryEntry>, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::history(self) $($await)*
            }

            $($async)* fn coverage(
                &mut self,
            ) -> Result<Option<$crate::Coverage>, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::coverage(self) $($await)*
            }

            $($async)* fn initialized(
                &mut self,
            ) -> Result<bool, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::initialized(self) $($await)*
            }

            $($async)* fn inspect(
                &mut self,
            ) -> Result<$crate::Inspection, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::inspect(self) $($await)*
            }

            $($async)* fn start_attempt(
                &mut self,
                entry: &$crate::CatalogEntry,
                direction: $crate::Direction,
            ) -> Result<i64, $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::start_attempt(self, entry, direction) $($await)*
            }

            $($async)* fn begin_migration(
                &mut self,
                policy: $crate::Policy,
            ) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::begin_migration(self, policy) $($await)*
            }

            $($async)* fn complete(
                &mut self,
                attempt: i64,
                entry: &$crate::CatalogEntry,
            ) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::complete(self, attempt, entry) $($await)*
            }

            $($async)* fn abort(&mut self, attempt: i64) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::abort(self, attempt) $($await)*
            }

            $($async)* fn release(&mut self) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::release(self) $($await)*
            }

            $($async)* fn script(&mut self, sql: &str) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::script(self, sql) $($await)*
            }

            $($async)* fn user_statement(
                &mut self,
                statement: &$crate::__core::ir::Statement,
                expect: $crate::__core::query::Expect,
            ) -> Result<$crate::__core::exec::Outcome, <Self as $crate::protocol::$mode::Io>::Error> {
                <$executor>::execute_migration_statement(self, statement, expect) $($await)*
            }

            $($async)* fn recover(
                &mut self,
                stream: &str,
                scheme: $crate::VersionScheme,
                entries: &[$crate::CatalogEntry],
                recovery: &$crate::Recovery<'_>,
            ) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::recover(self, stream, scheme, entries, recovery) $($await)*
            }

            $($async)* fn adopt(
                &mut self,
                stream: &str,
                scheme: $crate::VersionScheme,
                adoption: &$crate::Adoption<'_>,
                members: &[$crate::Covered],
            ) -> Result<(), $crate::Error<<Self as $crate::protocol::$mode::Io>::Error>> {
                $crate::protocol::$engine::$mode::adopt(self, stream, scheme, adoption, members) $($await)*
            }

            fn abandon(&mut self) {
                $crate::protocol::$engine::abandon(<Self as $crate::protocol::$mode::Io>::state(self));
            }
        }
    };
}
