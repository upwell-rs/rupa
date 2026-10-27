# rupa-example-upwell

## Role

Example Upwell application that runs the `db` crate's migration commands from
its generated command line: `migrate` for the numbered `primary` stream and
`reporting` for the timestamped one.

## Usage

```sh
cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate apply
cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate status --format json
cargo run -p rupa-example-upwell -- reporting plan --url reporting.db
cargo run -p rupa-example-upwell -- migrate --help
```

[`config/application.toml`](config/application.toml) configures both targets
under `migrations.<stream>.url`, relative to the working directory. `--url` or
`DATABASE_URL` overrides it. Without either, a command runs on an in-memory
database.

## Internal role

Implements `rupa::upwell::migrations::sync::Target` once per stream and
registers each target's `Migrations` plugin in `app!`. The commands run at
Upwell's Setup phase. The tests prove the generated command line migrates
without building a component that a normal build constructs, that `--url`
overrides the configuration, and that rupa's exit statuses reach `main`.

## Feature flags

None; it enables `rupa/upwell-migrations` and `rupa/sqlite`.
