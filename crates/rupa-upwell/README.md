# rupa-upwell

## Role

The Upwell integration. A `database!` declaration gives an Upwell application:

- a typed, separately configured database;
- a pool component gated on its migrations;
- every repository declared `#[repository(database = ..)]` over it, as
  `Arc<dyn Trait>`;
- its migration commands.

## Usage

`rupa = { version = "0.1", features = ["upwell-database", "sqlite"] }`:

```rust,ignore
#[repository(database = Primary)]     // provided as Arc<dyn NoteRepository>
pub trait NoteRepository: Send + Sync {
    #[query(order_by = id)]
    fn all(&self) -> Result<Vec<Note>, DynError>;
}

rupa::upwell::database! {
    /// Notes and tags.
    pub Primary {
        pool: SqlitePool,
        config: "database.primary",
        migrations: db::migrations,
        command: "migrate",
    }
}

#[component]
struct Notes {
    repository: Arc<dyn NoteRepository>,   // built after Primary's migrations
    database: Arc<Database<Primary>>,
}

impl Notes {
    /// Every note or none: the injected repository runs on a borrowed
    /// transaction of the injected database, allocating nothing.
    fn import(&self, notes: &[Note]) -> Result<(), DynError> {
        let tx = self.database.begin()?;

        for note in notes {
            self.repository.tx(&tx).add(note)?;
        }

        tx.commit()?;

        Ok(())
    }
}

app! {
    pub app App {
        name: "app",
        protocol: Rpc,
        plugins: [rupa::upwell::Migrations],   // once, for every database's command
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    rupa::upwell::exit_code(App::run().await)
}
```

```toml
[database.primary]
driver   = "postgres"
host     = "db.internal"
port     = 5432
user     = "app"
password = "${DB_PASSWORD}"
name     = "app"
params   = { sslmode = "require" }
# url = "postgres://${database.primary.user}@${database.primary.host}/app"   # replaces the fields

[database.primary.pool]
max_size = 16

[database.primary.migrations]
startup  = "apply"            # off | check (default) | apply
user     = "migrator"         # optional migration credentials
password = "${MIGRATOR_PASSWORD}"
```

Nothing is registered per database or repository: like `#[component]`,
`database!` and `#[repository(database = ..)]` are discovered at link time.
`Database<Primary>` is a `Connection` like its pool, so a component that
injects it runs queries, transactions and repositories on it directly. SQLite takes `path` (or `":memory:"`) instead of the
server fields. See
[`docs/migrations.md`](../../docs/migrations.md#upwell-applications).

## Internal role

- **Discovery.** `database!` registers itself at link time, like `#[component]`:
  its config binding and component go into Upwell's `CONFIG_BINDINGS` and
  `COMPONENTS`, and `#[repository(database = ..)]` puts its provider into
  `PROVIDERS`. Its command goes into
  rupa's `COMMANDS`, which the single `Migrations` plugin reads. Upwell has no
  link-time plugin discovery, so that one plugin is the only registration an
  application writes.
- **Configuration.** `Settings<D>` binds `DatabaseSettings` at the declared
  path and declares its schema, so Upwell's tooling lists every field.
- **Startup gate.** `Database<D>` is a singleton component. Its factory checks
  or applies migrations on a connection opened outside the pool, through
  `rupa_core::pool::Connect` and the migration credentials, before it opens the
  runtime pool. Everything that injects it is therefore built afterwards.
- **Modes.** Sync and async pools are dispatched with blanket impls over their
  `Connect::Completion` marker (`Immediate`, `Awaited`).
- **Connection.** `Database<D>` forwards the executor and transaction traits
  to its pool, which makes it, `&Database<D>` and `Arc<Database<D>>`
  connections.
- **Repositories.** A repository declared over `D` is a provider:
  `ProviderOf<dyn Trait>` builds it over the injected `Arc<Database<D>>`
  itself through `DynRepository`; nothing is cloned.
- **Commands.** The migration commands go through `rupa-migrate`'s command
  contract unchanged, at Upwell's Setup phase.

Upwell is a git dependency pinned to a commit (conditional components,
upwell-rs/upwell#212) until it is released. crates.io rejects git dependencies,
so this crate cannot be published before then. Nothing here depends on Upwell
scopes.

## Feature flags

| Feature | Enables |
|---|---|
| `database` | `database!`, `Database`, `Migrations`, the settings and the commands, for pools of each enabled mode |
| `sync` | sync pools: rusqlite, postgres, mysql |
| `async` | async pools: tokio-postgres, sqlx |
