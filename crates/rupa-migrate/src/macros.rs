/// Embeds a SQL file in a zero-sized planned migration. Paths are relative to
/// the invoking source file, as with `include_str!`; explicit catalogs track
/// changes to each referenced file through normal Rust dependency tracking.
#[macro_export]
macro_rules! sql_migration {
    ($(#[$meta:meta])* $vis:vis struct $name:ident {
        version: $version:literal,
        description: $description:literal,
        up: $path:literal $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy)]
        $vis struct $name;

        impl $crate::MigrationInfo for $name {
            fn info(&self) -> &'static $crate::Metadata {
                const INFO: $crate::Metadata = $crate::Metadata {
                    version: $version,
                    description: $description,
                    policy: $crate::Policy::TransactionalRequired,
                    kind: $crate::MigrationKind::Incremental,
                    inputs: &[$crate::ArtifactInput::new($path, ::std::include_str!($path))],
                };

                &INFO
            }
        }

        impl $crate::PlannedMigration for $name {
            const UP: &'static [$crate::Operation] = &[$crate::Operation::Sql(::std::include_str!($path))];
        }
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

            fn planned_operations(&self) -> Option<&'static [$crate::Operation]> {
                match self { $( Self::$variant(value) => $crate::MigrationInfo::planned_operations(value), )+ }
            }
        }

        impl<__Session: $crate::Session> $crate::Migration<__Session> for $name
        where $( $ty: $crate::Migration<__Session>, )+
        {
            type Error = $error<$(<$ty as $crate::Migration<__Session>>::Error),+>;

            fn up(&self, context: &mut $crate::Context<'_, __Session>) -> Result<(), Self::Error> {
                match self { $( Self::$variant(value) => $crate::Migration::up(value, context).map_err($error::$variant), )+ }
            }
        }
    };
}
