/// Embeds a SQL file in a zero-sized planned migration. Paths are relative to
/// the invoking source file, as with `include_str!`; explicit catalogs track
/// changes to each referenced file through normal Rust dependency tracking.
/// `policy` names a [`Policy`](crate::Policy) variant and defaults to
/// `TransactionalRequired`. `kind` names a [`MigrationKind`](crate::MigrationKind)
/// variant and defaults to `Incremental`; a `Snapshot` covers every
/// incremental at or below its version and has no down body. An optional
/// `down` file is the authored down body; both files are artifact inputs,
/// so either one changes the checksum.
#[macro_export]
macro_rules! sql_migration {
    ($(#[$meta:meta])* $vis:vis struct $name:ident {
        version: $version:literal,
        description: $description:literal,
        $(policy: $policy:ident,)?
        $(kind: $kind:ident,)?
        up: $path:literal
        $(, down: $down:literal)? $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy)]
        $vis struct $name;

        impl $crate::MigrationInfo for $name {
            fn info(&self) -> &'static $crate::Metadata {
                const INFO: $crate::Metadata = $crate::Metadata {
                    version: $version,
                    description: $description,
                    policy: $crate::__policy!($($policy)?),
                    kind: $crate::__kind!($($kind)?),
                    inputs: &[
                        $crate::ArtifactInput::new($path, ::std::include_str!($path)),
                        $($crate::ArtifactInput::new($down, ::std::include_str!($down)),)?
                    ],
                };

                &INFO
            }
        }

        impl $crate::PlannedMigration for $name {
            fn plan(&self) -> $crate::Plan {
                $crate::Plan::new().sql(::std::include_str!($path))
            }

            $(
                fn down(&self) -> ::std::option::Option<$crate::Plan> {
                    ::std::option::Option::Some($crate::Plan::new().sql(::std::include_str!($down)))
                }
            )?
        }
    };
}

/// The declared policy of a SQL migration, `TransactionalRequired` when omitted.
#[doc(hidden)]
#[macro_export]
macro_rules! __policy {
    () => {
        $crate::Policy::TransactionalRequired
    };
    ($policy:ident) => {
        $crate::Policy::$policy
    };
}

/// The declared kind of a SQL migration, `Incremental` when omitted.
#[doc(hidden)]
#[macro_export]
macro_rules! __kind {
    () => {
        $crate::MigrationKind::Incremental
    };
    ($kind:ident) => {
        $crate::MigrationKind::$kind
    };
}

/// Generates a closed mixed catalog and typed error enum, with static dispatch.
/// Each variant contains an imperative migration or an explicit `Planned<T>`.
#[macro_export]
macro_rules! migration_catalog {
    ($(#[$meta:meta])* $vis:vis enum $name:ident, error $error:ident {
        $($variant:ident($ty:ty)),+ $(,)?
    }) => {
        $(#[$meta])*
        $vis enum $name { $( #[doc = "A concrete catalog entry."] $variant($ty), )+ }

        #[doc = "Typed errors for the generated migration catalog."]
        #[derive(Debug)]
        $vis enum $error<$($variant),+> { $( #[doc = "The corresponding migration failed."] $variant($variant), )+ }

        impl<$($variant: ::std::error::Error),+> ::std::fmt::Display for $error<$($variant),+> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match self { $( Self::$variant(source) => ::std::fmt::Display::fmt(source, f), )+ }
            }
        }

        impl<$($variant: ::std::error::Error + 'static),+> ::std::error::Error for $error<$($variant),+> {
            fn source(&self) -> Option<&(dyn ::std::error::Error + 'static)> {
                match self { $( Self::$variant(source) => Some(source), )+ }
            }
        }

        impl $crate::MigrationInfo for $name {
            fn info(&self) -> &'static $crate::Metadata {
                match self { $( Self::$variant(value) => $crate::MigrationInfo::info(value), )+ }
            }

            fn planned(&self) -> Option<$crate::Plan> {
                match self { $( Self::$variant(value) => $crate::MigrationInfo::planned(value), )+ }
            }

            fn reversal(&self) -> $crate::Reversal {
                match self { $( Self::$variant(value) => $crate::MigrationInfo::reversal(value), )+ }
            }
        }

        $crate::__sync_catalog!($name, $error, $($variant($ty)),+);
        $crate::__async_catalog!($name, $error, $($variant($ty)),+);
    };
}

/// The sync execution of a generated catalog, emitted only when this crate
/// has `sync`, so consumers never evaluate a feature of their own.
#[cfg(feature = "sync")]
#[doc(hidden)]
#[macro_export]
macro_rules! __sync_catalog {
    ($name:ident, $error:ident, $($variant:ident($ty:ty)),+) => {
        impl<__Session: $crate::sync::Session> $crate::sync::Migration<__Session> for $name
        where $( $ty: $crate::sync::Migration<__Session>, )+
        {
            type Error = $error<$(<$ty as $crate::sync::Migration<__Session>>::Error),+>;

            fn up(&self, context: &mut $crate::sync::Context<'_, __Session>) -> Result<(), Self::Error> {
                match self { $( Self::$variant(value) => $crate::sync::Migration::up(value, context).map_err($error::$variant), )+ }
            }

            fn down(&self, context: &mut $crate::sync::Context<'_, __Session>) -> Result<(), Self::Error> {
                match self { $( Self::$variant(value) => $crate::sync::Migration::down(value, context).map_err($error::$variant), )+ }
            }
        }
    };
}

#[cfg(not(feature = "sync"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __sync_catalog {
    ($($tokens:tt)*) => {};
}

/// The async execution of a generated catalog, emitted only when this crate
/// has `async`.
#[cfg(feature = "async")]
#[doc(hidden)]
#[macro_export]
macro_rules! __async_catalog {
    ($name:ident, $error:ident, $($variant:ident($ty:ty)),+) => {
        impl<__Session: $crate::asynchronous::Session> $crate::asynchronous::Migration<__Session> for $name
        where Self: Send + Sync, $( $ty: $crate::asynchronous::Migration<__Session>, )+
        {
            type Error = $error<$(<$ty as $crate::asynchronous::Migration<__Session>>::Error),+>;

            async fn up(&self, context: &mut $crate::asynchronous::Context<'_, __Session>) -> Result<(), Self::Error> {
                match self { $( Self::$variant(value) => $crate::asynchronous::Migration::up(value, context).await.map_err($error::$variant), )+ }
            }

            async fn down(&self, context: &mut $crate::asynchronous::Context<'_, __Session>) -> Result<(), Self::Error> {
                match self { $( Self::$variant(value) => $crate::asynchronous::Migration::down(value, context).await.map_err($error::$variant), )+ }
            }
        }
    };
}

#[cfg(not(feature = "async"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __async_catalog {
    ($($tokens:tt)*) => {};
}

/// Includes the catalog `rupa_build::Migrations::new(stream)` generated in this
/// crate's build script: `STREAM`, `SCHEME`, the `Migrations` enum, its typed
/// `MigrationsError`, `ENTRIES` and `catalog()`.
///
/// The catalog is engine-independent, so it can live in a `db` crate while an
/// application crate applies it on its own dedicated connection.
#[macro_export]
macro_rules! include_migrations {
    ($stream:literal) => {
        #[doc(hidden)]
        use $crate as __rupa_migrate;

        ::std::include!(::std::concat!(
            ::std::env!("OUT_DIR"),
            "/rupa-migrations/",
            $stream,
            ".rs"
        ));
    };
}
