# rupa-upwell

## Role

The Upwell integration. It currently adds rupa's migration commands to an Upwell
application's command line. More is planned: DI components for pools,
databases and repositories, and a startup schema gate.

## Usage

`rupa = { version = "0.1", features = ["upwell-migrations", "sqlite"] }`, then
one target per migration stream:

```rust,ignore
use rupa::upwell::migrations::sync::{Migrations, Target};
use rupa::upwell::migrations::exit_code;

struct Primary;

impl Target for Primary {
    type Migration = db::migrations::Migrations;
    type Session = SqliteExecutor;
    type Error = SqliteError;

    const PLUGIN: PluginId = namespaced_id!(PluginId, "app/migrations");
    const COMMAND: &'static str = "migrate";
    const CONFIG: &'static str = "migrations.primary";

    fn catalog() -> Result<Catalog<'static, Self::Migration>, ValidationError> {
        db::migrations::catalog()
    }

    fn connect(url: Option<&str>) -> Result<Self::Session, Self::Error> {
        SqliteExecutor::open(url.unwrap_or("app.db"))
    }
}

app! {
    pub app App {
        name: "app",
        protocol: Rpc,
        plugins: [Migrations<Primary>],
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    exit_code(App::run().await)
}
```

`app migrate apply`, `app migrate status --format json` and the rest of the
operator commands then run before the application builds anything. See
[`docs/migrations.md`](../../docs/migrations.md#upwell-applications).

## Internal role

Consumes the standalone engine through `rupa-migrate`'s command contract
(`commands::cli::{sync, asynchronous}::execute`). It never runs migrations a
different way. Each target's plugin contributes one Setup-phase plugin command
through `upwell-app`'s public plugin API.

Upwell is a git dependency pinned to a commit (conditional components,
upwell-rs/upwell#212) until it is released. crates.io rejects git dependencies,
so this crate cannot be published before then.

## Feature flags

| Feature | Enables |
|---|---|
| `migrations` | `migrations`: the plugin, command and exit status helper, in each enabled mode |
| `sync` | `migrations::sync` targets on sync sessions |
| `async` | `migrations::asynchronous` targets on async sessions |
