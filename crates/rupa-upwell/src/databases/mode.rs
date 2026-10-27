use std::future::Future;

use rupa_core::config::DbConfig;
use rupa_core::error::ConnectError;
use rupa_core::pool::Connect;
use rupa_migrate::Catalog;
use rupa_migrate::commands::cli::Migrate;
use rupa_migrate::commands::{Command, Exit, Outcome};

/// How pools of one mode open and run migration commands of catalogs of
/// `M`, implemented by the `Connect::Completion` markers: `Immediate` for
/// sync pools, `Awaited` for async ones. Sync work runs on the calling
/// task, which only startup and Setup-phase commands do.
pub trait Mode<P: Connect, M>: Sized {
    /// Opens the pool `config` describes.
    fn pool(config: &DbConfig) -> impl Future<Output = Result<P, ConnectError>> + Send;

    /// Runs `command` on a connection opened outside the pool with `config`.
    fn run<'c>(
        catalog: &'c Catalog<'static, M>,
        command: &Command<'_>,
        config: &DbConfig,
    ) -> impl Future<Output = Outcome<'c>> + Send;

    /// Runs and prints `migrate`, connecting with `config` when the command
    /// needs the target; an `Err` config fails only then.
    fn execute(
        migrate: &Migrate,
        catalog: &Catalog<'static, M>,
        config: Result<&DbConfig, &str>,
    ) -> impl Future<Output = Exit> + Send;
}

/// A connection with `config`, or the reason the configuration has none.
fn configured<'c>(config: Result<&'c DbConfig, &str>) -> Result<&'c DbConfig, ConnectError> {
    config.map_err(|reason| ConnectError::InvalidUrl(reason.to_owned()))
}

#[cfg(feature = "sync")]
mod sync {
    use rupa_core::config::DbConfig;
    use rupa_core::error::ConnectError;
    use rupa_core::exec::Immediate;
    use rupa_core::pool::SyncConnect;
    use rupa_migrate::Catalog;
    use rupa_migrate::commands::cli::{self, Migrate};
    use rupa_migrate::commands::{self, Command, Exit, Outcome};
    use rupa_migrate::sync::{Migration, Session};

    use super::{Mode, configured};

    impl<P, M> Mode<P, M> for Immediate
    where
        P: SyncConnect,
        P::Connection: Session,
        M: Migration<P::Connection> + Sync,
    {
        async fn pool(config: &DbConfig) -> Result<P, ConnectError> {
            P::connect(config)
        }

        async fn run<'c>(
            catalog: &'c Catalog<'static, M>,
            command: &Command<'_>,
            config: &DbConfig,
        ) -> Outcome<'c> {
            commands::sync::run(catalog, command, || P::connect_one(config))
        }

        async fn execute(
            migrate: &Migrate,
            catalog: &Catalog<'static, M>,
            config: Result<&DbConfig, &str>,
        ) -> Exit {
            cli::sync::execute(migrate, catalog, |_| P::connect_one(configured(config)?))
        }
    }
}

#[cfg(feature = "async")]
mod asynchronous {
    use rupa_core::config::DbConfig;
    use rupa_core::error::ConnectError;
    use rupa_core::exec::Awaited;
    use rupa_core::pool::AsyncConnect;
    use rupa_migrate::Catalog;
    use rupa_migrate::asynchronous::{Migration, Session};
    use rupa_migrate::commands::cli::{self, Migrate};
    use rupa_migrate::commands::{self, Command, Exit, Outcome};

    use super::{Mode, configured};

    impl<P, M> Mode<P, M> for Awaited
    where
        P: AsyncConnect,
        P::Connection: Session,
        M: Migration<P::Connection> + Send + Sync,
    {
        async fn pool(config: &DbConfig) -> Result<P, ConnectError> {
            P::connect(config).await
        }

        async fn run<'c>(
            catalog: &'c Catalog<'static, M>,
            command: &Command<'_>,
            config: &DbConfig,
        ) -> Outcome<'c> {
            commands::asynchronous::run(catalog, command, || P::connect_one(config)).await
        }

        async fn execute(
            migrate: &Migrate,
            catalog: &Catalog<'static, M>,
            config: Result<&DbConfig, &str>,
        ) -> Exit {
            let connect =
                |_: Option<&str>| async move { P::connect_one(configured(config)?).await };

            cli::asynchronous::execute(migrate, catalog, connect).await
        }
    }
}
