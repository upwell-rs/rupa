# rupa-upwell

## Role

The Upwell integration. A `database!` declaration gives an Upwell application:

- a typed, separately configured database;
- a pool component gated on its migrations;
- its repositories as `Arc<dyn Trait>`;
- its migration commands.

## Usage

`rupa = { version = "0.1", features = ["upwell-database", "sqlite"] }`:

```rust,ignore
#[repository]
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
        repositories: [dyn NoteRepository],
    }
}

#[component]
struct Notes {
    repository: Arc<dyn NoteRepository>,   // built after Primary's migrations
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

Nothing is registered per database: like `#[component]`, `database!` is
discovered at link time. SQLite takes `path` (or `":memory:"`) instead of the
server fields. See
[`docs/migrations.md`](../../docs/migrations.md#upwell-applications).

## Internal role

- **Discovery.** `database!` registers itself at link time, like `#[component]`:
  its config binding, component and repository providers go into Upwell's
  `CONFIG_BINDINGS`, `COMPONENTS` and `PROVIDERS`. Its command goes into
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
- **Repositories.** Each listed repository is a provider: `ProviderOf<dyn
  Trait>` builds the repository over a clone of the pool through
  `DynRepository`.
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
