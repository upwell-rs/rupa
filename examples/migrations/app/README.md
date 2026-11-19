# rupa-example-app

## Role

Example application crate that runs the `db` crate's migration commands from
its own command line.

## Usage

`cargo run -p rupa-example-app -- primary apply` migrates an in-memory
database; add `--url app.db` (or set `DATABASE_URL`) to persist history, so a
second run applies nothing. Every operator command works per stream:

```sh
cargo run -p rupa-example-app -- primary status --url app.db
cargo run -p rupa-example-app -- primary plan --url app.db --format json
cargo run -p rupa-example-app -- primary revert --last 1 --dry-run --url app.db
cargo run -p rupa-example-app -- reporting apply --url reporting.db
cargo run -p rupa-example-app -- reporting create add_sessions --as sync --down \
    --dir examples/migrations/db/reporting
```

## Internal role

Embeds `rupa::migrate::commands::cli::Migrate` once per stream in its clap
command line and runs it with `cli::sync::execute`, opening the dedicated
SQLite connection the command needs. Each stream has its own database. It
depends on the catalogs only, never on how `db` discovered them.

## Feature flags

None.
