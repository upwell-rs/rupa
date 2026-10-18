//! The error enum every SQL driver declares, written once.

/// Declares a SQL driver's error type: the database's own error (`Db`) next
/// to the variants every driver shares (rendering, result shaping,
/// transaction misuse, pool checkout), with `thiserror` impls, `From`
/// conversions, [`ExecError`](rupa_core::exec::ExecError) and a conversion
/// into [`DynError`](rupa_core::DynError).
///
/// The calling crate must depend on `thiserror`.
///
/// ```ignore
/// rupa_sql::driver_error! {
///     /// Everything that can go wrong running a statement on SQLite.
///     pub enum SqliteError {
///         /// SQLite reported an error.
///         Db(rusqlite::Error),
///     }
/// }
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! driver_error {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(#[$db_meta:meta])*
            Db($db:ty) $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, ::thiserror::Error)]
        #[non_exhaustive]
        $vis enum $name {
            /// The statement could not be rendered in the driver's dialect.
            #[error("rendering failed: {0}")]
            Render(#[from] $crate::RenderError),
            $(#[$db_meta])*
            #[error("{0}")]
            Db(#[from] $db),
            /// The rows did not fit the query's result type.
            #[error("{0}")]
            Result(#[from] $crate::__private::rupa_core::ResultError),
            /// The transaction API was misused.
            #[error("{0}")]
            Tx(#[from] $crate::__private::rupa_core::TxError),
            /// A value the driver cannot bind, e.g. a type whose cargo feature
            /// (`chrono`, `uuid`) the driver was built without.
            #[error("cannot bind {0}")]
            Bind(::std::string::String),
            /// No pooled connection could be obtained (e.g. a timeout).
            #[error("connection pool: {0}")]
            Pool(#[source] ::std::boxed::Box<dyn ::std::error::Error + ::std::marker::Send + ::std::marker::Sync>),
        }

        impl $crate::__private::rupa_core::exec::ExecError for $name {
            fn result_error(&self) -> ::std::option::Option<&$crate::__private::rupa_core::ResultError> {
                match self {
                    Self::Result(e) => ::std::option::Option::Some(e),
                    _ => ::std::option::Option::None,
                }
            }
        }

        impl ::std::convert::From<$name> for $crate::__private::rupa_core::DynError {
            fn from(e: $name) -> Self {
                Self::new(e)
            }
        }
    };
}
