# rupa-driver-tokio-postgres

## Role

Async Postgres executor and pool on tokio-postgres and deadpool.

## Usage

`PgPool::connect(&DbConfig::new("postgres://..")).await?`, inside a tokio runtime. Usually enabled through a feature of the `rupa` facade.

## Internal role

Keeps server-side prepared statements per connection in an LRU (`StatementCache`, on `hashlink`). `shared` is reused by the sync driver, as is `migrations`: the migration session protocol, with its IO steps written once in an internal macro that each driver instantiates for its mode.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `pool` | `PgPool` on deadpool (tokio) |
| `dynamic` | `PgDriver` (`postgres`), the registry entry (implies `pool`) |
| `migration-protocol` | the PostgreSQL migration session protocol shared with the sync driver (no async migration code) |
| `migrations` | `rupa_migrate::asynchronous::Session` on `PgExecutor`, with `TransactionalRequired` and `NonTransactional` bodies (implies `migration-protocol`) |
| `tls-rustls` | TLS on rustls (ring, webpki roots), as the URL's `sslmode` asks |
