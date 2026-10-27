# rupa-example-upwell

## Role

Example Upwell application with two `database!` declarations over the `db`
crate's migration streams.

## Usage

```sh
cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate apply
cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate status --format json
cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config reporting plan
cargo run -p rupa-example-upwell -- migrate --help
```

[`config/application.toml`](config/application.toml) configures each database:

- `primary` field by field, with `startup = "apply"`;
- `reporting` with a templated `url`, `sqlite://${REPORTING_DATABASE:reporting.db}`.

`--url` or `DATABASE_URL` overrides the configured URL for a command.

## Internal role

- `Primary` and `Reporting` are `database!` declarations on rusqlite pools.
  `Primary` provides `Arc<dyn NoteRepository>`, which the `Notes` component
  injects.
- The tests prove:
  - startup applies before `Notes` is built, and `check` refuses a pending
    schema;
  - an async database on sqlx SQLite works the same way;
  - the commands build nothing, `--url` overrides the configuration, and rupa's
    exit statuses reach `main`;
  - each binding declares its configuration schema.

## Feature flags

None; it enables `rupa/upwell-database`, `rupa/sqlite` and `rupa/sqlx-sqlite`.
