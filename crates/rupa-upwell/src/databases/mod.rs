//! Databases in an Upwell application, declared with
//! [`database!`](crate::database!).
//!
//! A declaration names a driver pool, the configuration path of its
//! [`DatabaseSettings`], its migration catalog and an optional migration
//! command. Like `#[component]`, it registers itself at link time, so the
//! application discovers:
//!
//! - a typed configuration binding: separate connection fields, an optional
//!   `url` override templated by Upwell, the pool, driver options and the
//!   migration policy with its own credentials, with a declared schema for
//!   Upwell's tooling;
//! - a singleton [`Database`] component whose factory checks or applies the
//!   migrations before it opens the pool, so everything that injects it is
//!   built after the schema is ready, and nothing starts when it is not. It
//!   is a [`Connection`](rupa_core::connection::Connection) like its pool:
//!   `NoteRepositoryRepo::new(&*db)` or `db.begin()` work on it directly;
//! - every repository declared `#[repository(database = Primary)]`, as
//!   `Arc<dyn Repository>` over the component itself.
//!
//! The migration commands come from one plugin, [`Migrations`], registered
//! once for every declared database: Upwell discovers components, providers
//! and configuration at link time, but not plugins.

mod cli;
mod component;
mod connection;
mod declaration;
mod error;
mod mode;
mod registry;
mod settings;

pub use cli::Command;
pub use component::Database;
pub use declaration::Declaration;
pub use error::{CommandError, DatabaseError, exit_code};
pub use mode::Mode;
pub use registry::{
    COMMANDS, CommandRegistration, Migrations, command, component, provider, settings,
};
pub use settings::{DatabaseSettings, MigrationSettings, Settings, SettingsError, Startup};

/// Declares a database of an Upwell application. It registers itself:
/// nothing is added to the application's `plugins` for it.
///
/// ```ignore
/// rupa::upwell::database! {
///     /// Notes and tags.
///     pub Primary {
///         pool: rupa::drivers::sqlite::SqlitePool,
///         config: "database.primary",
///         migrations: db::migrations,
///         command: "migrate",
///     }
/// }
///
/// #[repository(database = Primary)]
/// pub trait NoteRepository: Send + Sync { .. }
/// ```
///
/// `migrations` is the module `include_migrations!` generated; its
/// `Migrations` must run on the pool's mode. `command` is optional. A command
/// name uses lowercase letters, digits, `.`, `_` and `-`; its runner is the
/// once-registered [`Migrations`] plugin. Inject `Arc<Database<Primary>>`, or
/// the `Arc<dyn Repository>` of a repository declared with
/// `#[repository(database = Primary)]`, which runs on that database.
#[macro_export]
macro_rules! database {
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident {
            pool: $pool:ty,
            config: $config:literal,
            migrations: $($migrations:ident)::+,
            $(command: $command:literal $(,)?)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy)]
        $vis struct $name;

        impl $crate::databases::Declaration for $name {
            type Pool = $pool;
            type Migration = $($migrations)::+::Migrations;
            type Mode = <$pool as $crate::__private::Connect>::Completion;

            const NAME: &'static str = ::core::stringify!($name);
            const ID: &'static str = ::core::concat!("rupa:database:", $config);
            const CONFIG: &'static str = $config;

            fn catalog() -> ::core::result::Result<
                $crate::__private::Catalog<'static, Self::Migration>,
                $crate::__private::ValidationError,
            > {
                $($migrations)::+::catalog()
            }
        }

        const _: () = {
            #[$crate::__private::linkme::distributed_slice($crate::__private::CONFIG_BINDINGS)]
            #[linkme(crate = $crate::__private::linkme)]
            #[allow(unsafe_code)]
            static SETTINGS: $crate::__private::ConfigBindingDescriptor =
                $crate::databases::settings::<$name>();

            #[$crate::__private::linkme::distributed_slice($crate::__private::COMPONENTS)]
            #[linkme(crate = $crate::__private::linkme)]
            #[allow(unsafe_code)]
            static COMPONENT: $crate::__private::ComponentDescriptor =
                $crate::databases::component::<$name>();
        };

        $(
            const _: () = {
                #[$crate::__private::linkme::distributed_slice($crate::databases::COMMANDS)]
                #[linkme(crate = $crate::__private::linkme)]
                #[allow(unsafe_code)]
                static COMMAND: $crate::databases::CommandRegistration =
                    $crate::databases::CommandRegistration {
                        name: $command,
                        id: match $crate::__private::ContributionId::new(::core::concat!(
                            "rupa/command.", $command
                        )) {
                            ::core::result::Result::Ok(id) => id,
                            ::core::result::Result::Err(_) => ::core::panic!(::core::concat!(
                                "`", $command, "` is not a valid migration command name"
                            )),
                        },
                        register: $crate::databases::command::<$name>,
                    };
            };
        )?
    };
}
